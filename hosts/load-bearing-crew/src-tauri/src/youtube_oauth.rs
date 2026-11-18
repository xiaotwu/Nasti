use crate::runtime_storage::{same_volume, OAUTH_PENDING};
use crate::ui_locale::Language;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use reqwest::{Client, StatusCode, Url};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{ErrorKind, Read, Write};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use uuid::Uuid;

const CONSENT_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const OAUTH_API_ORIGIN: &str = "https://oauth2.googleapis.com/";
const PLAYLIST_SCOPE: &str = "https://www.googleapis.com/auth/youtube.force-ssl";
const CALLBACK_PATH: &str = "/";
const CALLBACK_LIMIT: usize = 8192;
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(180);
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);
const CLIENT_FILE_LIMIT: u64 = 64 * 1024;
const REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);
const CANCELLED: &str = "已取消 Google 登录";

pub(crate) struct YoutubeOAuth {
    client: Client,
    api_origin: Url,
    config: Option<DesktopClient>,
    client_file: PathBuf,
    staging_dir: PathBuf,
    refresh_token: Option<String>,
    pending_revocation: Option<String>,
    access: Option<AccessToken>,
    pub(crate) verified: Option<SessionTime>,
}

#[derive(Deserialize)]
struct DesktopClient {
    #[serde(rename = "client_id")]
    id: String,
    #[serde(rename = "client_secret")]
    secret: Option<String>,
}

#[derive(Deserialize)]
struct DesktopClientDownload {
    installed: DesktopClient,
}

struct AccessToken {
    value: String,
    received: SessionTime,
    lifetime: Duration,
}

#[derive(Clone, Copy)]
pub(crate) struct SessionTime {
    monotonic: Instant,
    wall: SystemTime,
}

impl SessionTime {
    pub(crate) fn now() -> Self {
        Self {
            monotonic: Instant::now(),
            wall: SystemTime::now(),
        }
    }

    #[cfg(test)]
    pub(crate) fn after_suspension(duration: Duration) -> Self {
        Self {
            monotonic: Instant::now(),
            wall: SystemTime::now() - duration,
        }
    }

    pub(crate) fn elapsed(self) -> Duration {
        self.monotonic
            .elapsed()
            .max(self.wall.elapsed().unwrap_or_default())
    }
}

#[derive(Deserialize)]
struct TokenReply {
    access_token: String,
    token_type: String,
    refresh_token: Option<String>,
    expires_in: u64,
    scope: Option<String>,
}

#[derive(Deserialize)]
struct OAuthError {
    error: String,
    #[serde(rename = "error_description")]
    description: Option<String>,
}

