use crate::catalog::SourcePlaylist;
use crate::models::{AlbumRef, ArchiveSearchResult, ArchiveTrackMatch, ArtistRef, TrackItem};
use reqwest::{Client, RequestBuilder, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const LABEL: &str = "unfoundsound";
const API_ROOT: &str = "https://archive.org";
const LICENSE_QUERY: &str = concat!(
    "mediatype:audio AND collection:unfoundsound AND (",
    "licenseurl:\"https://creativecommons.org/licenses/by/4.0/\" OR ",
    "licenseurl:\"http://creativecommons.org/licenses/by/4.0/\" OR ",
    "licenseurl:\"https://creativecommons.org/publicdomain/zero/1.0/\" OR ",
    "licenseurl:\"http://creativecommons.org/publicdomain/zero/1.0/\")",
);
const MAX_METADATA_BYTES: usize = 4 * 1024 * 1024;
const INDEX_TTL: Duration = Duration::from_secs(600);
const UA: &str = concat!(
    "NASTI/",
    env!("CARGO_PKG_VERSION"),
    " (unfoundsound catalog)"
);

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("请使用 Internet Archive 的 /details/ 专辑链接")]
    InvalidLink,
    #[error("该专辑缺少 unfoundsound 厂牌记录、可用的许可或公开 MP3")]
    Unavailable,
    #[error("Internet Archive 请求过多，请稍后重试")]
    RateLimited,
    #[error("无法连接 Internet Archive")]
    Network(#[source] reqwest::Error),
    #[error("Internet Archive 返回的目录数据无效")]
    Metadata,
}

