use crate::archive_media::{ArchiveHttp, ArchiveMedia, MP3_MIME};
use crate::vault::{valid_media_id, Vault, VaultError, VaultPlaybackFile};
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header::{
    ACCEPT_RANGES, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE,
};
use axum::http::{HeaderMap, Method, Response, StatusCode};
use axum::routing::get;
use axum::Router;
use futures_util::StreamExt;
use std::{
    collections::HashMap,
    io,
    io::SeekFrom,
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

pub struct MediaSource {
    pub id: String,
    pub audio_path: PathBuf,
    pub video_path: Option<PathBuf>,
}

impl MediaSource {
    fn path(&self, mode: &str) -> Option<&PathBuf> {
        match mode {
            "audio" => Some(&self.audio_path),
            "video" => self.video_path.as_ref(),
            _ => None,
        }
    }
}

struct Registry {
    token: String,
    sources: HashMap<String, MediaSource>,
    playbacks: RwLock<HashMap<String, Arc<PlaybackEntry>>>,
    http: ArchiveHttp,
    vault: Arc<Vault>,
}

enum OriginalMedia {
    Bundled(PathBuf),
    Archive(ArchiveMedia),
}

struct PlaybackEntry {
    media_id: String,
    mode: String,
    original: OriginalMedia,
    vault: Mutex<PlaybackVault>,
}

struct PlaybackVault {
    active: bool,
    file: Option<VaultPlaybackFile>,
}

impl PlaybackEntry {
    fn vault_file(&self, vault: &Vault) -> Option<VaultPlaybackFile> {
        let mut state = self.vault.lock().ok()?;
        if !state.active {
            return vault.playback_file(&self.media_id, &self.mode);
        }
        if state.file.is_none() {
            state.file = vault.playback_file(&self.media_id, &self.mode);
        }
        state.file.clone()
    }
}

/// Owns the Range URL assigned to one native player's current media item.
pub struct PlaybackSource {
    key: String,
    url: String,
    entry: Arc<PlaybackEntry>,
    registry: Arc<Registry>,
}

impl PlaybackSource {
    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn finish(&self) {
        let mut state = self
            .entry
            .vault
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.active = false;
        state.file = None;
    }

    pub fn restart(&self) {
        let mut state = self
            .entry
            .vault
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !state.active {
            state.active = true;
            state.file = self
                .registry
                .vault
                .playback_file(&self.entry.media_id, &self.entry.mode);
        }
    }
}

impl Drop for PlaybackSource {
    fn drop(&mut self) {
        self.finish();
        let mut playbacks = self
            .registry
            .playbacks
            .write()
            .unwrap_or_else(|error| error.into_inner());
        playbacks.remove(&self.key);
    }
}

pub struct MediaPipe {
    origin: String,
    registry: Arc<Registry>,
    server: JoinHandle<()>,
}

impl MediaPipe {
    pub async fn start(sources: Vec<MediaSource>, vault: Arc<Vault>) -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let registry = Arc::new(Registry {
            token: Uuid::new_v4().simple().to_string(),
            sources: sources
                .into_iter()
                .filter(|source| valid_media_id(&source.id))
                .map(|source| (source.id.clone(), source))
                .collect(),
            playbacks: RwLock::new(HashMap::new()),
            http: ArchiveHttp::new().map_err(io::Error::other)?,
            vault,
        });
        let router = Router::new()
            .route("/v1/:token/:session", get(serve).head(serve))
            .with_state(Arc::clone(&registry));
        let server = tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, router).await {
                tracing::error!(%error, "Authorized media loopback server stopped");
            }
        });
        Ok(Self {
            origin,
            registry,
            server,
        })
    }

    pub fn bundled_playback(&self, id: &str, mode: &str) -> Option<PlaybackSource> {
        let path = self.registry.sources.get(id)?.path(mode)?;
        self.begin_playback(id, mode, OriginalMedia::Bundled(path.clone()))
    }

    pub fn archive_playback(&self, id: &str, media: ArchiveMedia) -> Option<PlaybackSource> {
        if !valid_media_id(id) || self.registry.sources.contains_key(id) {
            return None;
        }
        self.begin_playback(id, "audio", OriginalMedia::Archive(media))
    }

    fn begin_playback(
        &self,
        media_id: &str,
        mode: &str,
        original: OriginalMedia,
    ) -> Option<PlaybackSource> {
        let key = Uuid::new_v4().simple().to_string();
        let mut playbacks = self.registry.playbacks.write().ok()?;
        let entry = Arc::new(PlaybackEntry {
            media_id: media_id.to_owned(),
            mode: mode.to_owned(),
            original,
            vault: Mutex::new(PlaybackVault {
                active: true,
                file: self.registry.vault.playback_file(media_id, mode),
            }),
        });
        playbacks.insert(key.clone(), Arc::clone(&entry));
        Some(PlaybackSource {
            url: format!("{}/v1/{}/{key}", self.origin, self.registry.token),
            key,
            entry,
            registry: Arc::clone(&self.registry),
        })
    }

    pub async fn remove_saved(&self, id: String) -> Result<bool, VaultError> {
        let registry = Arc::clone(&self.registry);
        tokio::task::spawn_blocking(move || {
            registry.vault.remove_with(&id, || {
                let sessions = registry.playbacks.read().map_err(|_| VaultError::Busy)?;
                for playback in sessions.values().filter(|playback| playback.media_id == id) {
                    let mut state = playback.vault.lock().map_err(|_| VaultError::Busy)?;
                    if state.active && state.file.is_none() {
                        state.file = registry.vault.playback_file(&id, &playback.mode);
                    }
                }
                Ok(())
            })
        })
        .await
        .map_err(|_| VaultError::Busy)?
    }

    pub async fn save_archive(&self, id: String, media: ArchiveMedia) -> Result<(), VaultError> {
        let registry = Arc::clone(&self.registry);
        // Keep the atomic save alive if an IPC caller disconnects during the download.
        tokio::spawn(async move {
            registry
                .vault
                .download_archive(&id, &media, &registry.http)
                .await
        })
        .await
        .map_err(|_| VaultError::Busy)?
    }
}

