// Shared wire contract for the desktop host and browser preview.

export type ThemeVariant = "celestial-cyan" | "banshee-obsidian";
export type AtelierDisplayMode = "diamond-companion" | "wand-minimal" | "hidden";
export type CassetteStatusBadge = "song" | "mv" | "vault";
export type MediaMode = "audio" | "video";
export type PlaybackStatus = "idle" | "loading" | "ready" | "playing" | "paused" | "ended" | "error";

export interface ArtistRef {
  name: string;
  browseId?: string | null;
}

export interface AlbumRef {
  name: string;
  browseId?: string | null;
}

export interface TrackItem {
  videoId: string; // Stable media ID in the authorized catalog.
  mvVideoId?: string | null;
  mvTimelineAligned?: boolean;
  entryId?: string | null; // Identifies one occurrence within a playlist.
  title: string;
  artists: ArtistRef[];
  album?: AlbumRef | null;
  durationSec: number;
  thumbnailSquareUrl: string;
  thumbnailWideUrl?: string | null;
  isVaultDownloaded: boolean;
  isVaultVideoDownloaded?: boolean;
  loudnessDb?: number | null;
  trackNumber?: number | null;
  sourceName?: string | null;
  sourceUrl?: string | null;
  license?: string | null;
  licenseUrl?: string | null;
  audioUrl?: string | null;
  mvUrl?: string | null;
}

export interface PlaylistColumn {
  playlistId: string;
  columnIndex: number; // Physical column index, 0..8.
  codeName: string;
  title: string;
  subtitle?: string | null;
  isEditable: boolean;
  tracks: TrackItem[];
}

export interface WordTiming {
  word: string;
  startMs: number;
  endMs: number;
}

export interface LyricLine {
  startMs: number;
  endMs: number;
  text: string;
  translation?: string | null;
  words?: WordTiming[] | null;
  isBackgroundVocal?: boolean | null;
  duetSide?: "left" | "right" | null;
}

export interface PlaybackSpectrum {
  rms: number;
  subBass: number;
  midRange: number;
  highPresence: number;
  bars32: number[];
}

export interface PlaybackSnapshot {
  videoId: string | null;
  mode: MediaMode;
  status: PlaybackStatus;
  positionMs: number;
  durationMs: number;
  volume: number;
  error: string | null;
  spectrum: PlaybackSpectrum | null;
}

