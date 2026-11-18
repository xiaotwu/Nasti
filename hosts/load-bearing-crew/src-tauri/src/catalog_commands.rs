use crate::models::{
    LyricLine, MediaMode, PlaylistColumn, SearchResults, StorageStats, TrackItem, VaultItem,
    YoutubePlaylistSummary, YoutubePrivacy,
};
use crate::runtime_storage::RuntimeStorage;
use convallis_pipe::{
    youtube_id, youtube_id_from_vault_key, youtube_vault_key, ArchiveMedia, MediaPipe, MediaSource,
    PlaybackSource, Vault, YoutubeSearchHit, YoutubeSearchResults,
};
use serde::Deserialize;
use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Mutex;
use uraniborg_core::{
    music_track, playlist_column, playlist_id, ArchiveRecording, Catalog, Library,
};

pub struct MediaServices {
    catalog: Arc<Catalog>,
    recent_search: Mutex<std::collections::VecDeque<TrackItem>>,
    pub(crate) library: Library,
    vault: Arc<Vault>,
    pipe: MediaPipe,
    _data_lock: File,
}

impl MediaServices {
    fn mark_track(&self, track: &mut TrackItem) {
        let key = self.youtube_key(&track.video_id);
        let key = key.as_deref().unwrap_or(&track.video_id);
        track.is_vault_downloaded = self.vault.path(key, "audio").is_some();
        track.is_vault_video_downloaded = self.vault.path(key, "video").is_some();
    }

    fn youtube_key(&self, id: &str) -> Option<String> {
        if self.catalog.track(id).is_ok() {
            return None;
        }
        youtube_vault_key(id)
    }

    async fn youtube_track(&self, id: &str, channel: Option<&str>) -> Result<TrackItem, String> {
        if let Some(track) = self
            .library
            .saved_youtube_track(id)
            .await
            .map_err(error_string)?
        {
            return Ok(track);
        }
        if let Some(track) = self
            .recent_search
            .lock()
            .await
            .iter()
            .find(|track| track.video_id == id)
            .cloned()
        {
            return Ok(track);
        }
        let track = self
            .library
            .youtube_columns(channel)
            .await
            .map_err(error_string)?
            .into_iter()
            .flat_map(|column| column.tracks)
            .find(|track| track.video_id == id);
        Ok(track.unwrap_or_else(|| music_track(id, id.into(), String::new(), false)))
    }

    fn mark_playlist(&self, playlist: &mut PlaylistColumn) {
        for track in &mut playlist.tracks {
            self.mark_track(track);
        }
    }
}

pub(crate) fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let packaged = app.path().resource_dir()?.join("authorized-media");
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../assets/authorized-media");
    let root = if packaged.join("catalog.json").is_file() {
        packaged
    } else {
        dev
    };
    let isolated_data = std::env::var_os("NASTI_DATA_DIR").filter(|path| !path.is_empty());
    let data = match isolated_data.as_ref() {
        Some(path) => PathBuf::from(path),
        _ => app.path().app_data_dir()?,
    };
    std::fs::create_dir_all(&data)?;
    let data_lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(data.join(".library-lock"))?;
    data_lock
        .try_lock()
        .map_err(|_| "曲库正在另一个 Nasti 窗口中使用")?;

    let runtime_base = if isolated_data.is_some() {
        data.clone()
    } else {
        app.path().home_dir()?
    };
    let runtime = RuntimeStorage::open(&runtime_base, &app.config().identifier)?;
    let catalog = Arc::new(Catalog::load(&root)?);
    let vault = Arc::new(Vault::open_with_staging(
        data.join("vault"),
        runtime.vault_dir.clone(),
    )?);
    let sources = catalog
        .tracks()
        .map(
            |track| -> Result<MediaSource, uraniborg_core::CatalogError> {
                let id = &track.video_id;
                Ok(MediaSource {
                    id: id.clone(),
                    audio_path: catalog.media_path(id, MediaMode::Audio)?,
                    video_path: track
                        .mv_url
                        .as_ref()
                        .map(|_| catalog.media_path(id, MediaMode::Video))
                        .transpose()?,
                })
            },
        )
        .collect::<Result<_, _>>()?;
    let (library, pipe) = tauri::async_runtime::block_on(async {
        let library = Library::open(&data.join("library.sqlite3"), Arc::clone(&catalog)).await?;
        let pipe = MediaPipe::start(sources, Arc::clone(&vault)).await?;
        Ok::<_, Box<dyn std::error::Error>>((library, pipe))
    })?;
    let youtube = crate::youtube_commands::YoutubeServices::new(
        isolated_data.is_none(),
        &data.join("youtube-oauth-client.json"),
    );
    app.manage(crate::youtube_commands::YoutubeService(youtube));
    app.manage(runtime);
    app.manage(MediaServices {
        catalog,
        recent_search: Mutex::default(),
        library,
        vault,
        pipe,
        _data_lock: data_lock,
    });
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60 * 30)).await;
            let media = handle.state::<MediaServices>();
            if media
                .library
                .expire_youtube_snapshots()
                .await
                .unwrap_or(false)
            {
                let _ = handle.emit("youtube-snapshots-expired", ());
            }
        }
    });
    Ok(())
}