impl From<reqwest::Error> for ArchiveError {
    fn from(error: reqwest::Error) -> Self {
        Self::Network(error.without_url())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ArchiveRecording {
    pub track: TrackItem,
    pub item_id: String,
    pub filename: String,
    pub size: u64,
    pub sha1: String,
}

pub struct ArchiveAlbum {
    pub(crate) source: SourcePlaylist,
    pub(crate) recordings: Vec<ArchiveRecording>,
    artist: String,
    license: License,
}

pub struct ArchiveCatalog {
    client: Client,
    index: Mutex<Option<(Instant, Vec<ArchiveAlbum>)>>,
}

impl ArchiveCatalog {
    pub fn new() -> Result<Self, ArchiveError> {
        let client = Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(20))
            .user_agent(UA)
            .build()?;
        Ok(Self {
            client,
            index: Mutex::new(None),
        })
    }

    /// Leaves the existing bundled playlist identifiers in the local catalog.
    pub fn item_reference(input: &str) -> Result<Option<String>, ArchiveError> {
        let input = input.trim();
        if let Some(id) = input.strip_prefix("archive:") {
            return valid_item_id(id)
                .then(|| Some(id.to_owned()))
                .ok_or(ArchiveError::InvalidLink);
        }
        if !input.starts_with("http://") && !input.starts_with("https://") {
            return Ok(None);
        }
        let url = Url::parse(input).map_err(|_| ArchiveError::InvalidLink)?;
        if !matches!(url.host_str(), Some("archive.org" | "www.archive.org"))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
        {
            return Err(ArchiveError::InvalidLink);
        }
        let id = url
            .path()
            .trim_end_matches('/')
            .strip_prefix("/details/")
            .filter(|id| valid_item_id(id))
            .ok_or(ArchiveError::InvalidLink)?;
        Ok(Some(id.to_owned()))
    }

    pub async fn search(&self, query: &str) -> Result<Vec<ArchiveSearchResult>, ArchiveError> {
        let mut index = self.index.lock().await;
        if index
            .as_ref()
            .is_none_or(|(refreshed, _)| refreshed.elapsed() >= INDEX_TTL)
        {
            let data: IndexResponse = read_json(
                self.client
                    .get(format!("{API_ROOT}/advancedsearch.php"))
                    .query(&[
                        ("q", LICENSE_QUERY),
                        ("fl[]", "identifier"),
                        ("sort[]", "downloads desc"),
                        ("rows", "50"),
                        ("output", "json"),
                    ]),
            )
            .await?;
            let mut albums = Vec::new();
            for entry in data.response.docs {
                match self.album(&entry.identifier).await {
                    Ok(album) => albums.push(album),
                    Err(ArchiveError::Unavailable) => {}
                    Err(error) => return Err(error),
                }
            }
            *index = Some((Instant::now(), albums));
        }
        let query = query.trim().to_lowercase();
        Ok(index
            .as_ref()
            .into_iter()
            .flat_map(|(_, albums)| albums)
            .filter_map(|album| album.search_result(&query))
            .collect())
    }

    pub async fn album(&self, id: &str) -> Result<ArchiveAlbum, ArchiveError> {
        if !valid_item_id(id) {
            return Err(ArchiveError::InvalidLink);
        }
        let data: Value = read_json(self.client.get(format!("{API_ROOT}/metadata/{id}"))).await?;
        parse_album(id, data)
    }
}

impl ArchiveAlbum {
    fn search_result(&self, query: &str) -> Option<ArchiveSearchResult> {
        let matches = |name: &str| name.to_lowercase().contains(query);
        let recording = (!query.is_empty())
            .then(|| {
                self.recordings.iter().find(|recording| {
                    matches(&recording.track.title)
                        || recording
                            .track
                            .artists
                            .iter()
                            .any(|artist| matches(&artist.name))
                })
            })
            .flatten();
        let item_id = &self.recordings.first()?.item_id;
        if recording.is_none()
            && !matches(item_id)
            && !matches(&self.source.title)
            && !matches(&self.artist)
        {
            return None;
        }
        Some(ArchiveSearchResult {
            title: self.source.title.clone(),
            artist: self.artist.clone(),
            license: self.license.name(),
            license_url: self.license.url(),
            source_url: format!("{API_ROOT}/details/{item_id}"),
            track_count: self.recordings.len(),
            matching_track: recording.map(|recording| ArchiveTrackMatch {
                title: recording.track.title.clone(),
                video_id: recording.track.video_id.clone(),
                artist: recording.track.artists[0].name.clone(),
            }),
        })
    }
}

#[derive(Deserialize)]
struct IndexResponse {
    response: IndexDocs,
}

#[derive(Deserialize)]
struct IndexDocs {
    docs: Vec<IndexItem>,
}

#[derive(Deserialize)]
struct IndexItem {
    identifier: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum License {
    CcBy4,
    Cc0,
}

impl License {
    fn parse(value: &Value) -> Option<Self> {
        let url = Url::parse(value.as_str()?.trim()).ok()?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str() != Some("creativecommons.org")
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return None;
        }
        match url.path().trim_end_matches('/') {
            "/licenses/by/4.0" => Some(Self::CcBy4),
            "/publicdomain/zero/1.0" => Some(Self::Cc0),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::CcBy4 => "CC BY 4.0",
            Self::Cc0 => "CC0 1.0",
        }
    }

    fn url(self) -> &'static str {
        match self {
            Self::CcBy4 => "https://creativecommons.org/licenses/by/4.0/",
            Self::Cc0 => "https://creativecommons.org/publicdomain/zero/1.0/",
        }
    }
}

fn parse_album(id: &str, data: Value) -> Result<ArchiveAlbum, ArchiveError> {
    let meta = &data["metadata"];
    let licensed = License::parse(&meta["licenseurl"]).ok_or(ArchiveError::Unavailable)?;
    let title = field(&meta["title"]).ok_or(ArchiveError::Unavailable)?;
    let primary_collection = meta["collection"]
        .as_str()
        .or_else(|| meta["collection"].get(0)?.as_str());
    if meta["identifier"] != id
        || meta["mediatype"] != "audio"
        || primary_collection != Some(LABEL)
        || declared(&data["is_dark"])
        || declared(&meta["access-restricted-item"])
    {
        return Err(ArchiveError::Unavailable);
    }

    let artist = field(&meta["creator"]).unwrap_or("unfoundsound");
    let mut ordered: Vec<_> = data["files"]
        .as_array()
        .ok_or(ArchiveError::Unavailable)?
        .iter()
        .filter_map(|file| parse_recording(id, file, title, artist, licensed))
        .collect();
    ordered.sort_by(|(number, left), (other, right)| {
        number
            .cmp(other)
            .then_with(|| left.filename.cmp(&right.filename))
    });
    ordered.dedup_by(|(_, a), (_, b)| a.track.video_id == b.track.video_id);
    let recordings: Vec<_> = ordered.into_iter().map(|(_, record)| record).collect();
    if recordings.is_empty() {
        return Err(ArchiveError::Unavailable);
    }
    Ok(ArchiveAlbum {
        source: SourcePlaylist {
            id: format!("archive:{id}"),
            title: title.to_owned(),
            subtitle: Some(format!("unfoundsound · {}", licensed.name())),
            video_ids: recordings
                .iter()
                .map(|record| record.track.video_id.clone())
                .collect(),
        },
        artist: artist.to_owned(),
        license: licensed,
        recordings,
    })
}