export interface PlaybackVideoFrame {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface PreparedPlayback {
  playback: PlaybackSnapshot;
  previous: PlaybackSnapshot;
}

export interface PlaybackOutput {
  id: string;
  name: string;
  kind: "builtin" | "headphones" | "display" | "speaker" | "other";
}

export interface PlaybackOutputs {
  supported: boolean;
  selectedDeviceId: string | null;
  defaultDeviceId: string | null;
  devices: PlaybackOutput[];
}

export interface PlaybackController {
  playback_load(args: { videoId: string; mode: MediaMode; positionMs?: number; autoplay?: boolean }): Promise<PlaybackSnapshot>;
  playback_get_state(): Promise<PlaybackSnapshot>;
  playback_set_playing(args: { playing: boolean }): Promise<PlaybackSnapshot>;
  playback_seek(args: { positionMs: number }): Promise<PlaybackSnapshot>;
  playback_set_volume(args: { volume: number }): Promise<PlaybackSnapshot>;
  playback_set_video_frame(args: { frame: PlaybackVideoFrame | null }): Promise<void>;
  playback_prepare(args: { switchId: number; videoId: string; mode: MediaMode }): Promise<void>;
  playback_commit(args: { switchId: number; alignTime: boolean; positionMs: number; autoplay?: boolean }): Promise<PreparedPlayback>;
  playback_cancel_preparation(args: { switchId: number }): Promise<void>;
}

export interface AuthorizedCatalog {
  tracks: TrackItem[];
  playlists: { id: string; title: string; subtitle?: string; videoIds: string[] }[];
  lyrics: Record<string, LyricLine[]>;
}

export interface SearchResults {
  songs: TrackItem[];
  videos: TrackItem[];
  playlists: PlaylistColumn[];
}

export interface ArchiveSearchResult {
  title: string;
  artist: string;
  license: string;
  licenseUrl: string;
  sourceUrl: string;
  trackCount: number;
  matchingTrack: { videoId: string; title: string; artist: string } | null;
}

export interface StorageStats {
  usedBytes: number;
  limitBytes: number;
  trackCount: number;
}

export interface VaultItem {
  track: TrackItem;
  bytes: number;
}

export interface PlaylistImportPreview {
  sourceId: string;
  title: string;
  tracks: { sourceIndex: number; track: TrackItem; imported: boolean }[];
}

export interface PlaylistRemoval {
  playlistId: string;
  entries: {
    entryId: string;
    videoId: string;
    sourceIndex: number | null;
    index: number;
    previousId: string | null;
    nextId: string | null;
  }[];
}

export interface NastiIpcContract extends PlaybackController {
  playback_list_outputs(): Promise<PlaybackOutputs>;
  playback_set_output(args: { deviceId: string | null }): Promise<PlaybackOutputs>;
  catalog_fetch_columns(): Promise<PlaylistColumn[]>;
  catalog_import_playlist_url(args: { urlOrId: string; targetColumn?: number }): Promise<PlaylistColumn>;
  catalog_preview_playlist(args: { urlOrId: string }): Promise<PlaylistImportPreview>;
  catalog_apply_playlist_import(args: { sourceId: string; sourceIndex?: number }): Promise<PlaylistImportPreview>;
  catalog_search(args: { query: string; filter: "all" | "songs" | "videos" | "playlists" }): Promise<SearchResults>;
  catalog_search_archive(args: { query: string }): Promise<ArchiveSearchResult[]>;
  playlist_add_track(args: { playlistId: string; videoId: string }): Promise<{ entryId: string }>;
  playlist_remove_track(args: { playlistId: string; entryId: string }): Promise<boolean>;
  playlist_remove_entries(args: { playlistId: string; entryIds: string[] }): Promise<PlaylistRemoval>;
  playlist_restore_entries(args: { removal: PlaylistRemoval }): Promise<void>;
  vault_download_track(args: { videoId: string; downloadMv: boolean }): Promise<TrackItem>;
  vault_remove_track(args: { videoId: string }): Promise<boolean>;
  vault_get_storage_stats(): Promise<StorageStats>;
  vault_list_tracks(): Promise<VaultItem[]>;
  vault_set_storage_limit(args: { limitBytes: number }): Promise<void>;
  lyrics_get_synced(args: { videoId: string }): Promise<LyricLine[]>;
}

export type YoutubePrivacy = "private" | "public" | "unlisted";

export interface YoutubeStatus {
  canConnect: boolean;
  connected: boolean;
}

export interface YoutubePlaylistSummary {
  id: string;
  title: string;
  description: string;
  privacy: YoutubePrivacy;
  canEdit: boolean;
}

export interface YoutubePlaylistItem {
  entryId: string;
  videoId: string;
  title: string;
  artist: string;
  position: number;
}

export interface YoutubePlaylist extends YoutubePlaylistSummary {
  items: YoutubePlaylistItem[];
  importedAt?: number | null;
}

export type YoutubeRequest =
  | { action: "status" }
  | { action: "configureClient"; clientJson: string }
  | { action: "connect" }
  | { action: "disconnect" }
  | { action: "mine" }
  | { action: "imported" }
  | { action: "import"; reference: string }
  | { action: "load"; playlistId: string }
  | { action: "forget"; playlistId: string }
  | { action: "create"; title: string; description: string; privacy: YoutubePrivacy }
  | { action: "update"; playlistId: string; title: string; description: string; privacy: YoutubePrivacy }
  | { action: "add"; playlistId: string; reference: string }
  | { action: "remove"; playlistId: string; entryId: string }
  | { action: "move"; playlistId: string; entryId: string; position: number }
  | { action: "delete"; playlistId: string }
  | { action: "open"; playlistId: string; videoId?: string };

export type YoutubeReply =
  | { kind: "status"; value: YoutubeStatus }
  | { kind: "lists"; value: YoutubePlaylistSummary[] }
  | { kind: "playlist"; value: YoutubePlaylist }
  | { kind: "done" };
