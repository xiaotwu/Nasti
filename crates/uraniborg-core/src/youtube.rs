use crate::models::{YoutubePlaylist, YoutubePlaylistItem, YoutubePlaylistSummary, YoutubePrivacy};
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode, Url};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use std::{collections::HashSet, time::Duration};

const API_ROOT: &str = "https://www.googleapis.com/youtube/v3";
const PLAYLIST_PARTS: &str = "snippet,status,contentDetails";
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy)]
pub struct YoutubeAccess<'a> {
    pub token: &'a str,
    pub channel_id: &'a str,
}

#[derive(Debug, thiserror::Error)]
pub enum YoutubeError {
    #[error("请输入 YouTube Music 或 YouTube 的歌单链接或 ID")]
    InvalidPlaylistId,
    #[error("请输入 YouTube Music 或 YouTube 的曲目链接或视频 ID")]
    InvalidVideoId,
    #[error("歌单名称不能为空")]
    InvalidTitle,
    #[error("请先连接拥有该歌单的 YouTube 频道")]
    NotOwner,
    #[error("此 Google 帐号没有可用的 YouTube 频道")]
    NoChannel,
    #[error("授权返回了多个 YouTube 频道，请重新连接需要管理的频道")]
    AmbiguousChannel,
    #[error("找不到该 YouTube 歌单，或当前帐号没有读取权限")]
    PlaylistNotFound,
    #[error("找不到该入列记录，请刷新歌单后重试")]
    EntryNotFound,
    #[error("YouTube 未提供此条目的视频 ID，当前无法移动该条目")]
    EntryUnavailable,
    #[error("曲目位置无效，请刷新歌单后重试")]
    InvalidPosition,
    #[error("请在 YouTube 的歌单设置中把排序方式改为手动，再移动曲目")]
    ManualSortRequired,
    #[error("YouTube Data API 不支持对这个歌单执行该操作")]
    UnsupportedPlaylist,
    #[error("YouTube 请求已达到配额或速率限制，请稍后重试")]
    Quota,
    #[error("YouTube 授权无效，请重新连接帐号")]
    Unauthorized,
    #[error("读取过程中歌单条目已变化，请重新刷新")]
    PlaylistChanged,
    #[error("YouTube 返回的数据格式无效")]
    InvalidResponse,
    #[error("无法连接 YouTube Data API")]
    Network(#[source] reqwest::Error),
    #[error("YouTube 拒绝了请求（HTTP {status}，{reason}）")]
    RequestRejected { status: u16, reason: String },
}

impl From<reqwest::Error> for YoutubeError {
    fn from(error: reqwest::Error) -> Self {
        Self::Network(error.without_url())
    }
}

pub struct YoutubeApi {
    client: Client,
    base_url: String,
}

impl YoutubeApi {
    pub fn new() -> Result<Self, YoutubeError> {
        Ok(Self {
            client: client_builder().https_only(true).build()?,
            base_url: API_ROOT.into(),
        })
    }

    pub async fn channel_id(&self, token: &str) -> Result<String, YoutubeError> {
        let response: Page<ResourceId> =
            read_json(self.request(Method::GET, "channels", token)?.query(&[
                ("part", "id"),
                ("mine", "true"),
                ("maxResults", "50"),
            ]))
            .await?;
        match response.items.as_slice() {
            [] => Err(YoutubeError::NoChannel),
            [channel] if response.next_page_token.is_none() && !channel.id.is_empty() => {
                Ok(channel.id.clone())
            }
            [_] if response.next_page_token.is_none() => Err(YoutubeError::InvalidResponse),
            _ => Err(YoutubeError::AmbiguousChannel),
        }
    }

    pub async fn mine(
        &self,
        access: YoutubeAccess<'_>,
    ) -> Result<Vec<YoutubePlaylistSummary>, YoutubeError> {
        access.owner_channel()?;
        let playlists: Vec<RemotePlaylist> = self
            .collect_pages(
                "playlists",
                &[("part", PLAYLIST_PARTS), ("mine", "true")],
                access,
            )
            .await?;
        Ok(playlists
            .into_iter()
            .map(|playlist| playlist.resource.summary(access))
            .collect())
    }