impl YoutubeOAuth {
    pub(crate) fn new(client_file: &Path, staging_dir: &Path) -> Result<Self, String> {
        let config = DesktopClient::load(client_file)?;
        let client = Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|_| "无法初始化 Google 授权连接".to_owned())?;
        Ok(Self {
            client,
            api_origin: Url::parse(OAUTH_API_ORIGIN).expect("constant Google OAuth origin"),
            config,
            client_file: client_file.to_owned(),
            staging_dir: staging_dir.to_owned(),
            refresh_token: None,
            pending_revocation: None,
            access: None,
            verified: None,
        })
    }

    pub(crate) fn configured(&self) -> bool {
        self.config.is_some()
    }

    pub(crate) fn configure_client(&mut self, json: &str) -> Result<(), String> {
        if self.configured() || self.has_session() {
            return Err("Google 桌面 OAuth 客户端已配置".into());
        }
        let client = DesktopClient::parse_download(json.as_bytes())?;
        let directory = self
            .client_file
            .parent()
            .ok_or("无法保存 Google 桌面 OAuth 客户端 JSON")?;
        if !same_volume(directory, &self.staging_dir).unwrap_or(false) {
            return Err("无法保存 Google 桌面 OAuth 客户端 JSON".into());
        }
        let settings = serde_json::to_vec(&serde_json::json!({
            "installed": {"client_id": &client.id, "client_secret": &client.secret}
        }))
        .map_err(|_| "无法保存 Google 桌面 OAuth 客户端 JSON".to_owned())?;
        if settings.len() as u64 > CLIENT_FILE_LIMIT {
            return Err("Google 桌面 OAuth 客户端 JSON 文件过大".into());
        }
        // Publish the complete private file only if the destination still does not exist.
        let mut pending = tempfile::Builder::new()
            .prefix(OAUTH_PENDING)
            .rand_bytes(0)
            .tempfile_in(&self.staging_dir)
            .map_err(|_| "无法保存 Google 桌面 OAuth 客户端 JSON".to_owned())?;
        pending
            .write_all(&settings)
            .and_then(|_| pending.as_file().sync_all())
            .map_err(|_| "无法保存 Google 桌面 OAuth 客户端 JSON".to_owned())?;
        pending
            .persist_noclobber(&self.client_file)
            .map_err(|error| {
                if error.error.kind() == ErrorKind::AlreadyExists {
                    "Google 桌面 OAuth 客户端 JSON 文件已存在"
                } else {
                    "无法保存 Google 桌面 OAuth 客户端 JSON"
                }
                .to_owned()
            })?;
        self.config = Some(client);
        Ok(())
    }

    pub(crate) fn has_session(&self) -> bool {
        self.refresh_token.is_some()
    }

    pub(crate) fn invalidate_access_token(&mut self) {
        self.access = None;
    }

    pub(crate) async fn connect(
        &mut self,
        cancellation: &mut watch::Receiver<u64>,
    ) -> Result<(), String> {
        self.connect_with(open_external, cancellation).await
    }

    async fn connect_with(
        &mut self,
        open: impl FnOnce(&Url) -> Result<(), String>,
        cancellation: &mut watch::Receiver<u64>,
    ) -> Result<(), String> {
        if cancellation.has_changed().unwrap_or(true) {
            return Err(CANCELLED.into());
        }
        if self.pending_revocation.is_some() {
            return Err(
                "正在重试撤销上一次 Google 授权。也可以在 Google 账号设置中移除访问权限".into(),
            );
        }
        let config = self.config.as_ref().ok_or("此版本尚未启用 Google 登录")?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| "无法打开本机授权回调".to_owned())?;
        let address = listener
            .local_addr()
            .map_err(|_| "无法读取本机授权回调地址".to_owned())?;
        let redirect = Url::parse(&format!("http://{address}{CALLBACK_PATH}")).unwrap();
        let verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let state = Uuid::new_v4().simple().to_string();
        let browser_url = consent_url(&config.id, &redirect, &state, &verifier);
        open(&browser_url)?;
        // Cancellation drops the loopback listener. Once Google returns a code, finish the
        // exchange so a late cancellation cannot hide a newly granted session.
        let code = tokio::select! {
            biased;
            _ = cancellation.changed() => return Err(CANCELLED.into()),
            code = receive_code(listener, &redirect, &state, Language::current()) => code?,
        };
        let reply = self
            .request_tokens(
                config,
                &[
                    ("grant_type", "authorization_code"),
                    ("code", &code),
                    ("code_verifier", &verifier),
                    ("redirect_uri", redirect.as_str()),
                ],
            )
            .await?
            .ok_or("Google 授权码已过期，请重新连接")?;
        let access = AccessToken::from_reply(&reply)?;
        let refresh = reply
            .refresh_token
            .filter(|value| !value.is_empty())
            .ok_or("Google 未返回用于会话校验的授权，请重新同意歌单权限")?;
        self.refresh_token = Some(refresh);
        self.verified = Some(access.received);
        self.access = Some(access);
        Ok(())
    }

    pub(crate) async fn access_token(&mut self) -> Result<Option<String>, String> {
        if let Some(access) = self
            .access
            .as_ref()
            .filter(|token| token.received.elapsed() < token.lifetime)
        {
            return Ok(Some(access.value.clone()));
        }
        let (Some(config), Some(refresh)) = (self.config.as_ref(), self.refresh_token.as_ref())
        else {
            return Ok(None);
        };
        let reply = self
            .request_tokens(
                config,
                &[("grant_type", "refresh_token"), ("refresh_token", refresh)],
            )
            .await?;
        let Some(reply) = reply else {
            self.access = None;
            self.refresh_token = None;
            self.verified = None;
            return Ok(None);
        };
        let access = AccessToken::from_reply(&reply)?;
        let value = access.value.clone();
        self.verified = Some(access.received);
        self.access = Some(access);
        if let Some(rotated) = reply.refresh_token.filter(|value| !value.is_empty()) {
            self.refresh_token = Some(rotated);
        }
        Ok(Some(value))
    }

    pub(crate) async fn disconnect(&mut self) -> Result<(), String> {
        self.access = None;
        self.verified = None;
        if let Some(refresh) = self.refresh_token.take() {
            self.pending_revocation = Some(refresh);
        }
        self.retry_revoke().await
    }

    pub(crate) async fn retry_revoke(&mut self) -> Result<(), String> {
        let Some(refresh) = self.pending_revocation.as_deref() else {
            return Ok(());
        };
        let response = self
            .client
            .post(
                self.api_origin
                    .join("revoke")
                    .expect("fixed revocation path"),
            )
            .form(&[("token", refresh)])
            .send()
            .await
            .map_err(|_| "本地数据已清除。暂时无法联系 Google，Nasti 运行期间会重试。也可以在 Google 账号设置中移除访问权限".to_owned())?;
        let status = response.status();
        let already_revoked = status == StatusCode::BAD_REQUEST
            && response.json::<OAuthError>().await.is_ok_and(|reply| {
                matches!(reply.error.as_str(), "invalid_token" | "invalid_grant")
            });
        if !status.is_success() && !already_revoked {
            return Err("本地数据已清除。Google 尚未确认撤销，Nasti 运行期间会重试。也可以在 Google 账号设置中移除访问权限".into());
        }
        self.pending_revocation = None;
        Ok(())
    }

    async fn request_tokens(
        &self,
        config: &DesktopClient,
        grant: &[(&str, &str)],
    ) -> Result<Option<TokenReply>, String> {
        let mut form = grant.to_vec();
        form.push(("client_id", config.id.as_str()));
        if let Some(secret) = config.secret.as_deref() {
            form.push(("client_secret", secret));
        }
        let response = self
            .client
            .post(self.api_origin.join("token").expect("fixed token path"))
            .form(&form)
            .send()
            .await
            .map_err(|_| "暂时无法连接 Google 授权服务".to_owned())?;
        let status = response.status();
        if status.is_success() {
            return response
                .json::<TokenReply>()
                .await
                .map(Some)
                .map_err(|_| "Google 返回的授权信息无法读取".to_owned());
        }
        let error = response.json::<OAuthError>().await.ok();
        match error.as_ref().map(|failure| failure.error.as_str()) {
            Some("invalid_request")
                if error
                    .as_ref()
                    .and_then(|failure| failure.description.as_deref())
                    .is_some_and(|description| description.contains("client_secret")) =>
            {
                Err("此版本缺少 Google 应用客户端凭据，请联系发布者".into())
            }
            Some("invalid_grant") if status == StatusCode::BAD_REQUEST => Ok(None),
            Some("invalid_client" | "unauthorized_client") => {
                Err("请检查 Google 桌面 OAuth 客户端配置".into())
            }
            _ => Err(format!("Google 授权请求失败（HTTP {}）", status.as_u16())),
        }
    }
}

