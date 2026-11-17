use crate::models::{
    ArchiveSearchResult, LyricLine, MediaMode, PlaylistColumn, PlaylistImportPreview,
    PlaylistRemoval, SearchResults, StorageStats, TrackItem, VaultItem,
};
use crate::runtime_storage::RuntimeStorage;
use convallis_pipe::{ArchiveMedia, MediaPipe, MediaSource, PlaybackSource, Vault};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex;
use uraniborg_core::{ArchiveAlbum, ArchiveCatalog, ArchiveRecording, Catalog, Library};

pub struct MediaServices {
    catalog: Arc<Catalog>,
    archive: ArchiveCatalog,
    previews: Mutex<std::collections::VecDeque<(String, Arc<ArchiveAlbum>)>>,
    pub(crate) library: Library,
    vault: Arc<Vault>,
    pipe: MediaPipe,
    _data_lock: File,
}

impl MediaServices {
    fn mark_track(&self, track: &mut TrackItem) {
        track.is_vault_downloaded = self.vault.path(&track.video_id, "audio").is_some();
        track.is_vault_video_downloaded = self.vault.path(&track.video_id, "video").is_some();
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
    let archive = ArchiveCatalog::new()?;
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
    app.manage(crate::youtube_commands::YoutubeService(
        crate::youtube_commands::YoutubeServices::new(
            isolated_data.is_none(),
            &data.join("youtube-oauth-client.json"),
            &runtime.oauth_dir,
        ),
    ));
    app.manage(runtime);
    app.manage(MediaServices {
        catalog,
        archive,
        previews: Mutex::default(),
        library,
        vault,
        pipe,
        _data_lock: data_lock,
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
    Err("曲库中没有这个媒体版本".into())
}

fn archive_media(record: &ArchiveRecording) -> Result<ArchiveMedia, String> {
    ArchiveMedia::new(&record.item_id, &record.filename, record.size, &record.sha1)
        .map_err(error_string)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportArgs {
    url_or_id: String,
    target_column: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyImportArgs {
    source_id: String,
    source_index: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveEntriesArgs {
    playlist_id: String,
    entry_ids: Vec<String>,
}

#[derive(Deserialize)]
pub struct RestoreEntriesArgs {
    removal: PlaylistRemoval,
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
pub struct ArchiveSearchArgs {
    query: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddEntryArgs {
    playlist_id: String,
    video_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistEntry {
    entry_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveEntryArgs {
    playlist_id: String,
    entry_id: String,
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
) -> Result<Vec<PlaylistColumn>, String> {
    let mut columns = state.library.columns().await.map_err(error_string)?;
    for playlist in &mut columns {
        state.mark_playlist(playlist);
    }
    Ok(columns)
}

#[tauri::command]
pub(crate) async fn catalog_import_playlist_url(
    args: ImportArgs,
    state: State<'_, MediaServices>,
) -> Result<PlaylistColumn, String> {
    let mut playlist = if let Some(item) =
        ArchiveCatalog::item_reference(&args.url_or_id).map_err(error_string)?
    {
        match state
            .library
            .stored_archive_album(&item, args.target_column)
            .await
            .map_err(error_string)?
        {
            Some(playlist) => playlist,
            None => {
                let album = state.archive.album(&item).await.map_err(error_string)?;
                state
                    .library
                    .import_archive(&album, args.target_column)
                    .await
                    .map_err(error_string)?
            }
        }
    } else {
        state
            .library
            .import(&args.url_or_id, args.target_column)
            .await
            .map_err(error_string)?
    };
    state.mark_playlist(&mut playlist);
    Ok(playlist)
}

#[tauri::command]
pub(crate) async fn catalog_preview_playlist(
    args: ImportArgs,
    state: State<'_, MediaServices>,
) -> Result<PlaylistImportPreview, String> {
    let Some(item) = ArchiveCatalog::item_reference(&args.url_or_id).map_err(error_string)? else {
        return state
            .library
            .preview_builtin(&args.url_or_id)
            .await
            .map_err(error_string);
    };
    let album = Arc::new(state.archive.album(&item).await.map_err(error_string)?);
    let result = state
        .library
        .preview_archive(&album)
        .await
        .map_err(error_string)?;
    let mut previews = state.previews.lock().await;
    previews.retain(|(id, _)| id != &result.source_id);
    previews.push_front((result.source_id.clone(), album));
    previews.truncate(4);
    Ok(result)
}

#[tauri::command]
pub(crate) async fn catalog_apply_playlist_import(
    args: ApplyImportArgs,
    state: State<'_, MediaServices>,
) -> Result<PlaylistImportPreview, String> {
    if !args.source_id.starts_with("archive:") {
        return state
            .library
            .apply_builtin(&args.source_id, args.source_index)
            .await
            .map_err(error_string);
    }
    let album = state
        .previews
        .lock()
        .await
        .iter()
        .find(|(id, _)| id == &args.source_id)
        .map(|(_, album)| Arc::clone(album))
        .ok_or_else(|| "导入预览已过期，请重新查找歌单".to_owned())?;
    state
        .library
        .apply_archive(&album, args.source_index)
        .await
        .map_err(error_string)
}

#[tauri::command]
pub(crate) async fn catalog_search(
    args: SearchArgs,
    state: State<'_, MediaServices>,
) -> Result<SearchResults, String> {
    let mut results = state
        .library
        .search(&args.query, args.filter.as_str())
        .await
        .map_err(error_string)?;
    for track in results.songs.iter_mut().chain(&mut results.videos) {
        state.mark_track(track);
    }
    for playlist in &mut results.playlists {
        state.mark_playlist(playlist);
    }
    Ok(results)
}

#[tauri::command]
pub(crate) async fn catalog_search_archive(
    args: ArchiveSearchArgs,
    state: State<'_, MediaServices>,
) -> Result<Vec<ArchiveSearchResult>, String> {
    state
        .archive
        .search(&args.query)
        .await
        .map_err(error_string)
}

#[tauri::command]
pub(crate) async fn playlist_add_track(
    args: AddEntryArgs,
    state: State<'_, MediaServices>,
) -> Result<PlaylistEntry, String> {
    let entry_id = state
        .library
        .add_track(&args.playlist_id, &args.video_id)
        .await
        .map_err(error_string)?;
    Ok(PlaylistEntry { entry_id })
}

#[tauri::command]
pub(crate) async fn playlist_remove_track(
    args: RemoveEntryArgs,
    state: State<'_, MediaServices>,
) -> Result<bool, String> {
    state
        .library
        .remove_track(&args.playlist_id, &args.entry_id)
        .await
        .map_err(error_string)
}

#[tauri::command]
pub(crate) async fn playlist_remove_entries(
    args: RemoveEntriesArgs,
    state: State<'_, MediaServices>,
) -> Result<PlaylistRemoval, String> {
    state
        .library
        .remove_entries(args.playlist_id, &args.entry_ids)
        .await
        .map_err(error_string)
}

#[tauri::command]
pub(crate) async fn playlist_restore_entries(
    args: RestoreEntriesArgs,
    state: State<'_, MediaServices>,
) -> Result<(), String> {
    state
        .library
        .restore_entries(args.removal)
        .await
        .map_err(error_string)
}

#[tauri::command]
pub(crate) async fn vault_download_track(
    args: DownloadArgs,
    state: State<'_, MediaServices>,
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
    state
        .library
        .track(&args.video_id)
        .await
        .map_err(error_string)?;
    state
        .pipe
        .remove_saved(args.video_id)
        .await
        .map_err(error_string)
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
        let mut track = state.library.track(&id).await.map_err(error_string)?;
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