    pub async fn playlist(
        &self,
        id: &str,
        access: YoutubeAccess<'_>,
    ) -> Result<YoutubePlaylist, YoutubeError> {
        let playlist = self.playlist_resource(id, access).await?;
        let entries: Vec<RemoteEntry> = self
            .collect_pages(
                "playlistItems",
                &[("part", "snippet"), ("playlistId", id)],
                access,
            )
            .await?;
        let mut seen_ids = HashSet::new();
        let mut seen_positions = HashSet::new();
        let mut items = Vec::with_capacity(entries.len());
        for entry in entries {
            if entry.snippet.playlist_id != playlist.resource.id || !valid_entry_id(&entry.id) {
                return Err(YoutubeError::InvalidResponse);
            }
            if !seen_ids.insert(entry.id.clone()) || !seen_positions.insert(entry.snippet.position)
            {
                return Err(YoutubeError::PlaylistChanged);
            }
            items.push(entry.playlist_item());
        }
        items.sort_by_key(|item| item.position);
        Ok(YoutubePlaylist {
            summary: playlist.resource.summary(access),
            items,
            imported_at: None,
        })
    }

    pub async fn create(
        &self,
        title: &str,
        description: &str,
        privacy: YoutubePrivacy,
        access: YoutubeAccess<'_>,
    ) -> Result<YoutubePlaylist, YoutubeError> {
        access.owner_channel()?;
        let body = playlist_body(title, description, privacy)?;
        let created: PlaylistResource = read_json(
            self.request(Method::POST, "playlists", access.token)?
                .query(&[("part", "snippet,status")])
                .json(&body),
        )
        .await?;
        if !valid_playlist_id(&created.id) {
            return Err(YoutubeError::InvalidResponse);
        }
        Ok(YoutubePlaylist {
            summary: created.summary(access),
            items: Vec::new(),
            imported_at: None,
        })
    }

    pub async fn update(
        &self,
        id: &str,
        title: &str,
        description: &str,
        privacy: YoutubePrivacy,
        access: YoutubeAccess<'_>,
    ) -> Result<YoutubePlaylistSummary, YoutubeError> {
        let mut body = playlist_body(title, description, privacy)?;
        let current = self.owned_playlist(id, access).await?.resource;
        body["id"] = json!(current.id);
        // Update replaces mutable properties in each part sent with the request.
        if let Some(language) = current.snippet.default_language {
            body["snippet"]["defaultLanguage"] = json!(language);
        }
        if let Some(podcast_status) = current.status.podcast_status {
            body["status"]["podcastStatus"] = json!(podcast_status);
        }
        let updated: PlaylistResource = read_json(
            self.request(Method::PUT, "playlists", access.token)?
                .query(&[("part", "snippet,status")])
                .json(&body),
        )
        .await?;
        if updated.id != id {
            return Err(YoutubeError::InvalidResponse);
        }
        Ok(updated.summary(access))
    }

    pub async fn add(
        &self,
        playlist_id: &str,
        video_id: &str,
        access: YoutubeAccess<'_>,
    ) -> Result<String, YoutubeError> {
        if !valid_video_id(video_id) {
            return Err(YoutubeError::InvalidVideoId);
        }
        self.owned_playlist(playlist_id, access).await?;
        let added: ResourceId = read_json(
            self.request(Method::POST, "playlistItems", access.token)?
                .query(&[("part", "snippet")])
                .json(&json!({"snippet": {
                    "playlistId": playlist_id,
                    "resourceId": {"kind": "youtube#video", "videoId": video_id}
                }})),
        )
        .await?;
        if !valid_entry_id(&added.id) {
            return Err(YoutubeError::InvalidResponse);
        }
        Ok(added.id)
    }

    pub async fn remove(
        &self,
        playlist_id: &str,
        entry_id: &str,
        access: YoutubeAccess<'_>,
    ) -> Result<(), YoutubeError> {
        self.owned_playlist(playlist_id, access).await?;
        self.playlist_entry(playlist_id, entry_id, access).await?;
        send(
            self.request(Method::DELETE, "playlistItems", access.token)?
                .query(&[("id", entry_id)]),
        )
        .await?;
        Ok(())
    }

