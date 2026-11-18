//! Shared domain models. Keep these aligned with ipc-commands.ts.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArtistRef {
    pub name: String,
    pub browse_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AlbumRef {
    pub name: String,
    pub browse_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackItem {
    pub video_id: String,
    pub mv_video_id: Option<String>,
    #[serde(default)]
    pub mv_timeline_aligned: bool,
    pub entry_id: Option<String>,
    pub title: String,
    pub artists: Vec<ArtistRef>,
    pub album: Option<AlbumRef>,
    pub duration_sec: u32,
    pub thumbnail_square_url: String,
    pub thumbnail_wide_url: Option<String>,
    #[serde(default)]
    pub is_vault_downloaded: bool,
    #[serde(default)]
    pub is_vault_video_downloaded: bool,
    pub loudness_db: Option<f32>,
    pub track_number: Option<u32>,
    pub source_name: Option<String>,
    pub source_url: Option<String>,
    pub license: Option<String>,
    pub license_url: Option<String>,
    pub audio_url: Option<String>,
    pub mv_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistColumn {
    pub playlist_id: String,
    pub column_index: usize,
    pub code_name: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub is_editable: bool,
    pub tracks: Vec<TrackItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WordTiming {
    pub word: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricLine {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub translation: Option<String>,
    pub words: Option<Vec<WordTiming>>,
    pub is_background_vocal: Option<bool>,
    pub duet_side: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaMode {
    Audio,
    Video,
}

impl MediaMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackStatus {
    Idle,
    Loading,
    Ready,
    Playing,
    Paused,
    Ended,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackSpectrum {
    pub rms: f32,
    pub sub_bass: f32,
    pub mid_range: f32,
    pub high_presence: f32,
    pub bars32: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackSnapshot {
    pub video_id: Option<String>,
    pub mode: MediaMode,
    pub status: PlaybackStatus,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub volume: f32,
    pub error: Option<String>,
    pub spectrum: Option<PlaybackSpectrum>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackLoadArgs {
    pub video_id: String,
    pub mode: MediaMode,
    pub position_ms: Option<u64>,
    pub autoplay: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackPrepareArgs {
    pub switch_id: u64,
    pub video_id: String,
    pub mode: MediaMode,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackCommitArgs {
    pub switch_id: u64,
    pub align_time: bool,
    pub position_ms: u64,
    pub autoplay: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackCancelArgs {
    pub switch_id: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedPlayback {
    pub playback: PlaybackSnapshot,
    pub previous: PlaybackSnapshot,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackPlayingArgs {
    pub playing: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackSeekArgs {
    pub position_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackVolumeArgs {
    pub volume: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackOutputArgs {
    pub device_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackOutputKind {
    Builtin,
    Headphones,
    Display,
    Speaker,
    Other,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackOutput {
    pub id: String,
    pub name: String,
    pub kind: PlaybackOutputKind,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackOutputs {
    pub supported: bool,
    pub selected_device_id: Option<String>,
    pub default_device_id: Option<String>,
    pub devices: Vec<PlaybackOutput>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct PlaybackVideoFrame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Deserialize)]
pub struct PlaybackVideoFrameArgs {
    pub frame: Option<PlaybackVideoFrame>,
}

#[derive(Debug, Serialize)]
pub struct SearchResults {
    pub songs: Vec<TrackItem>,
    pub videos: Vec<TrackItem>,
    pub playlists: Vec<PlaylistColumn>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveSearchResult {
    pub title: String,
    pub artist: String,
    pub license: &'static str,
    pub license_url: &'static str,
    pub source_url: String,
    pub track_count: usize,
    pub matching_track: Option<ArchiveTrackMatch>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveTrackMatch {
    pub video_id: String,
    pub title: String,
    pub artist: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageStats {
    pub used_bytes: u64,
    pub limit_bytes: u64,
    pub track_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultItem {
    pub track: TrackItem,
    pub bytes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistImportEntry {
    pub source_index: usize,
    pub track: TrackItem,
    pub imported: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistImportPreview {
    pub source_id: String,
    pub title: String,
    pub tracks: Vec<PlaylistImportEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovedEntry {
    pub entry_id: String,
    pub video_id: String,
    pub source_index: Option<usize>,
    pub index: usize,
    pub previous_id: Option<String>,
    pub next_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistRemoval {
    pub playlist_id: String,
    pub entries: Vec<RemovedEntry>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum YoutubePrivacy {
    Private,
    Public,
    Unlisted,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateStatus {
    pub current_version: String,
    pub available_version: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AppUpdateInstallArgs {
    pub version: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YoutubeStatus {
    pub can_connect: bool,
    pub connected: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YoutubePlaylistSummary {
    pub id: String,
    pub title: String,
    pub description: String,
    pub privacy: YoutubePrivacy,
    pub can_edit: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YoutubePlaylistItem {
    pub entry_id: String,
    pub video_id: String,
    pub title: String,
    pub artist: String,
    pub position: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YoutubePlaylist {
    #[serde(flatten)]
    pub summary: YoutubePlaylistSummary,
    pub items: Vec<YoutubePlaylistItem>,
    pub imported_at: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum YoutubePolicyPage {
    Privacy,
    NastiTerms,
    YoutubeTerms,
}

#[derive(Deserialize)]
#[serde(
    tag = "action",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum YoutubeRequest {
    Status,
    ConfigureClient {
        client_json: String,
    },
    Connect,
    CancelConnect,
    OpenPolicy {
        page: YoutubePolicyPage,
    },
    Disconnect,
    Mine,
    Imported,
    Import {
        reference: String,
    },
    Load {
        playlist_id: String,
    },
    Forget {
        playlist_id: String,
    },
    Create {
        title: String,
        description: String,
        privacy: YoutubePrivacy,
    },
    Update {
        playlist_id: String,
        title: String,
        description: String,
        privacy: YoutubePrivacy,
    },
    Add {
        playlist_id: String,
        reference: String,
    },
    Remove {
        playlist_id: String,
        entry_id: String,
    },
    Move {
        playlist_id: String,
        entry_id: String,
        position: usize,
    },
    Delete {
        playlist_id: String,
    },
    Open {
        playlist_id: String,
        video_id: Option<String>,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum YoutubeReply {
    Status(YoutubeStatus),
    Lists(Vec<YoutubePlaylistSummary>),
    Playlist(YoutubePlaylist),
    Done,
}