impl Drop for MediaPipe {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn serve(
    State(registry): State<Arc<Registry>>,
    Path((token, session)): Path<(String, String)>,
    method: Method,
    headers: HeaderMap,
) -> Response<Body> {
    if token != registry.token {
        return empty_response(StatusCode::NOT_FOUND);
    }
    let playback = registry
        .playbacks
        .read()
        .ok()
        .and_then(|playbacks| playbacks.get(&session).cloned());
    let Some(playback) = playback else {
        return empty_response(StatusCode::NOT_FOUND);
    };
    let mime = match (&playback.original, playback.mode.as_str()) {
        (_, "video") => "video/mp4",
        (OriginalMedia::Archive(_), _) => MP3_MIME,
        _ => "audio/mp4",
    };

    if let Some(lease) = playback.vault_file(&registry.vault) {
        if let Ok(file) = lease.open() {
            return serve_file(File::from_std(file), &headers, method, mime, Some(lease)).await;
        }
    }
    match &playback.original {
        OriginalMedia::Bundled(path) => match File::open(path).await {
            Ok(file) => serve_file(file, &headers, method, mime, None).await,
            Err(_) => empty_response(StatusCode::NOT_FOUND),
        },
        OriginalMedia::Archive(media) => {
            serve_archive(&registry.http, media, &headers, method).await
        }
    }
}

struct ResponseRange {
    start: u64,
    length: u64,
    partial: bool,
}

impl ResponseRange {
    fn from_headers(headers: &HeaderMap, size: u64) -> Option<Self> {
        match headers.get(RANGE) {
            Some(range) => {
                let (start, length) = byte_range(range.to_str().ok()?, size)?;
                Some(Self {
                    start,
                    length,
                    partial: true,
                })
            }
            None => Some(Self {
                start: 0,
                length: size,
                partial: false,
            }),
        }
    }

    fn end(&self) -> u64 {
        self.start + self.length - 1
    }