impl DesktopClient {
    fn load(default_path: &Path) -> Result<Option<Self>, String> {
        if let Some(id) = env_value("NASTI_YOUTUBE_CLIENT_ID") {
            return Ok(Some(Self {
                id,
                secret: env_value("NASTI_YOUTUBE_CLIENT_SECRET"),
            }));
        }
        match std::env::var_os("NASTI_YOUTUBE_CLIENT_FILE").filter(|path| !path.is_empty()) {
            Some(path) => Self::read_file(Path::new(&path), true),
            None => match Self::read_file(default_path, false)? {
                // Keep the OAuth client an upgraded user already configured.
                Some(client) => Ok(Some(client)),
                None => Self::app_identity(
                    option_env!("NASTI_APP_YOUTUBE_CLIENT_ID"),
                    option_env!("NASTI_APP_YOUTUBE_CLIENT_SECRET"),
                ),
            },
        }
    }

    fn app_identity(id: Option<&str>, secret: Option<&str>) -> Result<Option<Self>, String> {
        let Some(id) = id.map(str::trim).filter(|id| !id.is_empty()) else {
            return Ok(None);
        };
        let valid = id
            .strip_suffix(".apps.googleusercontent.com")
            .is_some_and(|prefix| {
                !prefix.is_empty()
                    && prefix
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            });
        if !valid {
            return Err("此版本的 Google 登录配置无效，请联系发布者".into());
        }
        let secret = secret
            .map(str::trim)
            .filter(|secret| !secret.is_empty())
            .ok_or("此版本缺少 Google 应用客户端凭据，请联系发布者")?;
        Ok(Some(Self {
            id: id.to_owned(),
            secret: Some(secret.to_owned()),
        }))
    }

    fn read_file(path: &Path, required: bool) -> Result<Option<Self>, String> {
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if !required && error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err("找不到 Google 桌面 OAuth 客户端 JSON".into());
            }
            Err(_) => return Err("无法读取 Google 桌面 OAuth 客户端 JSON".into()),
        };
        if !metadata.is_file() || metadata.len() > CLIENT_FILE_LIMIT {
            return Err("Google 桌面 OAuth 客户端 JSON 文件无效".into());
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .and_then(|file| file.take(CLIENT_FILE_LIMIT + 1).read_to_end(&mut bytes))
            .map_err(|_| "无法读取 Google 桌面 OAuth 客户端 JSON".to_owned())?;
        Self::parse_download(&bytes).map(Some)
    }

    fn parse_download(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() as u64 > CLIENT_FILE_LIMIT {
            return Err("Google 桌面 OAuth 客户端 JSON 文件无效".into());
        }
        let DesktopClientDownload {
            installed: mut client,
        } = serde_json::from_slice(bytes)
            .map_err(|_| "请选择从 Google Cloud 下载的 Desktop app OAuth 客户端 JSON".to_owned())?;
        client.id = client.id.trim().to_owned();
        if client.id.is_empty() {
            return Err("Google 桌面 OAuth 客户端 JSON 缺少客户端 ID".into());
        }
        client.secret = client
            .secret
            .map(|secret| secret.trim().to_owned())
            .filter(|secret| !secret.is_empty());
        Ok(client)
    }
}