fn parse_recording(
    id: &str,
    file: &Value,
    album: &str,
    artist: &str,
    license: License,
) -> Option<(u32, ArchiveRecording)> {
    let filename = file["name"].as_str()?;
    let length = file["size"]
        .as_u64()
        .or_else(|| file["size"].as_str()?.parse().ok())?;
    let digest = file["sha1"].as_str()?;
    if file["source"] != "original"
        || declared(&file["private"])
        || (!file["licenseurl"].is_null() && License::parse(&file["licenseurl"]) != Some(license))
        || !file["format"]
            .as_str()
            .is_some_and(|format| format.ends_with("MP3"))
        || !filename.to_ascii_lowercase().ends_with(".mp3")
        || filename.contains(['\\', '\0'])
        || filename.chars().any(char::is_control)
        || filename
            .split('/')
            .any(|part| matches!(part, "" | "." | ".."))
        || !(1..=2 * 1024 * 1024 * 1024).contains(&length)
        || digest.len() != 40
        || !digest.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    let sha1 = digest.to_ascii_lowercase();
    let digest = Sha256::new()
        .chain_update(id)
        .chain_update([0])
        .chain_update(filename)
        .chain_update([0])
        .chain_update(&sha1)
        .finalize();
    let media_id = format!("ia-{:x}", digest)[..43].to_owned();
    let basename = filename.rsplit('/').next()?.rsplit_once('.')?.0;
    let number = field(&file["track"])
        .and_then(|text| text.split('/').next()?.trim().parse::<u32>().ok())
        .or_else(|| u32::try_from(file["track"].as_u64()?).ok());
    let track = TrackItem {
        video_id: media_id,
        mv_video_id: None,
        mv_timeline_aligned: false,
        entry_id: None,
        title: field(&file["title"]).unwrap_or(basename).to_owned(),
        artists: vec![ArtistRef {
            name: field(&file["creator"]).unwrap_or(artist).to_owned(),
            browse_id: None,
        }],
        album: Some(AlbumRef {
            name: album.to_owned(),
            browse_id: None,
        }),
        duration_sec: duration(&file["length"]),
        thumbnail_square_url: String::new(),
        thumbnail_wide_url: None,
        is_vault_downloaded: false,
        is_vault_video_downloaded: false,
        loudness_db: None,
        track_number: number,
        source_name: Some("Internet Archive · unfoundsound".into()),
        source_url: Some(format!("{API_ROOT}/details/{id}")),
        license: Some(license.name().into()),
        license_url: Some(license.url().into()),
        audio_url: None,
        mv_url: None,
    };
    Some((
        number.unwrap_or(u32::MAX),
        ArchiveRecording {
            track,
            item_id: id.to_owned(),
            filename: filename.to_owned(),
            size: length,
            sha1,
        },
    ))
}