    pub async fn move_entry(
        &self,
        playlist_id: &str,
        entry_id: &str,
        position: usize,
        access: YoutubeAccess<'_>,
    ) -> Result<(), YoutubeError> {
        let playlist = self.owned_playlist(playlist_id, access).await?;
        if position >= playlist.content_details.item_count {
            return Err(YoutubeError::InvalidPosition);
        }
        let entry = self.playlist_entry(playlist_id, entry_id, access).await?;
        if position == entry.snippet.position {
            return Ok(());
        }
        let video_id = entry
            .snippet
            .video_id()
            .ok_or(YoutubeError::EntryUnavailable)?;
        send(
            self.request(Method::PUT, "playlistItems", access.token)?
                .query(&[("part", "snippet")])
                .json(&json!({"id": entry.id, "snippet": {
                    "playlistId": playlist_id,
                    "resourceId": {"kind": "youtube#video", "videoId": video_id},
                    "position": position
                }})),
        )
        .await?;
        Ok(())
    }

    pub async fn delete(&self, id: &str, access: YoutubeAccess<'_>) -> Result<(), YoutubeError> {
        self.owned_playlist(id, access).await?;
        send(
            self.request(Method::DELETE, "playlists", access.token)?
                .query(&[("id", id)]),
        )
        .await?;
        Ok(())
    }

    async fn playlist_resource(
        &self,
        id: &str,
        access: YoutubeAccess<'_>,
    ) -> Result<RemotePlaylist, YoutubeError> {
        if !valid_playlist_id(id) {
            return Err(YoutubeError::InvalidPlaylistId);
        }
        let page: Page<RemotePlaylist> = read_json(
            self.request(Method::GET, "playlists", access.token)?
                .query(&[("part", PLAYLIST_PARTS), ("id", id)]),
        )
        .await?;
        page.items
            .into_iter()
            .find(|playlist| playlist.resource.id == id)
            .ok_or(YoutubeError::PlaylistNotFound)
    }

    async fn owned_playlist(
        &self,
        id: &str,
        access: YoutubeAccess<'_>,
    ) -> Result<RemotePlaylist, YoutubeError> {
        let channel = access.owner_channel()?;
        let playlist = self.playlist_resource(id, access).await?;
        if playlist.resource.snippet.channel_id != channel {
            return Err(YoutubeError::NotOwner);
        }
        Ok(playlist)
    }

    async fn playlist_entry(
        &self,
        playlist_id: &str,
        entry_id: &str,
        access: YoutubeAccess<'_>,
    ) -> Result<RemoteEntry, YoutubeError> {
        if !valid_entry_id(entry_id) {
            return Err(YoutubeError::EntryNotFound);
        }
        let page: Page<RemoteEntry> = read_json(
            self.request(Method::GET, "playlistItems", access.token)?
                .query(&[("part", "snippet"), ("id", entry_id)]),
        )
        .await?;
        page.items
            .into_iter()
            .find(|entry| entry.id == entry_id && entry.snippet.playlist_id == playlist_id)
            .ok_or(YoutubeError::EntryNotFound)
    }

    async fn collect_pages<T: DeserializeOwned>(
        &self,
        resource: &str,
        query: &[(&str, &str)],
        access: YoutubeAccess<'_>,
    ) -> Result<Vec<T>, YoutubeError> {
        let mut items = Vec::new();
        let mut next_page: Option<String> = None;
        let mut seen = HashSet::new();
        loop {
            let mut request = self
                .request(Method::GET, resource, access.token)?
                .query(query)
                .query(&[("maxResults", "50")]);
            if let Some(token) = next_page.as_deref() {
                request = request.query(&[("pageToken", token)]);
            }
            let page: Page<T> = read_json(request).await?;
            items.extend(page.items);
            let Some(token) = page.next_page_token else {
                return Ok(items);
            };
            if token.is_empty() || !seen.insert(token.clone()) {
                return Err(YoutubeError::InvalidResponse);
            }
            next_page = Some(token);
        }
    }