impl AccessToken {
    fn from_reply(reply: &TokenReply) -> Result<Self, String> {
        if reply.access_token.is_empty()
            || !reply.token_type.eq_ignore_ascii_case("Bearer")
            || reply.scope.as_ref().is_some_and(|scopes| {
                !scopes.split_ascii_whitespace().any(|scope| {
                    matches!(
                        scope,
                        PLAYLIST_SCOPE | "https://www.googleapis.com/auth/youtube"
                    )
                })
            })
        {
            return Err("Google 尚未授予歌单权限，请重新连接".into());
        }
        let lifetime =
            Duration::from_secs(reply.expires_in.saturating_sub(30)).min(REFRESH_INTERVAL);
        Ok(Self {
            value: reply.access_token.clone(),
            received: SessionTime::now(),
            lifetime,
        })
    }
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn consent_url(client_id: &str, redirect: &Url, state: &str, verifier: &str) -> Url {
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url = Url::parse(CONSENT_URL).unwrap();
    url.query_pairs_mut().extend_pairs([
        ("client_id", client_id),
        ("redirect_uri", redirect.as_str()),
        ("scope", PLAYLIST_SCOPE),
        ("response_type", "code"),
        ("access_type", "offline"),
        ("prompt", "consent select_account"),
        ("state", state),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
    ]);
    url
}

async fn receive_code(
    listener: TcpListener,
    redirect: &Url,
    state: &str,
    language: Language,
) -> Result<String, String> {
    tokio::time::timeout(CALLBACK_TIMEOUT, async {
        loop {
            let (mut stream, _) = listener.accept().await.map_err(|_| "本机授权回调已关闭".to_owned())?;
            let callback = tokio::time::timeout(CONNECTION_TIMEOUT, read_callback(&mut stream))
                .await.ok().flatten().and_then(|line| parse_callback(&line, redirect, state));
            let (status, message) = match &callback {
                Some(Ok(_)) => ("200 OK", language.choose("授权已返回 Nasti。可关闭此页。", "Authorization returned to Nasti. You can close this page.")),
                Some(Err(_)) => ("200 OK", language.choose("未授予 Nasti 歌单权限。可关闭此页。", "Nasti was not granted playlist access. You can close this page.")),
                None => ("404 Not Found", language.choose("无效的授权回调。", "Invalid authorization callback.")),
            };
            let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{message}", message.len());
            let _ = tokio::time::timeout(CONNECTION_TIMEOUT, stream.write_all(response.as_bytes())).await;
            if let Some(result) = callback {
                return result;
            }
        }
    }).await.map_err(|_| "等待 Google 授权超时，请重试".to_owned())?
}

async fn read_callback(stream: &mut TcpStream) -> Option<String> {
    let mut buffer = [0; CALLBACK_LIMIT];
    let mut used = 0;
    while used < buffer.len() {
        let count = stream.read(&mut buffer[used..]).await.ok()?;
        if count == 0 {
            return None;
        }
        used += count;
        let received = &buffer[..used];
        if received.windows(4).any(|window| window == b"\r\n\r\n") {
            let first = received.iter().position(|byte| *byte == b'\r')?;
            return std::str::from_utf8(&received[..first])
                .ok()
                .map(str::to_owned);
        }
    }
    None
}

fn parse_callback(line: &str, redirect: &Url, state: &str) -> Option<Result<String, String>> {
    let mut request = line.split_ascii_whitespace();
    if request.next()? != "GET" {
        return None;
    }
    let path = request.next()?;
    if path.split_once('?')?.0 != CALLBACK_PATH {
        return None;
    }
    let url = redirect.join(path).ok()?;
    let value = |key: &str| {
        let mut matches = url
            .query_pairs()
            .filter(|(name, _)| name == key)
            .map(|(_, value)| value);
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    };
    if value("state").as_deref() != Some(state) {
        return None;
    }
    if url.query_pairs().any(|(name, _)| name == "error") {
        return Some(Err("未授予 Google 歌单权限".into()));
    }
    value("code")
        .filter(|code| !code.is_empty())
        .map(|code| Ok(code.into_owned()))
}

pub(crate) fn open_external(url: &Url) -> Result<(), String> {
    let known_page = match url.host_str() {
        Some("accounts.google.com") => url.path() == "/o/oauth2/v2/auth",
        Some("www.youtube.com" | "youtube.com" | "music.youtube.com") => {
            matches!(url.path(), "/watch" | "/playlist")
        }
        _ => false,
    };
    if url.scheme() != "https"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || !known_page
    {
        return Err("无法打开此 YouTube 链接".into());
    }
    launch_browser(url)
}

#[cfg(target_os = "macos")]
pub(crate) fn launch_browser(url: &Url) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let result = Command::new("/usr/bin/open")
        .arg(url.as_str())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    result
        .is_ok_and(|status| status.success())
        .then_some(())
        .ok_or_else(|| "无法打开系统浏览器".to_owned())
}