fn valid_item_id(id: &str) -> bool {
    id.len() <= 100
        && id.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

fn field(value: &Value) -> Option<&str> {
    value
        .as_str()
        .or_else(|| value.get(0)?.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn declared(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::String(text) => !matches!(
            text.trim().to_ascii_lowercase().as_str(),
            "" | "0" | "no" | "false"
        ),
        _ => true,
    }
}

fn duration(value: &Value) -> u32 {
    let seconds = value.as_f64().or_else(|| {
        field(value)?.split(':').try_fold(0.0, |total, part| {
            Some(total * 60.0 + part.parse::<f64>().ok()?)
        })
    });
    seconds
        .filter(|value| value.is_finite())
        .map_or(0, |value| value.round() as u32)
}

#[cfg(test)]
pub(crate) fn fixture_album() -> ArchiveAlbum {
    parse_album("unfound-test", tests::album_fixture()).unwrap()
}

async fn read_json<T: serde::de::DeserializeOwned>(
    request: RequestBuilder,
) -> Result<T, ArchiveError> {
    let mut response = request.send().await?;
    match response.status() {
        StatusCode::OK => {}
        StatusCode::TOO_MANY_REQUESTS => return Err(ArchiveError::RateLimited),
        StatusCode::NOT_FOUND => return Err(ArchiveError::Unavailable),
        _ => return Err(ArchiveError::Metadata),
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > MAX_METADATA_BYTES {
            return Err(ArchiveError::Metadata);
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| ArchiveError::Metadata)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn album_fixture() -> Value {
        json!({
            "metadata":{"identifier":"unfound-test","title":"Open release","creator":"Artist",
                "collection":["unfoundsound","netlabels"],"mediatype":"audio",
                "licenseurl":"http://creativecommons.org/licenses/by/4.0/"},
            "files":[
                {"name":"02.mp3","source":"original","title":"Quiet track","length":"2:11","track":"02/02",
                    "format":"VBR MP3","size":"8","sha1":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
                {"name":"01.mp3","source":"original","title":"Opening track","length":"55.1","track":"01/02",
                    "format":"VBR MP3","size":12,"sha1":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
                {"name":"03.mp3","source":"original","format":"MP3","size":"4","private":"true",
                    "sha1":"cccccccccccccccccccccccccccccccccccccccc"},
                {"name":"04.mp3","source":"original","format":"MP3","size":"4",
                    "sha1":"dddddddddddddddddddddddddddddddddddddddd",
                    "licenseurl":"https://creativecommons.org/licenses/by-nc/4.0/"}
            ]
        })
    }

    #[test]
    fn accepts_only_provider_album_links_and_ids() {
        assert_eq!(
            ArchiveCatalog::item_reference("https://archive.org/details/unfound88#tracks")
                .unwrap()
                .as_deref(),
            Some("unfound88")
        );
        assert_eq!(
            ArchiveCatalog::item_reference("nasti://playlist/orbital").unwrap(),
            None
        );
        for invalid in [
            "https://archive.org.example.com/details/a",
            "https://archive.org/details/a%2fb",
            "https://user@archive.org/details/unfound88",
            "archive:../g",
        ] {
            assert!(matches!(
                ArchiveCatalog::item_reference(invalid),
                Err(ArchiveError::InvalidLink)
            ));
        }
    }

    #[test]
    fn checks_item_and_file_permissions_orders_tracks_and_matches_song_titles() {
        let mut metadata = album_fixture();
        let album = parse_album("unfound-test", metadata.clone()).unwrap();
        assert_eq!(album.recordings.len(), 2);
        assert_eq!(album.recordings[0].track.title, "Opening track");
        assert_eq!(album.recordings[1].track.duration_sec, 131);
        let hit = album.search_result("quiet").unwrap();
        assert_eq!(hit.matching_track.unwrap().title, "Quiet track");
        assert!(album.search_result("unrelated").is_none());
        assert_eq!(album.search_result("").unwrap().license, "CC BY 4.0");
        metadata["metadata"]["collection"] = json!(["other", "unfoundsound"]);
        assert!(matches!(
            parse_album("unfound-test", metadata),
            Err(ArchiveError::Unavailable)
        ));
        let mut restricted = album_fixture();
        restricted["metadata"]["access-restricted-item"] = json!("true");
        assert!(matches!(
            parse_album("unfound-test", restricted),
            Err(ArchiveError::Unavailable)
        ));
    }
}
