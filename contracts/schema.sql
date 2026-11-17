-- Local catalog membership. Remote recordings keep their verified provenance for offline use.
CREATE TABLE IF NOT EXISTS library_bootstrap (
    id INTEGER PRIMARY KEY CHECK (id = 1)
);

CREATE TABLE IF NOT EXISTS library_playlists (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL,
    column_index INTEGER NOT NULL UNIQUE CHECK (column_index BETWEEN 0 AND 8),
    title TEXT NOT NULL,
    subtitle TEXT
);

CREATE TABLE IF NOT EXISTS library_entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    playlist_id TEXT NOT NULL REFERENCES library_playlists (id) ON DELETE CASCADE,
    media_id TEXT NOT NULL,
    position INTEGER NOT NULL CHECK (position >= 0),
    source_index INTEGER CHECK (source_index >= 0),
    UNIQUE (playlist_id, position)
);

CREATE TABLE IF NOT EXISTS library_archive_tracks (
    media_id TEXT PRIMARY KEY,
    record_json TEXT NOT NULL
);

-- Imported YouTube playlist snapshots. IDs and positions come from playlistItems.
CREATE TABLE IF NOT EXISTS library_youtube_playlists (
    id TEXT PRIMARY KEY,
    account_id TEXT,
    imported_at INTEGER NOT NULL,
    playlist_json TEXT NOT NULL
);