#[cfg(target_os = "windows")]
pub(crate) fn launch_browser(url: &Url) -> Result<(), String> {
    use std::ptr::null;
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let operation = [b'o' as u16, b'p' as u16, b'e' as u16, b'n' as u16, 0];
    let target: Vec<u16> = url.as_str().encode_utf16().chain([0]).collect();
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            target.as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        )
    };
    ((result as isize) > 32)
        .then_some(())
        .ok_or_else(|| "无法打开系统浏览器".to_owned())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) fn launch_browser(_: &Url) -> Result<(), String> {
    Err("此宿主尚未接入系统浏览器".into())
}

#[cfg(target_os = "macos")]
pub(crate) fn remove_legacy_keychain_grants() -> Result<(), String> {
    remove_keychain_service("dev.nasti.terminal.youtube-oauth")
}

#[cfg(target_os = "macos")]
fn remove_keychain_service(service: &str) -> Result<(), String> {
    use security_framework::item::{ItemClass, ItemSearchOptions, Limit};

    match ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(service)
        .limit(Limit::All) // The legacy file-based Keychain defaults to one result.
        .delete()
    {
        Ok(()) => Ok(()),
        Err(error) if error.code() == -25300 => Ok(()), // errSecItemNotFound
        Err(_) => Err("无法移除旧版 Nasti Google 授权钥匙串条目，请解锁钥匙串后重启 Nasti".into()),
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn remove_legacy_keychain_grants() -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;
    use tokio::task::JoinHandle;

    const TEST_TIMEOUT: Duration = Duration::from_secs(5);
    pub(crate) const FIRST_GRANT: &str = r#"{"token_type":"Bearer","access_token":"first-access","refresh_token":"first-refresh","expires_in":3600}"#;

    pub(crate) struct MockRequest {
        pub(crate) path: String,
        pub(crate) form: HashMap<String, String>,
    }

    pub(crate) fn mock_origin(oauth: &YoutubeOAuth) -> Url {
        oauth.api_origin.clone()
    }

    pub(crate) async fn mock_google(
        replies: Vec<(&'static str, &'static str)>,
    ) -> (YoutubeOAuth, JoinHandle<Vec<MockRequest>>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let api_origin =
            Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, body) in replies {
                let (mut stream, _) = listener.accept().await.unwrap();
                requests.push(read_mock_request(&mut stream).await);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
            requests
        });
        (mock_oauth(api_origin), server)
    }

    pub(crate) fn mock_oauth(api_origin: Url) -> YoutubeOAuth {
        YoutubeOAuth {
            client: Client::builder()
                .no_proxy()
                .timeout(TEST_TIMEOUT)
                .build()
                .unwrap(),
            api_origin,
            config: Some(DesktopClient {
                id: "desktop-client".into(),
                secret: Some("desktop-secret".into()),
            }),
            client_file: PathBuf::new(),
            staging_dir: PathBuf::new(),
            refresh_token: None,
            pending_revocation: None,
            access: None,
            verified: None,
        }
    }

    async fn read_mock_request(stream: &mut TcpStream) -> MockRequest {
        let mut received = Vec::new();
        let mut buffer = [0; 4096];
        let header_end = loop {
            let count = stream.read(&mut buffer).await.unwrap();
            assert!(count > 0);
            received.extend_from_slice(&buffer[..count]);
            if let Some(position) = received.windows(4).position(|part| part == b"\r\n\r\n") {
                break position;
            }
        };
        let headers = std::str::from_utf8(&received[..header_end]).unwrap();
        let mut lines = headers.lines();
        let mut request = lines.next().unwrap().split_ascii_whitespace();
        assert_eq!(request.next(), Some("POST"));
        let path = request.next().unwrap().to_owned();
        let length: usize = lines
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("Content-Length")
                    .then(|| value.trim().parse().unwrap())
            })
            .unwrap();
        let body_start = header_end + 4;
        while received.len() < body_start + length {
            let count = stream.read(&mut buffer).await.unwrap();
            assert!(count > 0);
            received.extend_from_slice(&buffer[..count]);
        }
        let body = std::str::from_utf8(&received[body_start..body_start + length]).unwrap();
        let url = Url::parse(&format!("http://127.0.0.1/?{body}")).unwrap();
        let form = url.query_pairs().into_owned().collect();
        MockRequest { path, form }
    }

    pub(crate) async fn connect_in_mock_browser(
        oauth: &mut YoutubeOAuth,
        response: (&'static str, &'static str),
    ) -> (Result<(), String>, HashMap<String, String>) {
        let mut browser = None;
        let (_cancel, mut cancellation) = watch::channel(0);
        let result = tokio::time::timeout(
            TEST_TIMEOUT,
            oauth.connect_with(
                |consent| {
                    assert_eq!(consent.host_str(), Some("accounts.google.com"));
                    assert_eq!(consent.path(), "/o/oauth2/v2/auth");
                    let params: HashMap<_, _> = consent.query_pairs().into_owned().collect();
                    assert_eq!(params["client_id"], "desktop-client");
                    assert_eq!(params["scope"], PLAYLIST_SCOPE);
                    assert_eq!(params["access_type"], "offline");
                    assert_eq!(params["code_challenge_method"], "S256");
                    assert!(!params.contains_key("client_secret"));
                    let mut callback = Url::parse(&params["redirect_uri"]).unwrap();
                    assert_eq!(callback.host_str(), Some("127.0.0.1"));
                    callback
                        .query_pairs_mut()
                        .append_pair("state", &params["state"])
                        .append_pair(response.0, response.1);
                    let task = tokio::spawn(async move {
                        let reply = Client::builder()
                            .no_proxy()
                            .timeout(TEST_TIMEOUT)
                            .build()
                            .unwrap()
                            .get(callback)
                            .send()
                            .await
                            .unwrap();
                        assert_eq!(reply.status(), StatusCode::OK);
                        assert_eq!(reply.headers()["Cache-Control"], "no-store");
                        assert!(!reply.text().await.unwrap().contains(response.1));
                    });
                    browser = Some((params, task));
                    Ok(())
                },
                &mut cancellation,
            ),
        )
        .await
        .unwrap();
        let (params, task) = browser.unwrap();
        task.await.unwrap();
        (result, params)
    }

    #[test]
    fn publisher_identity_requires_both_application_credentials() {
        assert!(DesktopClient::app_identity(None, None).unwrap().is_none());
        assert!(DesktopClient::app_identity(Some("  "), None)
            .unwrap()
            .is_none());
        assert!(DesktopClient::app_identity(Some("not-a-google-client"), Some("fixture")).is_err());
        assert!(DesktopClient::app_identity(
            Some("https://example.apps.googleusercontent.com"),
            Some("fixture")
        )
        .is_err());
        assert!(
            DesktopClient::app_identity(Some("12345-nasti.apps.googleusercontent.com"), None)
                .is_err()
        );
        let client = DesktopClient::app_identity(
            Some(" 12345-nasti.apps.googleusercontent.com "),
            Some(" fixture-application-secret "),
        )
        .unwrap()
        .unwrap();
        assert_eq!(client.id, "12345-nasti.apps.googleusercontent.com");
        assert_eq!(client.secret.as_deref(), Some("fixture-application-secret"));
    }

    #[tokio::test]
    async fn cancel_closes_the_loopback_and_allows_a_new_attempt() {
        let (mut oauth, google) = mock_google(vec![("200 OK", FIRST_GRANT)]).await;
        let (cancel, mut cancellation) = watch::channel(0);
        let mut callback = None;
        let error = oauth
            .connect_with(
                |consent| {
                    callback = consent
                        .query_pairs()
                        .find(|(name, _)| name == "redirect_uri")
                        .map(|(_, uri)| Url::parse(&uri).unwrap());
                    cancel.send_replace(1);
                    Ok(())
                },
                &mut cancellation,
            )
            .await
            .unwrap_err();
        assert_eq!(error, CANCELLED);
        let callback = callback.unwrap();
        assert!(
            TcpStream::connect((Ipv4Addr::LOCALHOST, callback.port().unwrap()))
                .await
                .is_err()
        );
        assert!(!oauth.has_session());
        assert!(oauth.access_token().await.unwrap().is_none());

        // A cancellation queued before browser launch must not open a stale login.
        cancel.send_replace(2);
        assert_eq!(
            oauth
                .connect_with(
                    |_| panic!("canceled login opened a browser"),
                    &mut cancellation
                )
                .await
                .unwrap_err(),
            CANCELLED
        );

        // A subsequent attempt receives a fresh subscription and a fresh PKCE challenge.
        let (result, _) = connect_in_mock_browser(&mut oauth, ("code", "retry-code")).await;
        result.unwrap();
        assert!(oauth.has_session());
        let requests = google.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].form["code"], "retry-code");
    }

    #[tokio::test]
    async fn legacy_secretless_configuration_omits_secret_and_keeps_pkce() {
        let (mut oauth, google) = mock_google(vec![("200 OK", FIRST_GRANT)]).await;
        oauth.config.as_mut().unwrap().secret = None;
        let (result, _) = connect_in_mock_browser(&mut oauth, ("code", "public-client-code")).await;
        result.unwrap();
        let requests = google.await.unwrap();
        assert!(!requests[0].form.contains_key("client_secret"));
        assert!(requests[0].form.contains_key("code_verifier"));
    }

    #[tokio::test]
    async fn missing_app_credentials_do_not_save_a_grant() {
        let (mut oauth, google) = mock_google(vec![(
            "400 Bad Request",
            r#"{"error":"invalid_request","error_description":"client_secret is missing."}"#,
        )])
        .await;
        let (result, _) = connect_in_mock_browser(&mut oauth, ("code", "test-code")).await;
        assert_eq!(
            result.unwrap_err(),
            "此版本缺少 Google 应用客户端凭据，请联系发布者"
        );
        assert!(!oauth.has_session());
        assert!(oauth.access_token().await.unwrap().is_none());
        assert_eq!(google.await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn browser_launch_failure_leaves_no_listener_or_grant() {
        let (mut oauth, google) = mock_google(Vec::new()).await;
        let (_cancel, mut cancellation) = watch::channel(0);
        let mut port = None;
        let result = oauth
            .connect_with(
                |consent| {
                    let redirect = consent
                        .query_pairs()
                        .find(|(name, _)| name == "redirect_uri")
                        .unwrap()
                        .1;
                    port = Url::parse(&redirect).unwrap().port();
                    Err("无法打开系统浏览器".into())
                },
                &mut cancellation,
            )
            .await;
        assert_eq!(result.unwrap_err(), "无法打开系统浏览器");
        assert!(TcpStream::connect((Ipv4Addr::LOCALHOST, port.unwrap()))
            .await
            .is_err());
        assert!(!oauth.has_session());
        assert!(google.await.unwrap().is_empty());
    }

    #[test]
    fn reads_google_desktop_client_download() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("youtube-oauth-client.json");
        std::fs::write(
            &path,
            r#"{"installed":{"client_id":"test.apps.googleusercontent.com","client_secret":"test-secret","token_uri":"https://ignored.example/token"}}"#,
        )
        .unwrap();
        let client = DesktopClient::read_file(&path, false).unwrap().unwrap();
        assert_eq!(client.id, "test.apps.googleusercontent.com");
        assert_eq!(client.secret.as_deref(), Some("test-secret"));

        std::fs::write(&path, r#"{"web":{"client_id":"wrong-type"}}"#).unwrap();
        assert!(DesktopClient::read_file(&path, false).is_err());
        let absent = directory.path().join("absent.json");
        assert!(DesktopClient::read_file(&absent, false).unwrap().is_none());
        assert!(DesktopClient::read_file(&absent, true).is_err());
    }

    #[tokio::test]
    async fn configures_an_empty_install_with_private_persistence_and_no_overwrite() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("youtube-oauth-client.json");
        let (mut oauth, server) = mock_google(Vec::new()).await;
        oauth.config = None;
        oauth.client_file = path.clone();
        oauth.staging_dir = directory.path().join("staging");
        std::fs::create_dir(&oauth.staging_dir).unwrap();
        assert!(oauth.access_token().await.unwrap().is_none());

        let web_client = r#"{"web":{"client_id":"synthetic-web-client"}}"#;
        assert!(oauth.configure_client(web_client).is_err());
        assert!(oauth
            .configure_client(&"x".repeat(CLIENT_FILE_LIMIT as usize + 1))
            .is_err());
        assert!(!path.exists());
        assert!(!oauth.configured());

        let desktop_client = r#"{"installed":{"client_id":" fixture.apps.googleusercontent.com ","client_secret":" synthetic "}}"#;
        // A file created after launch may belong to the user. A first-time import must preserve it.
        std::fs::write(&path, "existing").unwrap();
        assert!(oauth.configure_client(desktop_client).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "existing");
        assert!(!oauth.configured());
        std::fs::remove_file(&path).unwrap();

        oauth.configure_client(desktop_client).unwrap();
        assert!(oauth.configured());
        assert!(!oauth.has_session());
        assert!(oauth.configure_client(desktop_client).is_err());
        let reloaded = DesktopClient::read_file(&path, true).unwrap().unwrap();
        assert_eq!(reloaded.id, "fixture.apps.googleusercontent.com");
        assert_eq!(reloaded.secret.as_deref(), Some("synthetic"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert_eq!(std::fs::read_dir(&oauth.staging_dir).unwrap().count(), 0);
        assert!(server.await.unwrap().is_empty());
    }

    #[test]
    fn consent_derives_pkce_challenge_without_sending_the_verifier() {
        let redirect = Url::parse("http://127.0.0.1:49152/").unwrap();
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let url = consent_url("client-id", &redirect, "expected-state", verifier);
        let params = url
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            params["code_challenge"],
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(params["state"], "expected-state");
        assert_eq!(params["redirect_uri"], redirect.as_str());
        assert!(!url.as_str().contains(verifier));
    }

    #[tokio::test]
    async fn loopback_checks_state_before_accepting_consent() {
        async fn request(address: std::net::SocketAddr, query: &str) -> String {
            let mut stream = TcpStream::connect(address).await.unwrap();
            stream
                .write_all(
                    format!("GET {CALLBACK_PATH}?{query} HTTP/1.1\r\nHost: {address}\r\n\r\n")
                        .as_bytes(),
                )
                .await
                .unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            response
        }
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let redirect = Url::parse(&format!("http://{address}{CALLBACK_PATH}")).unwrap();
        let receiver = tokio::spawn(async move {
            receive_code(listener, &redirect, "expected", Language::English).await
        });
        assert!(request(address, "state=wrong&code=never-use")
            .await
            .starts_with("HTTP/1.1 404"));
        assert!(
            request(address, "state=expected&state=duplicate&code=never-use")
                .await
                .starts_with("HTTP/1.1 404")
        );
        let response = request(address, "state=expected&code=return%2Fvalue").await;
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("Authorization returned to Nasti"));
        assert!(!response.contains("return/value"));
        assert_eq!(receiver.await.unwrap().unwrap(), "return/value");
    }

    #[tokio::test]
    async fn oauth_grant_refresh_and_revoke_follow_the_browser_and_google_contract() {
        let (mut oauth, google) = mock_google(vec![
            ("200 OK", FIRST_GRANT),
            ("200 OK", r#"{"token_type":"Bearer","access_token":"next-access","refresh_token":"next-refresh","expires_in":3600}"#),
            ("503 Service Unavailable", "{}"),
            ("200 OK", "{}"),
        ]).await;
        let (result, consent) =
            connect_in_mock_browser(&mut oauth, ("code", "granted/code+")).await;
        result.unwrap();
        assert!(oauth.has_session());
        assert_eq!(
            oauth.access_token().await.unwrap().as_deref(),
            Some("first-access")
        );

        oauth.invalidate_access_token();
        assert_eq!(
            oauth.access_token().await.unwrap().as_deref(),
            Some("next-access")
        );
        assert!(oauth.disconnect().await.is_err());
        assert!(!oauth.has_session());
        assert!(oauth.access_token().await.unwrap().is_none());
        assert!(oauth.pending_revocation.is_some());
        oauth.retry_revoke().await.unwrap();
        assert!(oauth.pending_revocation.is_none());
        assert!(!oauth.has_session());
        assert!(oauth.access_token().await.unwrap().is_none());

        let requests = tokio::time::timeout(TEST_TIMEOUT, google)
            .await
            .unwrap()
            .unwrap();
        let grant = &requests[0];
        assert_eq!(grant.path, "/token");
        assert_eq!(grant.form["grant_type"], "authorization_code");
        assert_eq!(grant.form["code"], "granted/code+");
        assert_eq!(grant.form["redirect_uri"], consent["redirect_uri"]);
        assert_eq!(grant.form["client_id"], "desktop-client");
        assert_eq!(grant.form["client_secret"], "desktop-secret");
        let verifier = &grant.form["code_verifier"];
        assert!((43..=128).contains(&verifier.len()));
        assert_eq!(
            consent["code_challenge"],
            URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
        );
        assert_eq!(requests[1].path, "/token");
        assert_eq!(requests[1].form["grant_type"], "refresh_token");
        assert_eq!(requests[1].form["refresh_token"], "first-refresh");
        for revoke in &requests[2..] {
            assert_eq!(revoke.path, "/revoke");
            assert_eq!(revoke.form["token"], "next-refresh");
        }
    }

    #[tokio::test]
    async fn oauth_denial_and_invalid_refresh_leave_no_account_session() {
        let (mut oauth, google) = mock_google(vec![
            ("200 OK", FIRST_GRANT),
            ("400 Bad Request", r#"{"error":"invalid_grant"}"#),
        ])
        .await;
        let (result, _) = connect_in_mock_browser(&mut oauth, ("error", "access_denied")).await;
        assert_eq!(result.unwrap_err(), "未授予 Google 歌单权限");
        assert!(oauth.access_token().await.unwrap().is_none());
        let (result, _) = connect_in_mock_browser(&mut oauth, ("code", "consent-code")).await;
        result.unwrap();
        oauth.invalidate_access_token();
        assert!(oauth.access_token().await.unwrap().is_none());
        assert!(!oauth.has_session());
        assert!(oauth.access_token().await.unwrap().is_none());
        let requests = tokio::time::timeout(TEST_TIMEOUT, google)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(requests[0].form["code"], "consent-code");
        assert_eq!(requests[1].form["refresh_token"], "first-refresh");
    }
}