pub(crate) async fn resolve_playback(
    app: &AppHandle,
    id: &str,
    mode: &str,
) -> Result<PlaybackSource, String> {
    let state = app.state::<MediaServices>();
    if let Some(source) = state.pipe.bundled_playback(id, mode) {
        return Ok(source);
    }
    if mode == "audio" {
        if let Some(record) = state
            .library
            .archive_recording(id)
            .await
            .map_err(error_string)?
        {
            if let Some(source) = state.pipe.archive_playback(id, archive_media(&record)?) {
                return Ok(source);
            }
        }
    }
    if state.youtube_key(id).is_some() {
        return state
            .pipe
            .youtube_playback(id, mode)
            .await
            .map_err(error_string);
    }
    Err("曲库中没有这个媒体版本".into())
}

fn archive_media(record: &ArchiveRecording) -> Result<ArchiveMedia, String> {
    ArchiveMedia::new(&record.item_id, &record.filename, record.size, &record.sha1)
        .map_err(error_string)
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchFilter {
    All,
    Songs,
    Videos,
    Playlists,
}

impl SearchFilter {
    fn as_str(&self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Songs => "songs",
            Self::Videos => "videos",
            Self::Playlists => "playlists",
        }
    }
}

#[derive(Deserialize)]
pub struct SearchArgs {
    query: String,
    filter: SearchFilter,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackArgs {
    video_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadArgs {
    video_id: String,
    download_mv: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageLimitArgs {
    limit_bytes: u64,
}

#[tauri::command]
pub(crate) async fn catalog_fetch_columns(
    state: State<'_, MediaServices>,
    youtube: State<'_, crate::youtube_commands::YoutubeService>,
) -> Result<Vec<PlaylistColumn>, String> {
    let account = youtube
        .0
        .as_ref()
        .ok()
        .and_then(|service| service.status().channel_id);
    let mut columns = state
        .library
        .youtube_columns(account.as_deref())
        .await
        .map_err(error_string)?;
    for playlist in &mut columns {
        state.mark_playlist(playlist);
    }
    Ok(columns)
}

#[tauri::command]
pub(crate) async fn catalog_search(
    args: SearchArgs,
    state: State<'_, MediaServices>,
    youtube: State<'_, crate::youtube_commands::YoutubeService>,
) -> Result<SearchResults, String> {
    let query = args.query.trim();
    let filter = args.filter.as_str();
    if query.is_empty() {
        return Ok(SearchResults {
            songs: vec![],
            videos: vec![],
            playlists: vec![],
        });
    }
    let mut results = match state.pipe.youtube_search(query, filter).await {
        Ok(results) => public_search_results(results),
        Err(error) => match &youtube.0 {
            Ok(service) if service.status().connected => service.search(query, filter).await?,
            _ => return Err(error_string(error)),
        },
    };
    for track in results.songs.iter_mut().chain(&mut results.videos) {
        state.mark_track(track);
    }
    {
        let mut recent = state.recent_search.lock().await;
        for track in results.songs.iter().chain(&results.videos) {
            recent.retain(|prior| prior.video_id != track.video_id);
            recent.push_front(track.clone());
        }
        recent.truncate(256);
    }
    for playlist in &mut results.playlists {
        state.mark_playlist(playlist);
    }
    Ok(results)
}

fn public_search_results(results: YoutubeSearchResults) -> SearchResults {
    let track = |hit: YoutubeSearchHit, video| {
        let title = hit.title.filter(|title| !title.is_empty())?;
        if !youtube_id(&hit.id) {
            return None;
        }
        let mut track = music_track(&hit.id, title, hit.channel.unwrap_or_default(), video);
        track.duration_sec = hit.duration.unwrap_or(0.0).max(0.0).round() as u32;
        Some(track)
    };
    let mut ids = std::collections::HashSet::new();
    SearchResults {
        songs: results
            .songs
            .into_iter()
            .filter_map(|hit| track(hit, false))
            .collect(),
        videos: results
            .videos
            .into_iter()
            .filter_map(|hit| track(hit, true))
            .collect(),
        playlists: results
            .playlists
            .into_iter()
            .filter_map(|hit| {
                let id = playlist_id(hit.url.as_deref()?).ok()?;
                let title = hit.title.filter(|title| !title.is_empty())?;
                if !ids.insert(id.clone()) {
                    return None;
                }
                Some(playlist_column(
                    YoutubePlaylistSummary {
                        id,
                        title,
                        description: hit.channel.unwrap_or_default(),
                        privacy: YoutubePrivacy::Public,
                        can_edit: false,
                    },
                    0,
                ))
            })
            .collect(),
    }
}

#[tauri::command]
pub(crate) async fn vault_download_track(
    args: DownloadArgs,
    state: State<'_, MediaServices>,
    youtube: State<'_, crate::youtube_commands::YoutubeService>,
) -> Result<TrackItem, String> {
    if let Some(record) = state
        .library
        .archive_recording(&args.video_id)
        .await
        .map_err(error_string)?
    {
        if args.download_mv {
            return Err("这首授权曲目仅提供音频".into());
        }
        let media = archive_media(&record)?;
        state
            .pipe
            .save_archive(args.video_id, media)
            .await
            .map_err(error_string)?;
        let mut track = record.track;
        state.mark_track(&mut track);
        return Ok(track);
    }
    if state.youtube_key(&args.video_id).is_some() {
        let channel = youtube
            .0
            .as_ref()
            .ok()
            .and_then(|service| service.status().channel_id);
        let mut track = state
            .youtube_track(&args.video_id, channel.as_deref())
            .await?;
        state
            .pipe
            .save_youtube(args.video_id, args.download_mv)
            .await
            .map_err(error_string)?;
        if args.download_mv {
            track.mv_video_id = Some(track.video_id.clone());
        }
        state
            .library
            .save_youtube_track(&track)
            .await
            .map_err(error_string)?;
        state.mark_track(&mut track);
        return Ok(track);
    }
    let audio = state
        .catalog
        .media_path(&args.video_id, MediaMode::Audio)
        .map_err(error_string)?;
    let video = args
        .download_mv
        .then(|| state.catalog.media_path(&args.video_id, MediaMode::Video))
        .transpose()
        .map_err(error_string)?;
    let mut track = state
        .catalog
        .track(&args.video_id)
        .map_err(error_string)?
        .clone();
    let vault = Arc::clone(&state.vault);
    let id = args.video_id;
    tauri::async_runtime::spawn_blocking(move || vault.download(&id, &audio, video.as_deref()))
        .await
        .map_err(error_string)?
        .map_err(error_string)?;
    state.mark_track(&mut track);
    Ok(track)
}

#[tauri::command]
pub(crate) async fn vault_remove_track(
    args: TrackArgs,
    state: State<'_, MediaServices>,
) -> Result<bool, String> {
    let youtube_key = state.youtube_key(&args.video_id);
    let removed = state
        .pipe
        .remove_saved(youtube_key.clone().unwrap_or_else(|| args.video_id.clone()))
        .await
        .map_err(error_string)?;
    if youtube_key.is_some() {
        state
            .library
            .forget_youtube_track(&args.video_id)
            .await
            .map_err(error_string)?;
    }
    Ok(removed)
}

#[tauri::command]
pub(crate) async fn vault_get_storage_stats(
    state: State<'_, MediaServices>,
) -> Result<StorageStats, String> {
    let vault = Arc::clone(&state.vault);
    let (used_bytes, limit_bytes, track_count) =
        tauri::async_runtime::spawn_blocking(move || vault.stats())
            .await
            .map_err(error_string)?
            .map_err(error_string)?;
    Ok(StorageStats {
        used_bytes,
        limit_bytes,
        track_count,
    })
}

#[tauri::command]
pub(crate) async fn vault_list_tracks(
    state: State<'_, MediaServices>,
) -> Result<Vec<VaultItem>, String> {
    let vault = Arc::clone(&state.vault);
    let items = tauri::async_runtime::spawn_blocking(move || vault.items())
        .await
        .map_err(error_string)?
        .map_err(error_string)?;
    let mut rows = Vec::with_capacity(items.len());
    for (id, bytes) in items {
        let mut track = if let Some(video_id) = youtube_id_from_vault_key(&id) {
            state.youtube_track(&video_id, None).await?
        } else {
            state.library.track(&id).await.map_err(error_string)?
        };
        state.mark_track(&mut track);
        rows.push(VaultItem { track, bytes });
    }
    Ok(rows)
}

#[tauri::command]
pub(crate) async fn vault_set_storage_limit(
    args: StorageLimitArgs,
    state: State<'_, MediaServices>,
) -> Result<(), String> {
    let vault = Arc::clone(&state.vault);
    tauri::async_runtime::spawn_blocking(move || vault.set_limit(args.limit_bytes))
        .await
        .map_err(error_string)?
        .map_err(error_string)
}

#[tauri::command]
pub(crate) async fn lyrics_get_synced(
    args: TrackArgs,
    state: State<'_, MediaServices>,
) -> Result<Vec<LyricLine>, String> {
    match state.catalog.lyrics(&args.video_id) {
        Ok(lines) => Ok(lines),
        Err(error) => {
            if state
                .library
                .archive_recording(&args.video_id)
                .await
                .map_err(error_string)?
                .is_some()
            {
                Ok(Vec::new())
            } else {
                Err(error_string(error))
            }
        }
    }
}

fn error_string(error: impl std::fmt::Display) -> String {
    error.to_string()
}
