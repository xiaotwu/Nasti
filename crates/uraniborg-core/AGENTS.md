# AGENTS.md — Agent-1: `uraniborg-core` (乌拉尼博格核心)

## Territory & Boundaries
- **Allowed Write Path**: `crates/uraniborg-core/**`
- **Read-Only Contract**: `../../contracts/ipc-models.rs`, `../../contracts/schema.sql`
- **Forbidden**: Avoid modifying without Orchestrator sync: `contracts/`, `crates/convallis-pipe/`, `packages/`, or `hosts/`.

## Core Responsibilities
1. **Unauthenticated InnerTube Client Pool (`src/innertube/`)**:
   - Impersonate official clients (`WEB_REMIX`, `IOS_MUSIC`, `ANDROID_VR`, `TVHTML5_SIMPLY_EMBEDDED_PLAYER`) following `SimoHypers/limusic`.
   - Support unauthenticated public/unlisted playlist import by URL or ID (with automatic continuation token pagination) and mixed search (`songs`, `videos`, `playlists`).
2. **"Songs, not MVs" Track Purifier (`src/matcher/`)**:
   - Following `kushagrasinghx/BitChord` v1.1+, resolve the pure Album Audio release (`video_id`) for every track while preserving its counterpart music video (`mv_video_id`) for instant Audio <-> MV toggling.
3. **Lazy Authentication & Two-Way Playlist Sync (`src/auth/`, `src/playlist_sync/`)**:
   - Compute `SAPISIDHASH` headers when cookies are present in the OS Keychain.
   - Implement `browse/edit_playlist` for `ACTION_ADD_VIDEO` (capturing returned `set_video_id`) and `ACTION_REMOVE_VIDEO` (sending both `video_id` and `set_video_id`).
   - Persist playlists and an offline optimistic mutation queue in SQLite (`contracts/schema.sql`).
4. **9-Source Word-by-Word Lyrics Engine (`src/lyrics/`)**:
   - Query `LRCLIB`, `LyricsPlus`, `Unison`, `YouTube Music Timed Lyrics`, and `Netease` (with translation) keyed by exact track duration (`±2s`), returning `Vec<LyricLine>` with per-word timings (`WordTiming`).

## Verification Command
```bash
cargo test -p uraniborg-core
```