    fn response(&self, size: u64, mime: &str, body: Body) -> Response<Body> {
        let mut response = Response::builder()
            .header(ACCEPT_RANGES, "bytes")
            .header(CACHE_CONTROL, "no-store")
            .header(CONTENT_LENGTH, self.length)
            .header(CONTENT_TYPE, mime);
        if self.partial {
            response = response.status(StatusCode::PARTIAL_CONTENT).header(
                CONTENT_RANGE,
                format!("bytes {}-{}/{size}", self.start, self.end()),
            );
        }
        response.body(body).unwrap()
    }
}

async fn serve_file(
    mut file: File,
    headers: &HeaderMap,
    method: Method,
    mime: &str,
    lease: Option<VaultPlaybackFile>,
) -> Response<Body> {
    let Ok(metadata) = file.metadata().await else {
        return empty_response(StatusCode::INTERNAL_SERVER_ERROR);
    };
    let size = metadata.len();
    let Some(range) = ResponseRange::from_headers(headers, size) else {
        return out_of_range(size);
    };
    if method == Method::HEAD {
        return range.response(size, mime, Body::empty());
    }
    if range.start > 0 && file.seek(SeekFrom::Start(range.start)).await.is_err() {
        return empty_response(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let body = ReaderStream::new(file.take(range.length)).map(move |chunk| {
        // Keep removed bytes accounted for until this Range response releases the file.
        let _ = &lease;
        chunk
    });
    range.response(size, mime, Body::from_stream(body))
}

async fn serve_archive(
    http: &ArchiveHttp,
    media: &ArchiveMedia,
    headers: &HeaderMap,
    method: Method,
) -> Response<Body> {
    let size = media.size;
    let Some(range) = ResponseRange::from_headers(headers, size) else {
        return out_of_range(size);
    };
    if method == Method::HEAD {
        return range.response(size, MP3_MIME, Body::empty());
    }
    let response = http
        .get(media, range.partial.then(|| (range.start, range.end())))
        .await;
    let Ok(response) = response else {
        return empty_response(StatusCode::BAD_GATEWAY);
    };
    let body = futures_util::stream::try_unfold(
        (response, range.length),
        |(mut response, remaining)| async move {
            if remaining == 0 {
                return Ok(None);
            }
            let chunk = response
                .chunk()
                .await
                .map_err(io::Error::other)?
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::UnexpectedEof, "Incomplete archive response")
                })?;
            let left = remaining.checked_sub(chunk.len() as u64).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "Oversized archive response")
            })?;
            Ok::<_, io::Error>(Some((chunk, (response, left))))
        },
    );
    range.response(size, MP3_MIME, Body::from_stream(body))
}

fn empty_response(status: StatusCode) -> Response<Body> {
    Response::builder()
        .status(status)
        .body(Body::empty())
        .unwrap()
}

fn out_of_range(size: u64) -> Response<Body> {
    Response::builder()
        .status(StatusCode::RANGE_NOT_SATISFIABLE)
        .header(ACCEPT_RANGES, "bytes")
        .header(CONTENT_RANGE, format!("bytes */{size}"))
        .body(Body::empty())
        .unwrap()
}