    fn request(
        &self,
        method: Method,
        resource: &str,
        token: &str,
    ) -> Result<RequestBuilder, YoutubeError> {
        if token.trim().is_empty() {
            return Err(YoutubeError::Unauthorized);
        }
        Ok(self
            .client
            .request(method, format!("{}/{resource}", self.base_url))
            .bearer_auth(token))
    }
}

impl<'a> YoutubeAccess<'a> {
    fn owner_channel(self) -> Result<&'a str, YoutubeError> {
        if self.channel_id.is_empty() {
            return Err(YoutubeError::NoChannel);
        }
        Ok(self.channel_id)
    }
}

pub fn playlist_id(reference: &str) -> Result<String, YoutubeError> {
    let reference = reference.trim();
    if valid_playlist_id(reference) {
        return Ok(reference.into());
    }
    let url = youtube_url(reference).ok_or(YoutubeError::InvalidPlaylistId)?;
    let id = query_value(&url, "list").filter(|id| valid_playlist_id(id));
    id.ok_or(YoutubeError::InvalidPlaylistId)
}

pub fn video_id(reference: &str) -> Result<String, YoutubeError> {
    let reference = reference.trim();
    if valid_video_id(reference) {
        return Ok(reference.into());
    }
    let url = youtube_url(reference).ok_or(YoutubeError::InvalidVideoId)?;
    let id = match (url.host_str(), url.path()) {
        (Some("youtu.be"), path) => path.strip_prefix('/').map(str::to_owned),
        (_, "/watch") => query_value(&url, "v"),
        (_, path) => ["/shorts/", "/live/", "/embed/"]
            .iter()
            .find_map(|prefix| path.strip_prefix(prefix))
            .map(str::to_owned),
    };
    id.filter(|id| valid_video_id(id))
        .ok_or(YoutubeError::InvalidVideoId)
}

fn youtube_url(reference: &str) -> Option<Url> {
    let url = Url::parse(reference).ok()?;
    (matches!(url.scheme(), "http" | "https")
        && matches!(
            url.host_str(),
            Some(
                "music.youtube.com"
                    | "youtube.com"
                    | "www.youtube.com"
                    | "m.youtube.com"
                    | "youtu.be"
            )
        )
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none())
    .then_some(url)
}

fn query_value(url: &Url, name: &str) -> Option<String> {
    let mut values = url.query_pairs().filter(|(key, _)| key == name);
    let value = values.next()?.1.into_owned();
    values.next().is_none().then_some(value)
}

fn valid_playlist_id(id: &str) -> bool {
    (2..=128).contains(&id.len()) && youtube_token(id)
}

fn valid_video_id(id: &str) -> bool {
    id.len() == 11 && youtube_token(id)
}

fn youtube_token(id: &str) -> bool {
    id.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn valid_entry_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control)
}

fn client_builder() -> reqwest::ClientBuilder {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(25))
        .user_agent(concat!("NASTI/", env!("CARGO_PKG_VERSION")))
}

fn playlist_body(
    title: &str,
    description: &str,
    privacy: YoutubePrivacy,
) -> Result<Value, YoutubeError> {
    if title.trim().is_empty() {
        return Err(YoutubeError::InvalidTitle);
    }
    Ok(
        json!({"snippet": {"title": title, "description": description}, "status": {"privacyStatus": privacy}}),
    )
}

async fn read_json<T: DeserializeOwned>(request: RequestBuilder) -> Result<T, YoutubeError> {
    let bytes = read_body(send(request).await?).await?;
    serde_json::from_slice(&bytes).map_err(|_| YoutubeError::InvalidResponse)
}

