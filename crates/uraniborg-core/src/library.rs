use crate::archive::{ArchiveAlbum, ArchiveRecording};
use crate::catalog::{matches_track, Catalog, CatalogError, SourcePlaylist, COLUMN_COUNT};
use crate::models::{
    PlaylistColumn, PlaylistImportEntry, PlaylistImportPreview, PlaylistRemoval, RemovedEntry,
    SearchResults, TrackItem,
};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

#[path = "youtube_store.rs"]
mod youtube_store;

const SCHEMA: &str = include_str!("../../../contracts/schema.sql");

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error("曲库数据库操作失败：{0}")]
    Database(#[from] sqlx::Error),
    #[error("YouTube 歌单超过会话快照容量")]
    YoutubeSnapshotTooLarge,
    #[error("无法清理旧版 YouTube 曲库记录，数据库正忙")]
    YoutubeMigrationBusy,
    #[error("已保存的在线曲目信息无效：{0}")]
    Metadata(#[from] serde_json::Error),
    #[error("未知歌单。可导入内置歌单链接或 Internet Archive 专辑")]
    UnknownPlaylist,
    #[error("请选择第 1 至第 9 列")]
    InvalidColumn,
    #[error("该列已有歌单")]
    OccupiedColumn,
    #[error("九列均已使用")]
    Full,
    #[error("曲目入列记录已变化，请重新选择")]
    EntriesChanged,
}

pub struct Library {
    pool: SqlitePool,
    catalog: Arc<Catalog>,
    youtube_snapshots: Mutex<youtube_store::YoutubeSnapshots>,
}

impl Library {
    pub async fn open(path: &Path, catalog: Arc<Catalog>) -> Result<Self, LibraryError> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(3));
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        sqlx::raw_sql(SCHEMA).execute(&pool).await?;
        let library = Self {
            pool,
            catalog,
            youtube_snapshots: Mutex::default(),
        };
        library.scrub_legacy_youtube_snapshots().await?;
        library.migrate_source_entries().await?;
        library.seed().await?;
        Ok(library)
    }

    async fn migrate_source_entries(&self) -> Result<(), LibraryError> {
        let mut tx = self.pool.begin().await?;
        let columns = sqlx::query("PRAGMA table_info(library_entries)")
            .fetch_all(&mut *tx)
            .await?;
        if !columns
            .iter()
            .any(|row| row.get::<String, _>("name") == "source_index")
        {
            sqlx::query("ALTER TABLE library_entries ADD COLUMN source_index INTEGER CHECK (source_index >= 0)")
                .execute(&mut *tx).await?;
            for source in &self.catalog.playlists {
                for (index, id) in source.video_ids.iter().enumerate() {
                    sqlx::query(
                        "UPDATE library_entries SET source_index = position
                                 WHERE position = ? AND media_id = ? AND playlist_id IN
                                 (SELECT id FROM library_playlists WHERE source_id = ?)",
                    )
                    .bind(index as i64)
                    .bind(id)
                    .bind(&source.id)
                    .execute(&mut *tx)
                    .await?;
                }
            }
            sqlx::query(
                "UPDATE library_entries SET source_index = position WHERE playlist_id IN
                         (SELECT id FROM library_playlists WHERE source_id LIKE 'archive:%')",
            )
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS library_source_occurrence
                     ON library_entries(playlist_id, source_index) WHERE source_index IS NOT NULL",
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn seed(&self) -> Result<(), LibraryError> {
        let mut tx = self.pool.begin().await?;
        let inserted = sqlx::query("INSERT OR IGNORE INTO library_bootstrap (id) VALUES (1)")
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if inserted > 0 {
            // Leave room to import more catalog playlists.
            for (column, source) in self.catalog.playlists.iter().take(5).enumerate() {
                insert_playlist(&mut tx, source, column).await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn columns(&self) -> Result<Vec<PlaylistColumn>, LibraryError> {
        let rows = sqlx::query(
            "SELECT p.id, p.column_index, p.title, p.subtitle, e.id AS entry_id, e.media_id, a.record_json
             FROM library_playlists p LEFT JOIN library_entries e ON p.id = e.playlist_id
             LEFT JOIN library_archive_tracks a ON a.media_id = e.media_id
             ORDER BY p.column_index, e.position",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut columns: Vec<PlaylistColumn> = Vec::new();
        for row in rows {
            let id: String = row.try_get("id")?;
            let column = row.try_get::<i64, _>("column_index")? as usize;
            if columns
                .last()
                .is_none_or(|playlist| playlist.column_index != column)
            {
                columns.push(PlaylistColumn {
                    playlist_id: id,
                    column_index: column,
                    code_name: format!("RL-{:02}", column + 1),
                    title: row.try_get("title")?,
                    subtitle: row.try_get("subtitle")?,
                    is_editable: true,
                    tracks: Vec::new(),
                });
            }
            let entry: Option<i64> = row.try_get("entry_id")?;
            let media_id: Option<String> = row.try_get("media_id")?;
            if let (Some(entry), Some(media_id), Some(column)) =
                (entry, media_id, columns.last_mut())
            {
                let remote: Option<String> = row.try_get("record_json")?;
                let track = match (self.catalog.track(&media_id), remote) {
                    (Ok(local), _) => Some(local.clone()),
                    (_, Some(record)) => {
                        Some(serde_json::from_str::<ArchiveRecording>(&record)?.track)
                    }
                    _ => None,
                };
                if let Some(mut track) = track {
                    track.entry_id = Some(entry.to_string());
                    track.track_number = Some(column.tracks.len() as u32 + 1);
                    column.tracks.push(track);
                }
            }
        }
        Ok(columns)
    }

    pub async fn search(&self, query: &str, filter: &str) -> Result<SearchResults, LibraryError> {
        let mut results = self.catalog.search(query, filter);
        if filter == "videos" {
            return Ok(results);
        }
        let query = query.trim().to_lowercase();
        let mut seen: HashSet<_> = results
            .songs
            .iter()
            .map(|track| track.video_id.clone())
            .collect();
        for column in self.columns().await? {
            // The bundled catalog is already included above. Archive playlists and their
            // imported occurrences are searchable with no connection to the provider.
            if matches!(filter, "all" | "songs") {
                results.songs.extend(
                    column
                        .tracks
                        .iter()
                        .filter(|track| {
                            track.video_id.starts_with("ia-")
                                && matches_track(track, &query)
                                && seen.insert(track.video_id.clone())
                        })
                        .cloned(),
                );
            }
            if matches!(filter, "all" | "playlists")
                && column.playlist_id.starts_with("archive:")
                && (column.title.to_lowercase().contains(&query)
                    || column.playlist_id.to_lowercase().contains(&query))
            {
                results.playlists.push(column);
            }
        }
        Ok(results)
    }

    pub async fn import(
        &self,
        url_or_id: &str,
        target: Option<usize>,
    ) -> Result<PlaylistColumn, LibraryError> {
        let id = url_or_id.trim();
        let id = id.strip_prefix("nasti://playlist/").unwrap_or(id);
        let source = self
            .catalog
            .playlists
            .iter()
            .find(|playlist| playlist.id == id)
            .ok_or(LibraryError::UnknownPlaylist)?;
        let mut tx = self.pool.begin().await?;
        let column = available_column(&mut tx, target).await?;
        let id = insert_playlist(&mut tx, source, column).await?;
        tx.commit().await?;
        self.column(&id).await
    }

    fn builtin_source(&self, reference: &str) -> Result<&SourcePlaylist, LibraryError> {
        let reference = reference.trim();
        let id = reference
            .strip_prefix("nasti://playlist/")
            .unwrap_or(reference);
        self.catalog
            .playlists
            .iter()
            .find(|playlist| playlist.id == id)
            .ok_or(LibraryError::UnknownPlaylist)
    }

    pub async fn preview_builtin(
        &self,
        reference: &str,
    ) -> Result<PlaylistImportPreview, LibraryError> {
        self.preview(self.builtin_source(reference)?, &[]).await
    }

    pub async fn preview_archive(
        &self,
        album: &ArchiveAlbum,
    ) -> Result<PlaylistImportPreview, LibraryError> {
        self.preview(&album.source, &album.recordings).await
    }

    async fn preview(
        &self,
        source: &SourcePlaylist,
        recordings: &[ArchiveRecording],
    ) -> Result<PlaylistImportPreview, LibraryError> {
        let current = sqlx::query_as::<_, SourceEntry>(
            "SELECT e.id, e.source_index, e.media_id FROM library_entries e
             WHERE e.playlist_id = (SELECT id FROM library_playlists WHERE source_id = ? ORDER BY column_index LIMIT 1)
             AND e.source_index IS NOT NULL ORDER BY e.source_index",
        ).bind(&source.id).fetch_all(&self.pool).await?;
        let matched = match_source_entries(&source.video_ids, &current);
        let tracks = source
            .video_ids
            .iter()
            .enumerate()
            .map(|(source_index, id)| {
                let track = recordings
                    .iter()
                    .find(|record| &record.track.video_id == id)
                    .map(|record| Ok(record.track.clone()))
                    .unwrap_or_else(|| self.catalog.track(id).cloned())?;
                Ok(PlaylistImportEntry {
                    source_index,
                    track,
                    imported: matched[source_index].is_some(),
                })
            })
            .collect::<Result<_, CatalogError>>()?;
        Ok(PlaylistImportPreview {
            source_id: source.id.clone(),
            title: source.title.clone(),
            tracks,
        })
    }

    pub async fn apply_builtin(
        &self,
        reference: &str,
        index: Option<usize>,
    ) -> Result<PlaylistImportPreview, LibraryError> {
        let source = self.builtin_source(reference)?;
        self.apply_import(source, &[], index).await?;
        self.preview(source, &[]).await
    }

    pub async fn apply_archive(
        &self,
        album: &ArchiveAlbum,
        index: Option<usize>,
    ) -> Result<PlaylistImportPreview, LibraryError> {
        self.apply_import(&album.source, &album.recordings, index)
            .await?;
        self.preview_archive(album).await
    }

    async fn apply_import(
        &self,
        source: &SourcePlaylist,
        recordings: &[ArchiveRecording],
        index: Option<usize>,
    ) -> Result<(), LibraryError> {
        if source.video_ids.is_empty() || index.is_some_and(|i| i >= source.video_ids.len()) {
            return Err(LibraryError::EntriesChanged);
        }
        let mut tx = self.pool.begin().await?;
        let list_id = match existing_archive(&mut tx, &source.id, None).await? {
            Some(id) => id,
            None => {
                let column = available_column(&mut tx, None).await?;
                create_playlist(&mut tx, source, column).await?
            }
        };
        let current = sqlx::query_as::<_, SourceEntry>(
            "SELECT id, source_index, media_id FROM library_entries
             WHERE playlist_id = ? AND source_index IS NOT NULL ORDER BY source_index",
        )
        .bind(&list_id)
        .fetch_all(&mut *tx)
        .await?;
        let matched = match_source_entries(&source.video_ids, &current);
        if current.iter().any(|&(id, old_index, _)| {
            matched.get(old_index as usize).copied().flatten() != Some(id)
        }) {
            // Vacate the unique index before assigning new positions, including swaps.
            // Keep unmatched entries as local additions, since legacy source provenance can be ambiguous.
            sqlx::query("UPDATE library_entries SET source_index = NULL WHERE playlist_id = ? AND source_index IS NOT NULL")
                .bind(&list_id).execute(&mut *tx).await?;
            for (new_index, id) in matched
                .iter()
                .enumerate()
                .filter_map(|(i, id)| id.map(|id| (i, id)))
            {
                sqlx::query(
                    "UPDATE library_entries SET source_index = ? WHERE id = ? AND playlist_id = ?",
                )
                .bind(new_index as i64)
                .bind(id)
                .bind(&list_id)
                .execute(&mut *tx)
                .await?;
            }
        }
        for (i, media_id) in source.video_ids.iter().enumerate() {
            if index.is_some_and(|requested| requested != i) {
                continue;
            }
            if let Some(record) = recordings
                .iter()
                .find(|record| &record.track.video_id == media_id)
            {
                sqlx::query("INSERT INTO library_archive_tracks (media_id, record_json) VALUES (?, ?)
                             ON CONFLICT(media_id) DO UPDATE SET record_json = excluded.record_json")
                    .bind(media_id).bind(serde_json::to_string(record)?).execute(&mut *tx).await?;
            }
            if matched[i].is_none() {
                sqlx::query("INSERT INTO library_entries (playlist_id, media_id, position, source_index)
                             SELECT ?, ?, COALESCE(MAX(position) + 1, 0), ? FROM library_entries WHERE playlist_id = ?")
                    .bind(&list_id).bind(media_id).bind(i as i64).bind(&list_id).execute(&mut *tx).await?;
            }
        }
        if index.is_none() {
            let order = sqlx::query_scalar::<_, i64>(
                "SELECT id FROM library_entries WHERE playlist_id = ? ORDER BY source_index IS NULL, source_index, position",
            ).bind(&list_id).fetch_all(&mut *tx).await?;
            renumber(&mut tx, &list_id, &order).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn stored_archive_album(
        &self,
        item_id: &str,
        target: Option<usize>,
    ) -> Result<Option<PlaylistColumn>, LibraryError> {
        let mut tx = self.pool.begin().await?;
        let id = existing_archive(&mut tx, &format!("archive:{item_id}"), target).await?;
        tx.commit().await?;
        match id {
            Some(id) => self.column(&id).await.map(Some),
            None => Ok(None),
        }
    }

    pub async fn import_archive(
        &self,
        album: &ArchiveAlbum,
        target: Option<usize>,
    ) -> Result<PlaylistColumn, LibraryError> {
        let mut tx = self.pool.begin().await?;
        if let Some(id) = existing_archive(&mut tx, &album.source.id, target).await? {
            tx.commit().await?;
            return self.column(&id).await;
        }
        let column = available_column(&mut tx, target).await?;
        for record in &album.recordings {
            sqlx::query(
                "INSERT INTO library_archive_tracks (media_id, record_json) VALUES (?, ?)
                 ON CONFLICT(media_id) DO UPDATE SET record_json = excluded.record_json",
            )
            .bind(&record.track.video_id)
            .bind(serde_json::to_string(record)?)
            .execute(&mut *tx)
            .await?;
        }
        let id = insert_playlist(&mut tx, &album.source, column).await?;
        tx.commit().await?;
        self.column(&id).await
    }

    pub async fn archive_recording(
        &self,
        id: &str,
    ) -> Result<Option<ArchiveRecording>, LibraryError> {
        let record = sqlx::query_scalar::<_, String>(
            "SELECT record_json FROM library_archive_tracks WHERE media_id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(record.map(|json| serde_json::from_str(&json)).transpose()?)
    }

    pub async fn track(&self, id: &str) -> Result<TrackItem, LibraryError> {
        match self.catalog.track(id) {
            Ok(track) => Ok(track.clone()),
            Err(missing) => self
                .archive_recording(id)
                .await?
                .map(|record| record.track)
                .ok_or(missing.into()),
        }
    }

    async fn column(&self, id: &str) -> Result<PlaylistColumn, LibraryError> {
        self.columns()
            .await?
            .into_iter()
            .find(|playlist| playlist.playlist_id == id)
            .ok_or(LibraryError::UnknownPlaylist)
    }

    pub async fn add_track(
        &self,
        playlist_id: &str,
        video_id: &str,
    ) -> Result<String, LibraryError> {
        self.track(video_id).await?;
        let mut tx = self.pool.begin().await?;
        let exists =
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM library_playlists WHERE id = ?")
                .bind(playlist_id)
                .fetch_one(&mut *tx)
                .await?;
        if exists == 0 {
            return Err(LibraryError::UnknownPlaylist);
        }
        let row = sqlx::query(
            "INSERT INTO library_entries (playlist_id, media_id, position)
             SELECT ?, ?, COALESCE(MAX(position) + 1, 0) FROM library_entries WHERE playlist_id = ?",
        ).bind(playlist_id).bind(video_id).bind(playlist_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(row.last_insert_rowid().to_string())
    }

    pub async fn remove_track(
        &self,
        playlist_id: &str,
        entry_id: &str,
    ) -> Result<bool, LibraryError> {
        let Ok(entry_id) = entry_id.parse::<i64>() else {
            return Ok(false);
        };
        let result = sqlx::query("DELETE FROM library_entries WHERE playlist_id = ? AND id = ?")
            .bind(playlist_id)
            .bind(entry_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn remove_entries(
        &self,
        list_id: String,
        entry_ids: &[String],
    ) -> Result<PlaylistRemoval, LibraryError> {
        let requested = entry_ids
            .iter()
            .map(|id| id.parse::<i64>())
            .collect::<Result<HashSet<_>, _>>()
            .map_err(|_| LibraryError::EntriesChanged)?;
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query_as::<_, (i64, String, Option<i64>)>(
            "SELECT id, media_id, source_index FROM library_entries WHERE playlist_id = ? ORDER BY position",
        ).bind(&list_id).fetch_all(&mut *tx).await?;
        let entries: Vec<_> = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| requested.contains(&row.0))
            .map(|(index, (id, media, source_index))| RemovedEntry {
                entry_id: id.to_string(),
                video_id: media.clone(),
                source_index: source_index.map(|i| i as usize),
                index,
                previous_id: index.checked_sub(1).map(|i| rows[i].0.to_string()),
                next_id: rows.get(index + 1).map(|row| row.0.to_string()),
            })
            .collect();
        if requested.is_empty() || entries.len() != requested.len() {
            return Err(LibraryError::EntriesChanged);
        }
        for entry in &entries {
            sqlx::query("DELETE FROM library_entries WHERE playlist_id = ? AND id = ?")
                .bind(&list_id)
                .bind(&entry.entry_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(PlaylistRemoval {
            playlist_id: list_id,
            entries,
        })
    }

    pub async fn restore_entries(&self, mut removal: PlaylistRemoval) -> Result<(), LibraryError> {
        for entry in &removal.entries {
            self.track(&entry.video_id).await?;
        }
        let list_id = &removal.playlist_id;
        let mut tx = self.pool.begin().await?;
        let mut order = sqlx::query_scalar::<_, i64>(
            "SELECT id FROM library_entries WHERE playlist_id = ? ORDER BY position",
        )
        .bind(list_id)
        .fetch_all(&mut *tx)
        .await?;
        removal.entries.sort_by_key(|entry| entry.index);
        for entry in removal.entries {
            let id = entry
                .entry_id
                .parse::<i64>()
                .ok()
                .filter(|id| *id > 0)
                .ok_or(LibraryError::EntriesChanged)?;
            if order.contains(&id) {
                continue;
            }
            let source = entry
                .source_index
                .map(i64::try_from)
                .transpose()
                .map_err(|_| LibraryError::EntriesChanged)?;
            let existing = match source {
                Some(index) => sqlx::query_scalar::<_, String>("SELECT media_id FROM library_entries WHERE playlist_id = ? AND source_index = ?")
                    .bind(list_id).bind(index).fetch_optional(&mut *tx).await?,
                None => None,
            };
            if let Some(media_id) = existing {
                if media_id == entry.video_id {
                    continue;
                }
                return Err(LibraryError::EntriesChanged);
            }
            let find = |id: Option<String>| {
                id.and_then(|value| value.parse::<i64>().ok())
                    .and_then(|value| order.iter().position(|entry_id| *entry_id == value))
            };
            let index = find(entry.previous_id)
                .map(|i| i + 1)
                .or_else(|| find(entry.next_id))
                .unwrap_or_else(|| entry.index.min(order.len()));
            sqlx::query("INSERT INTO library_entries (id, playlist_id, media_id, position, source_index)
                         SELECT ?, ?, ?, COALESCE(MAX(position) + 1, 0), ? FROM library_entries WHERE playlist_id = ?")
                .bind(id).bind(list_id).bind(entry.video_id).bind(source).bind(list_id).execute(&mut *tx).await?;
            order.insert(index, id);
        }
        renumber(&mut tx, list_id, &order).await?;
        tx.commit().await?;
        Ok(())
    }
}

type SourceEntry = (i64, i64, String);

fn match_source_entries(source: &[String], current: &[SourceEntry]) -> Vec<Option<i64>> {
    let mut saved = HashMap::<&str, Vec<(usize, i64)>>::new();
    for (id, index, media) in current {
        saved.entry(media).or_default().push((*index as usize, *id));
    }
    let mut incoming = HashMap::<&str, Vec<usize>>::new();
    for (index, media) in source.iter().enumerate() {
        incoming.entry(media).or_default().push(index);
    }

    let mut matched = vec![None; source.len()];
    for (media, previous) in saved {
        let Some(indices) = incoming.get(media) else {
            continue;
        };
        // With the same count, keep repeated appearances in their previous relative order.
        if previous.len() == indices.len() {
            for ((_, id), &index) in previous.into_iter().zip(indices) {
                matched[index] = Some(id);
            }
            continue;
        }

        // A partial import may have saved only a later appearance. Prefer exact positions
        // before mapping any remaining entries to the available appearances of this media.
        let mut remaining = Vec::new();
        for (index, id) in previous {
            if source
                .get(index)
                .is_some_and(|candidate| candidate == media)
            {
                matched[index] = Some(id);
            } else {
                remaining.push(id);
            }
        }
        let mut remaining = remaining.into_iter();
        for &index in indices {
            if matched[index].is_none() {
                matched[index] = remaining.next();
            }
        }
    }
    matched
}

async fn renumber(
    tx: &mut Transaction<'_, Sqlite>,
    id: &str,
    order: &[i64],
) -> Result<(), sqlx::Error> {
    let base: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(position), 0) + 1 FROM library_entries WHERE playlist_id = ?",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query("UPDATE library_entries SET position = position + ? WHERE playlist_id = ?")
        .bind(base)
        .bind(id)
        .execute(&mut **tx)
        .await?;
    for (index, entry) in order.iter().enumerate() {
        sqlx::query("UPDATE library_entries SET position = ? WHERE id = ? AND playlist_id = ?")
            .bind(index as i64)
            .bind(entry)
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

async fn existing_archive(
    tx: &mut Transaction<'_, Sqlite>,
    source_id: &str,
    target: Option<usize>,
) -> Result<Option<String>, LibraryError> {
    if target.is_some_and(|column| column >= COLUMN_COUNT) {
        return Err(LibraryError::InvalidColumn);
    }
    let column = target.map(|index| index as i64);
    Ok(sqlx::query_scalar(
        "SELECT id FROM library_playlists WHERE source_id = ? AND (? IS NULL OR column_index = ?)
         ORDER BY column_index LIMIT 1",
    )
    .bind(source_id)
    .bind(column)
    .bind(column)
    .fetch_optional(&mut **tx)
    .await?)
}

async fn available_column(
    tx: &mut Transaction<'_, Sqlite>,
    target: Option<usize>,
) -> Result<usize, LibraryError> {
    let occupied = sqlx::query_scalar::<_, i64>("SELECT column_index FROM library_playlists")
        .fetch_all(&mut **tx)
        .await?;
    match target {
        Some(column) if column >= COLUMN_COUNT => Err(LibraryError::InvalidColumn),
        Some(column) if occupied.contains(&(column as i64)) => Err(LibraryError::OccupiedColumn),
        Some(column) => Ok(column),
        None => (0..COLUMN_COUNT)
            .find(|column| !occupied.contains(&(*column as i64)))
            .ok_or(LibraryError::Full),
    }
}

async fn insert_playlist(
    tx: &mut Transaction<'_, Sqlite>,
    source: &SourcePlaylist,
    column: usize,
) -> Result<String, sqlx::Error> {
    let id = create_playlist(tx, source, column).await?;
    for (position, track) in source.video_ids.iter().enumerate() {
        sqlx::query(
            "INSERT INTO library_entries (playlist_id, media_id, position, source_index) VALUES (?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(track)
        .bind(position as i64)
        .bind(position as i64)
        .execute(&mut **tx)
        .await?;
    }
    Ok(id)
}

async fn create_playlist(
    tx: &mut Transaction<'_, Sqlite>,
    source: &SourcePlaylist,
    column: usize,
) -> Result<String, sqlx::Error> {
    let id = format!("{}-{}", source.id, column + 1);
    sqlx::query("INSERT INTO library_playlists (id, source_id, column_index, title, subtitle) VALUES (?, ?, ?, ?, ?)")
        .bind(&id).bind(&source.id).bind(column as i64).bind(&source.title).bind(&source.subtitle)
        .execute(&mut **tx).await?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_catalog(root: &Path) -> Arc<Catalog> {
        let mut track = crate::archive::fixture_album().recordings.remove(0).track;
        track.video_id = "sample-track".into();
        track.title = "Synthetic test track".into();
        track.audio_url = Some("/authorized-media/test-data".into());
        track.thumbnail_square_url = "/authorized-media/test-data".into();
        track.mv_url = None;
        track.mv_video_id = None;
        std::fs::write(root.join("test-data"), b"fixture bytes").unwrap();
        let manifest = serde_json::json!({
            "tracks": [track],
            "playlists": [{"id": "orbital", "title": "Test playlist", "videoIds": ["sample-track"]}],
            "lyrics": {}
        });
        std::fs::write(root.join("catalog.json"), manifest.to_string()).unwrap();
        Arc::new(Catalog::load(root).unwrap())
    }

    #[tokio::test]
    async fn empty_release_library_can_import_its_first_archive_playlist() {
        let scratch = tempfile::tempdir().unwrap();
        let media = scratch.path().join("media");
        std::fs::create_dir(&media).unwrap();
        std::fs::write(
            media.join("catalog.json"),
            br#"{"tracks":[],"playlists":[],"lyrics":{}}"#,
        )
        .unwrap();
        let catalog = Arc::new(Catalog::load(&media).unwrap());
        let path = scratch.path().join("library.sqlite3");
        let library = Library::open(&path, Arc::clone(&catalog)).await.unwrap();
        assert!(library.columns().await.unwrap().is_empty());

        let album = crate::archive::fixture_album();
        let imported = library.apply_archive(&album, None).await.unwrap();
        assert!(!imported.tracks.is_empty());
        assert!(imported.tracks.iter().all(|entry| entry.imported));
        drop(library);

        let reopened = Library::open(&path, catalog).await.unwrap();
        let columns = reopened.columns().await.unwrap();
        assert_eq!(columns.len(), 1);
        assert_eq!(columns[0].tracks.len(), album.source.video_ids.len());
        assert!(columns[0]
            .tracks
            .iter()
            .all(|track| track.entry_id.is_some()));
    }

    #[tokio::test]
    async fn partial_source_import_and_atomic_undo_keep_repeated_occurrences() {
        let db = tempfile::tempdir().unwrap();
        let catalog = test_catalog(db.path());
        let library = Library::open(&db.path().join("library.sqlite3"), catalog)
            .await
            .unwrap();
        let mut album = crate::archive::fixture_album();
        let first = album.source.video_ids[0].clone();
        album.source.video_ids.push(first.clone());
        let last = album.source.video_ids.len() - 1;

        let preview = library.preview_archive(&album).await.unwrap();
        assert!(preview.tracks.iter().all(|item| !item.imported));
        let partial = library.apply_archive(&album, Some(last)).await.unwrap();
        assert_eq!(
            partial.tracks.iter().filter(|item| item.imported).count(),
            1
        );
        let prior = library.columns().await.unwrap().pop().unwrap();
        let prior_id = prior.tracks[0].entry_id.as_deref().unwrap();
        let preview = library.apply_archive(&album, None).await.unwrap();
        assert!(preview.tracks.iter().all(|item| item.imported));
        let full = library.columns().await.unwrap().pop().unwrap();
        let ordered = |column: &PlaylistColumn| {
            column
                .tracks
                .iter()
                .map(|track| (track.entry_id.clone(), track.video_id.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(full.tracks[last].entry_id.as_deref(), Some(prior_id));
        assert_eq!(full.tracks[0].video_id, first);
        assert_ne!(full.tracks[0].entry_id, full.tracks[last].entry_id);

        let removed = vec![
            full.tracks[0].entry_id.clone().unwrap(),
            full.tracks[last].entry_id.clone().unwrap(),
        ];
        let undo = library
            .remove_entries(full.playlist_id.clone(), &removed)
            .await
            .unwrap();
        assert_eq!(
            library
                .column(&full.playlist_id)
                .await
                .unwrap()
                .tracks
                .len(),
            full.tracks.len() - 2
        );
        library.restore_entries(undo).await.unwrap();
        assert_eq!(
            ordered(&full),
            ordered(&library.column(&full.playlist_id).await.unwrap())
        );
    }

    #[tokio::test]
    async fn source_reorder_reuses_repeat_ids_and_keeps_manual_entries() {
        let db = tempfile::tempdir().unwrap();
        let catalog = test_catalog(db.path());
        let path = db.path().join("library.sqlite3");
        let library = Library::open(&path, Arc::clone(&catalog)).await.unwrap();
        let mut album = crate::archive::fixture_album();
        let first = album.source.video_ids[0].clone();
        album.source.video_ids.insert(1, first.clone());
        library.apply_archive(&album, None).await.unwrap();
        let initial = library.columns().await.unwrap().pop().unwrap();
        let id = &initial.playlist_id;
        // The old database migration assigned source_index even to manual additions.
        let legacy_media = "sample-track";
        let legacy = library.add_track(id, legacy_media).await.unwrap();
        sqlx::query("UPDATE library_entries SET source_index = 3 WHERE id = ?")
            .bind(&legacy)
            .execute(&library.pool)
            .await
            .unwrap();
        let manual = library.add_track(id, &first).await.unwrap();
        let ids = |column: PlaylistColumn| {
            column
                .tracks
                .into_iter()
                .map(|track| track.entry_id.unwrap())
                .collect::<Vec<_>>()
        };
        let original = ids(initial.clone());
        let expected = vec![
            original[2].clone(),
            original[0].clone(),
            original[1].clone(),
            legacy.clone(),
            manual,
        ];

        album.source.video_ids.rotate_right(1);
        let preview = library.preview_archive(&album).await.unwrap();
        assert!(preview.tracks.iter().all(|item| item.imported));
        library.apply_archive(&album, None).await.unwrap();
        library.apply_archive(&album, None).await.unwrap();
        let reordered = library.column(id).await.unwrap();
        let track_order: Vec<_> = reordered
            .tracks
            .iter()
            .map(|track| track.video_id.clone())
            .collect();
        assert_eq!(
            track_order,
            [
                album.source.video_ids.clone(),
                vec![legacy_media.into(), first]
            ]
            .concat()
        );
        assert_eq!(ids(reordered), expected);
        let legacy_index: Option<i64> =
            sqlx::query_scalar("SELECT source_index FROM library_entries WHERE id = ?")
                .bind(&legacy)
                .fetch_one(&library.pool)
                .await
                .unwrap();
        assert_eq!(legacy_index, None);

        library.pool.close().await;
        let library = Library::open(&path, catalog).await.unwrap();
        assert_eq!(ids(library.column(id).await.unwrap()), expected);
    }

    #[tokio::test]
    async fn single_import_after_source_reorder_reuses_the_shifted_entry() {
        let db = tempfile::tempdir().unwrap();
        let library = Library::open(
            &db.path().join("library.sqlite3"),
            test_catalog(db.path()),
        )
        .await
        .unwrap();
        let mut album = crate::archive::fixture_album();
        library.apply_archive(&album, Some(0)).await.unwrap();
        let initial = library.columns().await.unwrap().pop().unwrap();
        let first_id = initial.tracks[0].entry_id.clone();
        album.source.video_ids.swap(0, 1);

        let preview = library.preview_archive(&album).await.unwrap();
        assert!(!preview.tracks[0].imported);
        assert!(preview.tracks[1].imported);
        library.apply_archive(&album, Some(0)).await.unwrap();
        library.apply_archive(&album, None).await.unwrap();
        let column = library.column(&initial.playlist_id).await.unwrap();
        assert_eq!(column.tracks.len(), 2);
        assert_eq!(column.tracks[0].video_id, album.source.video_ids[0]);
        assert_eq!(column.tracks[1].video_id, album.source.video_ids[1]);
        assert_eq!(column.tracks[1].entry_id, first_id);
    }

    #[tokio::test]
    async fn archive_import_adds_metadata_without_reseeding_an_existing_library() {
        let db = tempfile::tempdir().unwrap();
        let catalog = test_catalog(db.path());
        let path = db.path().join("library.sqlite3");
        let library = Library::open(&path, Arc::clone(&catalog)).await.unwrap();
        let initial = library.columns().await.unwrap();
        sqlx::query("DROP TABLE library_archive_tracks")
            .execute(&library.pool)
            .await
            .unwrap();
        library.pool.close().await;

        let library = Library::open(&path, Arc::clone(&catalog)).await.unwrap();
        let album = crate::archive::fixture_album();
        let (imported, duplicate) = tokio::join!(
            library.import_archive(&album, None),
            library.import_archive(&album, None),
        );
        let imported = imported.unwrap();
        assert_eq!(imported.playlist_id, duplicate.unwrap().playlist_id);
        assert_eq!(imported.column_index, initial.len());
        assert_eq!(imported.tracks[0].title, "Opening track");
        let song = &imported.tracks[0];
        library
            .add_track(&initial[0].playlist_id, &song.video_id)
            .await
            .unwrap();
        library.pool.close().await;

        let library = Library::open(&path, catalog).await.unwrap();
        let loaded = library
            .stored_archive_album("unfound-test", None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.tracks[0].entry_id, song.entry_id);
        assert_eq!(
            loaded.tracks[0].license_url.as_deref(),
            Some("https://creativecommons.org/licenses/by/4.0/")
        );
        let archived = library
            .archive_recording(&song.video_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(archived.filename, "01.mp3");
        assert_eq!(archived.size, 12);
        let columns = library.columns().await.unwrap();
        assert_eq!(columns.len(), initial.len() + 1);
        assert_eq!(columns[0].tracks[0].entry_id, initial[0].tracks[0].entry_id);
        assert_eq!(columns[0].tracks.last().unwrap().video_id, song.video_id);
        let search = library.search("opening track", "songs").await.unwrap();
        assert_eq!(search.songs.len(), 1);
        assert_eq!(search.songs[0].video_id, song.video_id);
    }

    #[tokio::test]
    async fn repeated_track_occurrences_survive_reopen_and_remove_independently() {
        let db = tempfile::tempdir().unwrap();
        let catalog = test_catalog(db.path());
        let path = db.path().join("library.sqlite3");
        let library = Library::open(&path, Arc::clone(&catalog)).await.unwrap();
        let first = library.columns().await.unwrap().remove(0);
        let id = &first.playlist_id;
        let track = &first.tracks[0];
        let entry = library.add_track(id, &track.video_id).await.unwrap();
        assert_ne!(track.entry_id.as_deref(), Some(entry.as_str()));
        let imported = library
            .import("nasti://playlist/orbital", Some(8))
            .await
            .unwrap();
        assert_eq!(imported.column_index, 8);
        assert!(matches!(
            library.import("orbital", Some(8)).await,
            Err(LibraryError::OccupiedColumn)
        ));
        assert!(matches!(
            library.import("orbital", Some(9)).await,
            Err(LibraryError::InvalidColumn)
        ));
        assert!(!library
            .remove_track(&imported.playlist_id, &entry)
            .await
            .unwrap());
        assert!(library.remove_track(id, &entry).await.unwrap());
        drop(library);

        let library = Library::open(&path, catalog).await.unwrap();
        let columns = library.columns().await.unwrap();
        assert_eq!(columns[0].tracks[0].entry_id, track.entry_id);
        assert_eq!(columns[0].tracks.len(), first.tracks.len());
        assert_eq!(columns.last().unwrap().playlist_id, imported.playlist_id);
    }
}