fn byte_range(value: &str, size: u64) -> Option<(u64, u64)> {
    let (first, last) = value.strip_prefix("bytes=")?.split_once('-')?;
    if size == 0 {
        return None;
    }
    if first.is_empty() {
        let length = last.parse::<u64>().ok()?.min(size);
        return (length > 0).then(|| (size - length, length));
    }
    let start: u64 = first.parse().ok()?;
    let end = if last.is_empty() {
        size - 1
    } else {
        last.parse::<u64>().ok()?.min(size - 1)
    };
    (start <= end).then(|| (start, end - start + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_handles_avplayer_probe_and_suffix() {
        assert_eq!(byte_range("bytes=0-1", 10), Some((0, 2)));
        assert_eq!(byte_range("bytes=4-", 10), Some((4, 6)));
        assert_eq!(byte_range("bytes=-3", 10), Some((7, 3)));
        assert_eq!(byte_range("bytes=7-20", 10), Some((7, 3)));
        assert_eq!(byte_range("bytes=11-", 10), None);
        assert_eq!(byte_range("bytes=-0", 10), None);
        assert_eq!(byte_range("bytes=0-1,3-4", 10), None);
    }

    #[tokio::test]
    async fn removed_archive_copy_remains_seekable_until_playback_finishes() {
        let scratch = tempfile::tempdir().unwrap();
        let vault = Arc::new(Vault::open(scratch.path().join("vault")).unwrap());
        let cached = scratch.path().join("vault/ia-offline");
        std::fs::create_dir(&cached).unwrap();
        std::fs::write(cached.join("audio.mp3"), b"abcdefghij").unwrap();
        let pipe = MediaPipe::start(vec![], Arc::clone(&vault)).await.unwrap();
        let media = ArchiveMedia::new(
            "not-an-archive-item",
            "song.mp3",
            10,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap();
        let source = pipe.archive_playback("ia-offline", media).unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let response = client
            .get(source.url())
            .header(RANGE, "bytes=2-4")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.headers()[CONTENT_RANGE], "bytes 2-4/10");
        assert_eq!(response.headers()[CONTENT_TYPE], MP3_MIME);
        assert_eq!(response.bytes().await.unwrap().as_ref(), b"cde");

        let storage = Arc::clone(&vault);
        tokio::task::spawn_blocking(move || storage.set_limit(10).unwrap())
            .await
            .unwrap();
        assert!(pipe.remove_saved("ia-offline".into()).await.unwrap());
        let storage = Arc::clone(&vault);
        tokio::task::spawn_blocking(move || {
            assert!(storage.path("ia-offline", "audio").is_none());
            assert!(storage.items().unwrap().is_empty());
            assert_eq!(storage.stats().unwrap(), (10, 10, 0));
            assert!(matches!(
                storage.set_limit(9),
                Err(VaultError::LimitBelowUsage)
            ));
        })
        .await
        .unwrap();

        for (requested, expected) in [("bytes=7-9", b"hij"), ("bytes=0-2", b"abc")] {
            let response = client
                .get(source.url())
                .header(RANGE, requested)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
            assert_eq!(response.bytes().await.unwrap().as_ref(), expected);
        }

        source.finish();
        let directory = scratch.path().join("vault");
        tokio::task::spawn_blocking(move || {
            assert_eq!(vault.stats().unwrap(), (0, 10, 0));
            assert!(std::fs::read_dir(directory).unwrap().all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".removed-")
            }));
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn prepared_copy_and_inflight_range_keep_the_removed_generation_separate() {
        let scratch = tempfile::tempdir().unwrap();
        let bundled = scratch.path().join("bundled.m4a");
        let old = scratch.path().join("old.m4a");
        let updated = scratch.path().join("updated.m4a");
        std::fs::write(&bundled, b"bundle").unwrap();
        std::fs::write(&old, b"old").unwrap();
        std::fs::write(&updated, b"newer").unwrap();
        let vault = Arc::new(Vault::open(scratch.path().join("vault")).unwrap());
        let setup = Arc::clone(&vault);
        tokio::task::spawn_blocking(move || setup.download("track", &old, None).unwrap())
            .await
            .unwrap();
        let pipe = MediaPipe::start(
            vec![MediaSource {
                id: "track".into(),
                audio_path: bundled,
                video_path: None,
            }],
            Arc::clone(&vault),
        )
        .await
        .unwrap();
        let first = pipe.bundled_playback("track", "audio").unwrap();
        let candidate = pipe.bundled_playback("track", "audio").unwrap();
        let lease = first.entry.vault_file(&vault).unwrap();
        let file = File::from_std(lease.open().unwrap());
        let response = serve_file(
            file,
            &HeaderMap::new(),
            Method::GET,
            "audio/mp4",
            Some(lease),
        )
        .await;

        let storage = Arc::clone(&vault);
        tokio::task::spawn_blocking(move || {
            assert!(storage.remove("track").unwrap());
            storage.download("track", &updated, None).unwrap();
        })
        .await
        .unwrap();
        let updated = pipe.bundled_playback("track", "audio").unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        for (source, expected) in [
            (&candidate, b"old".as_slice()),
            (&updated, b"newer".as_slice()),
        ] {
            let response = client.get(source.url()).send().await.unwrap();
            assert_eq!(response.bytes().await.unwrap().as_ref(), expected);
        }

        let first_url = first.url().to_owned();
        drop(first);
        drop(candidate);
        let storage = Arc::clone(&vault);
        tokio::task::spawn_blocking(move || assert_eq!(storage.stats().unwrap().0, 8))
            .await
            .unwrap();
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 3)
                .await
                .unwrap()
                .as_ref(),
            b"old"
        );
        assert_eq!(
            client.get(first_url).send().await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        tokio::task::spawn_blocking(move || assert_eq!(vault.stats().unwrap().0, 5))
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn removal_preserves_a_copy_saved_after_playback_started() {
        let scratch = tempfile::tempdir().unwrap();
        let vault = Arc::new(Vault::open(scratch.path().join("vault")).unwrap());
        let pipe = MediaPipe::start(vec![], Arc::clone(&vault)).await.unwrap();
        let media = ArchiveMedia::new("fixture", "song.mp3", 10, &"a".repeat(40)).unwrap();
        let source = pipe.archive_playback("ia-fixture", media).unwrap();
        assert!(source.entry.vault.lock().unwrap().file.is_none());
        let copy = scratch.path().join("vault/ia-fixture");
        std::fs::create_dir(&copy).unwrap();
        std::fs::write(copy.join("audio.mp3"), b"downloaded").unwrap();

        assert!(pipe.remove_saved("ia-fixture".into()).await.unwrap());
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(source.url())
            .header(RANGE, "bytes=1-4")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.bytes().await.unwrap().as_ref(), b"ownl");
        source.finish();
        let stats = tokio::task::spawn_blocking(move || vault.stats().unwrap())
            .await
            .unwrap();
        assert_eq!(stats.0, 0);
    }
}