async fn send(request: RequestBuilder) -> Result<Response, YoutubeError> {
    let response = request.send().await?;
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let reason = read_body(response)
        .await
        .ok()
        .and_then(|body| serde_json::from_slice::<Value>(&body).ok())
        .and_then(|body| {
            body.pointer("/error/errors/0/reason")
                .and_then(Value::as_str)
                .filter(|reason| {
                    reason.len() < 64 && reason.bytes().all(|b| b.is_ascii_alphanumeric())
                })
                .map(str::to_owned)
        });
    Err(match reason.as_deref() {
        Some("manualSortRequired") => YoutubeError::ManualSortRequired,
        Some("invalidPlaylistItemPosition") => YoutubeError::InvalidPosition,
        Some(
            "playlistOperationUnsupported"
            | "watchLaterNotAccessible"
            | "watchHistoryNotAccessible",
        ) => YoutubeError::UnsupportedPlaylist,
        Some(
            "quotaExceeded" | "dailyLimitExceeded" | "rateLimitExceeded" | "userRateLimitExceeded",
        ) => YoutubeError::Quota,
        Some("playlistNotFound" | "playlistNotAccessible" | "playlistItemsNotAccessible") => {
            YoutubeError::PlaylistNotFound
        }
        Some("playlistItemNotFound") => YoutubeError::EntryNotFound,
        Some("youtubeSignupRequired" | "channelNotFound") => YoutubeError::NoChannel,
        _ if status == StatusCode::UNAUTHORIZED => YoutubeError::Unauthorized,
        _ if status == StatusCode::TOO_MANY_REQUESTS => YoutubeError::Quota,
        _ => YoutubeError::RequestRejected {
            status: status.as_u16(),
            reason: reason.unwrap_or_else(|| "requestFailed".to_owned()),
        },
    })
}

async fn read_body(mut response: Response) -> Result<Vec<u8>, YoutubeError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if chunk.len() > MAX_RESPONSE_BYTES - bytes.len() {
            return Err(YoutubeError::InvalidResponse);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page<T> {
    items: Vec<T>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct ResourceId {
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemotePlaylist {
    #[serde(flatten)]
    resource: PlaylistResource,
    content_details: PlaylistContentDetails,
}

#[derive(Deserialize)]
struct PlaylistResource {
    id: String,
    snippet: PlaylistSnippet,
    status: PlaylistStatus,
}

impl PlaylistResource {
    fn summary(self, access: YoutubeAccess<'_>) -> YoutubePlaylistSummary {
        let can_edit = access
            .owner_channel()
            .is_ok_and(|channel| self.snippet.channel_id == channel);
        YoutubePlaylistSummary {
            id: self.id,
            title: self.snippet.title,
            description: self.snippet.description,
            privacy: self.status.privacy_status,
            can_edit,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlaylistSnippet {
    channel_id: String,
    title: String,
    #[serde(default)]
    description: String,
    default_language: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlaylistStatus {
    privacy_status: YoutubePrivacy,
    podcast_status: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlaylistContentDetails {
    item_count: usize,
}

#[derive(Deserialize)]
struct RemoteEntry {
    id: String,
    snippet: EntrySnippet,
}

impl RemoteEntry {
    fn playlist_item(self) -> YoutubePlaylistItem {
        YoutubePlaylistItem {
            entry_id: self.id,
            video_id: self.snippet.video_id().unwrap_or("").to_owned(),
            title: self.snippet.title,
            artist: self.snippet.video_owner_channel_title,
            position: self.snippet.position,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EntrySnippet {
    playlist_id: String,
    position: usize,
    title: String,
    #[serde(default)]
    video_owner_channel_title: String,
    resource_id: Option<EntryResourceId>,
}

impl EntrySnippet {
    fn video_id(&self) -> Option<&str> {
        let resource = self.resource_id.as_ref()?;
        resource
            .video_id
            .as_deref()
            .filter(|id| resource.kind == "youtube#video" && valid_video_id(id))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EntryResourceId {
    kind: String,
    video_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Bytes,
        extract::State,
        http::{HeaderMap, Uri},
        routing::any,
        Json, Router,
    };
    use std::{
        collections::{BTreeMap, VecDeque},
        sync::Arc,
    };
    use tokio::{net::TcpListener, sync::Mutex};

    const OAUTH: YoutubeAccess = YoutubeAccess {
        token: "test-token",
        channel_id: "test-channel",
    };
    const PLAYLIST: &str = "PL_test";
    const VIDEO: &str = "QL-ABC_defg";

    struct Exchange {
        method: Method,
        resource: &'static str,
        query: &'static [(&'static str, &'static str)],
        body: Option<Value>,
        status: StatusCode,
        response: Value,
    }

    impl Exchange {
        fn get(
            resource: &'static str,
            query: &'static [(&'static str, &'static str)],
            response: Value,
        ) -> Self {
            Self {
                method: Method::GET,
                resource,
                query,
                body: None,
                status: StatusCode::OK,
                response,
            }
        }

        fn write(mut self, method: Method, body: Value) -> Self {
            self.method = method;
            self.body = Some(body);
            self
        }

        fn delete(mut self) -> Self {
            self.method = Method::DELETE;
            self.status = StatusCode::NO_CONTENT;
            self
        }
    }

    #[derive(Clone)]
    struct Script {
        exchanges: Arc<Mutex<VecDeque<Exchange>>>,
    }

    async fn exchange(
        State(script): State<Script>,
        method: Method,
        uri: Uri,
        headers: HeaderMap,
        body: Bytes,
    ) -> (StatusCode, Json<Value>) {
        let expected = script
            .exchanges
            .lock()
            .await
            .pop_front()
            .expect("unexpected request");
        assert_eq!(method, expected.method);
        assert_eq!(uri.path(), format!("/youtube/v3/{}", expected.resource));
        let url = Url::parse(&format!("http://localhost{uri}")).unwrap();
        let query: BTreeMap<_, _> = url.query_pairs().collect();
        assert_eq!(query.len(), expected.query.len());
        for &(name, value) in expected.query {
            assert_eq!(
                query.get(name).map(|value| value.as_ref()),
                Some(value),
                "query key {name}"
            );
        }
        assert_eq!(
            headers.get("authorization").unwrap().to_str().unwrap(),
            format!("Bearer {}", OAUTH.token)
        );
        assert!(!headers.contains_key("x-goog-api-key"));
        if let Some(expected_body) = expected.body {
            assert_eq!(
                serde_json::from_slice::<Value>(&body).unwrap(),
                expected_body
            );
        } else {
            assert!(body.is_empty());
        }
        (expected.status, Json(expected.response))
    }

    struct ApiFixture {
        api: YoutubeApi,
        script: Script,
        server: tokio::task::JoinHandle<()>,
    }

    impl ApiFixture {
        async fn start(requests: Vec<Exchange>) -> Self {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let base_url = format!("http://{}/youtube/v3", listener.local_addr().unwrap());
            let script = Script {
                exchanges: Arc::new(Mutex::new(requests.into())),
            };
            let app = Router::new()
                .fallback(any(exchange))
                .with_state(script.clone());
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let api = YoutubeApi {
                client: client_builder().no_proxy().build().unwrap(),
                base_url,
            };
            Self {
                api,
                script,
                server,
            }
        }

        async fn assert_consumed(&self) {
            assert!(
                self.script.exchanges.lock().await.is_empty(),
                "expected more API requests"
            );
        }
    }

    impl Drop for ApiFixture {
        fn drop(&mut self) {
            self.server.abort();
        }
    }

    fn playlist_resource(owner: &str) -> Value {
        json!({"id": PLAYLIST,
            "snippet": {"channelId": owner, "title": "Example", "description": "Original", "defaultLanguage": "zh-CN"},
            "status": {"privacyStatus": "unlisted", "podcastStatus": "enabled"},
            "contentDetails": {"itemCount": 3}
        })
    }

    fn owned_playlist_request() -> Exchange {
        Exchange::get(
            "playlists",
            &[("part", PLAYLIST_PARTS), ("id", PLAYLIST)],
            json!({"items": [playlist_resource("test-channel")]}),
        )
    }

    fn entry(id: &str, playlist: &str, position: usize) -> Value {
        json!({"id": id, "snippet": {
            "playlistId": playlist, "position": position, "title": "Example recording",
            "videoOwnerChannelTitle": "Artist",
            "resourceId": {"kind": "youtube#video", "videoId": VIDEO}
        }})
    }

    #[test]
    fn references_use_supported_hosts_and_single_identifiers() {
        assert_eq!(
            playlist_id(" https://music.youtube.com/playlist?list=PL_test&si=tag ").unwrap(),
            PLAYLIST
        );
        assert_eq!(playlist_id(PLAYLIST).unwrap(), PLAYLIST);
        assert_eq!(video_id("https://youtu.be/QL-ABC_defg").unwrap(), VIDEO);
        assert_eq!(
            video_id("https://music.youtube.com/watch?v=QL-ABC_defg&list=PL_test").unwrap(),
            VIDEO
        );
        assert!(
            playlist_id("https://music.youtube.com.example.com/playlist?list=PL_test").is_err()
        );
        assert!(playlist_id("https://music.youtube.com/playlist?list=PL_test&list=other").is_err());
        assert!(video_id("https://other.example/watch?v=QL-ABC_defg").is_err());
        assert!(video_id("https://youtube.com/watch?v=QL-ABC_defg&v=ABCDEFGHIJK").is_err());
    }

    #[tokio::test]
    async fn import_retains_duplicate_video_occurrences_and_paginates() {
        let mut private_playlist = playlist_resource(OAUTH.channel_id);
        private_playlist["status"]["privacyStatus"] = json!("private");
        let fixture = ApiFixture::start(vec![
            Exchange::get("playlists", &[("part", PLAYLIST_PARTS), ("id", PLAYLIST)], json!({"items": [private_playlist]})),
            Exchange::get("playlistItems", &[("part", "snippet"), ("playlistId", PLAYLIST), ("maxResults", "50")], json!({
                "nextPageToken": "next+page",
                "items": [entry("entry-second", PLAYLIST, 1)]
            })),
            Exchange::get("playlistItems", &[("part", "snippet"), ("playlistId", PLAYLIST), ("maxResults", "50"), ("pageToken", "next+page")], json!({
                "items": [entry("entry-first", PLAYLIST, 0), {"id": "entry-unavailable", "snippet": {"playlistId": PLAYLIST, "position": 2, "title": "Private recording"}}]
            })),
        ]).await;
        let without_token = YoutubeAccess { token: "", ..OAUTH };
        assert!(matches!(
            fixture.api.playlist(PLAYLIST, without_token).await,
            Err(YoutubeError::Unauthorized)
        ));
        let playlist = fixture.api.playlist(PLAYLIST, OAUTH).await.unwrap();
        assert_eq!(playlist.summary.privacy, YoutubePrivacy::Private);
        assert!(playlist.summary.can_edit);
        assert_eq!(playlist.items[0].entry_id, "entry-first");
        assert_eq!(playlist.items[1].entry_id, "entry-second");
        assert_eq!(playlist.items[0].video_id, VIDEO);
        assert_eq!(playlist.items[1].video_id, VIDEO);
        assert_eq!(playlist.items[1].artist, "Artist");
        assert_eq!(playlist.items[2].entry_id, "entry-unavailable");
        assert!(playlist.items[2].video_id.is_empty());
        fixture.assert_consumed().await;
    }

    #[tokio::test]
    async fn oauth_playlist_operations_use_the_verified_remote_occurrence_id() {
        let get_entry = || {
            Exchange::get(
                "playlistItems",
                &[("part", "snippet"), ("id", "entry-second")],
                json!({"items": [entry("entry-second", PLAYLIST, 1)]}),
            )
        };
        let moved = || {
            Exchange::get("playlistItems", &[("part", "snippet")], json!({"id": "entry-second"}))
            .write(Method::PUT, json!({"id": "entry-second", "snippet": {
                "playlistId": PLAYLIST, "position": 0, "resourceId": {"kind": "youtube#video", "videoId": VIDEO}
            }}))
        };
        let rejected = Exchange {
            status: StatusCode::BAD_REQUEST,
            response: json!({"error": {"errors": [{"reason": "manualSortRequired"}]}}),
            ..moved()
        };
        let fixture = ApiFixture::start(vec![
            Exchange::get("channels", &[("part", "id"), ("mine", "true"), ("maxResults", "50")], json!({"items": [{"id": "test-channel"}]})),
            Exchange::get("playlists", &[("part", PLAYLIST_PARTS), ("mine", "true"), ("maxResults", "50")], json!({"items": [playlist_resource("test-channel")]})),
            Exchange::get("playlists", &[("part", "snippet,status")], json!({
                "id": PLAYLIST,
                "snippet": {"channelId": OAUTH.channel_id, "title": "Created title", "description": "Created description"},
                "status": {"privacyStatus": "private"}
            }))
                .write(Method::POST, json!({"snippet": {"title": "New playlist", "description": "Original"}, "status": {"privacyStatus": "private"}})),
            owned_playlist_request(),
            Exchange::get("playlists", &[("part", "snippet,status")], json!({
                "id": PLAYLIST,
                "snippet": {"channelId": OAUTH.channel_id, "title": "Updated title", "description": "Updated description"},
                "status": {"privacyStatus": "public"}
            }))
                .write(Method::PUT, json!({"id": PLAYLIST, "snippet": {"title": "Updated", "description": "", "defaultLanguage": "zh-CN"}, "status": {"privacyStatus": "public", "podcastStatus": "enabled"}})),
            owned_playlist_request(),
            Exchange::get("playlistItems", &[("part", "snippet")], entry("entry-added", PLAYLIST, 3))
                .write(Method::POST, json!({"snippet": {"playlistId": PLAYLIST, "resourceId": {"kind": "youtube#video", "videoId": VIDEO}}})),
            owned_playlist_request(),
            get_entry(),
            rejected,
            owned_playlist_request(),
            get_entry(),
            moved(),
            owned_playlist_request(),
            get_entry(),
            Exchange::get("playlistItems", &[("id", "entry-second")], Value::Null).delete(),
            owned_playlist_request(),
            Exchange::get("playlists", &[("id", PLAYLIST)], Value::Null).delete(),
        ]).await;
        let api = &fixture.api;
        assert_eq!(api.channel_id("test-token").await.unwrap(), "test-channel");
        assert!(api.mine(OAUTH).await.unwrap()[0].can_edit);
        let created = api
            .create("New playlist", "Original", YoutubePrivacy::Private, OAUTH)
            .await
            .unwrap();
        assert_eq!(created.summary.id, PLAYLIST);
        assert_eq!(created.summary.title, "Created title");
        assert_eq!(created.summary.description, "Created description");
        assert_eq!(created.summary.privacy, YoutubePrivacy::Private);
        assert!(created.summary.can_edit);
        assert!(created.items.is_empty());
        assert!(created.imported_at.is_none());
        let updated = api
            .update(PLAYLIST, "Updated", "", YoutubePrivacy::Public, OAUTH)
            .await
            .unwrap();
        assert_eq!(updated.id, PLAYLIST);
        assert_eq!(updated.title, "Updated title");
        assert_eq!(updated.description, "Updated description");
        assert_eq!(updated.privacy, YoutubePrivacy::Public);
        assert!(updated.can_edit);
        assert_eq!(
            api.add(PLAYLIST, VIDEO, OAUTH).await.unwrap(),
            "entry-added"
        );
        assert!(matches!(
            api.move_entry(PLAYLIST, "entry-second", 0, OAUTH).await,
            Err(YoutubeError::ManualSortRequired)
        ));
        api.move_entry(PLAYLIST, "entry-second", 0, OAUTH)
            .await
            .unwrap();
        api.remove(PLAYLIST, "entry-second", OAUTH).await.unwrap();
        api.delete(PLAYLIST, OAUTH).await.unwrap();
        fixture.assert_consumed().await;
    }

    #[tokio::test]
    async fn rejected_ownership_or_membership_prevents_mutation() {
        let fixture = ApiFixture::start(vec![
            Exchange::get(
                "playlists",
                &[("part", PLAYLIST_PARTS), ("id", PLAYLIST)],
                json!({"items": [playlist_resource("other-channel")]}),
            ),
            owned_playlist_request(),
            Exchange::get(
                "playlistItems",
                &[("part", "snippet"), ("id", "entry-other")],
                json!({"items": [entry("entry-other", "PL_other", 0)]}),
            ),
        ])
        .await;
        assert!(matches!(
            fixture.api.delete(PLAYLIST, OAUTH).await,
            Err(YoutubeError::NotOwner)
        ));
        assert!(matches!(
            fixture.api.remove(PLAYLIST, "entry-other", OAUTH).await,
            Err(YoutubeError::EntryNotFound)
        ));
        fixture.assert_consumed().await;
    }
}
