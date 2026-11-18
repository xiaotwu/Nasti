use super::{Library, LibraryError};
use crate::models::{YoutubePlaylist, YoutubePlaylistSummary};
use std::{
    collections::VecDeque,
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Default)]
pub(super) struct YoutubeSnapshots(VecDeque<Snapshot>);

struct Snapshot {
    id: String,
    account: Option<String>,
    json: Vec<u8>,
}

impl Snapshot {
    fn visible(&self, account: Option<&str>) -> bool {
        self.account.is_none() || self.account.as_deref() == account
    }

    fn playlist(&self) -> Result<YoutubePlaylist, LibraryError> {
        Ok(serde_json::from_slice(&self.json)?)
    }
}

impl Library {
    pub fn save_youtube_playlist(
        &self,
        playlist: &mut YoutubePlaylist,
        account: Option<&str>,
    ) -> Result<(), LibraryError> {
        playlist.imported_at = Some(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        );
        let json = serde_json::to_vec(playlist)?;
        if json.len() > MAX_SNAPSHOT_BYTES {
            return Err(LibraryError::YoutubeSnapshotTooLarge);
        }
        let mut snapshots = self.youtube_snapshots.lock().unwrap();
        snapshots.0.retain(|item| item.id != playlist.summary.id);
        let mut total = snapshots
            .0
            .iter()
            .map(|item| item.json.len())
            .sum::<usize>()
            + json.len();
        while total > MAX_SNAPSHOT_BYTES {
            total -= snapshots.0.pop_back().unwrap().json.len();
        }
        snapshots.0.push_front(Snapshot {
            id: playlist.summary.id.clone(),
            account: account.map(str::to_owned),
            json,
        });
        Ok(())
    }

    pub fn youtube_imports(
        &self,
        account: Option<&str>,
    ) -> Result<Vec<YoutubePlaylistSummary>, LibraryError> {
        self.youtube_snapshots
            .lock()
            .unwrap()
            .0
            .iter()
            .filter(|item| item.visible(account))
            .map(|item| Ok(item.playlist()?.summary))
            .collect()
    }

    pub fn saved_youtube_playlist(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<Option<YoutubePlaylist>, LibraryError> {
        self.youtube_snapshots
            .lock()
            .unwrap()
            .0
            .iter()
            .find(|item| item.id == id && item.visible(account))
            .map(Snapshot::playlist)
            .transpose()
    }

    pub fn forget_youtube_playlist(&self, id: &str) {
        self.youtube_snapshots
            .lock()
            .unwrap()
            .0
            .retain(|item| item.id != id);
    }

    pub fn clear_youtube_account_data(&self) {
        self.youtube_snapshots.lock().unwrap().0.clear();
    }

    pub(super) async fn scrub_legacy_youtube_snapshots(&self) -> Result<(), LibraryError> {
        let mut connection = self.pool.acquire().await?;
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM sqlite_schema WHERE type = 'table'
             AND name = 'library_youtube_playlists')",
        )
        .fetch_one(&mut *connection)
        .await?;
        if !exists {
            return Ok(());
        }

        // Keep the table until the old pages and WAL have been rewritten. An interruption
        // before the checkpoint leaves the table in place and causes the scrub to run again.
        sqlx::raw_sql(
            "PRAGMA secure_delete = ON;
             PRAGMA temp_store = MEMORY;
             DELETE FROM library_youtube_playlists;
             VACUUM;",
        )
        .execute(&mut *connection)
        .await?;
        let (busy, _, _): (i32, i32, i32) = sqlx::query_as("PRAGMA wal_checkpoint(TRUNCATE)")
            .fetch_one(&mut *connection)
            .await?;
        if busy != 0 {
            return Err(LibraryError::YoutubeMigrationBusy);
        }
        sqlx::query("DROP TABLE library_youtube_playlists")
            .execute(&mut *connection)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{models::YoutubePrivacy, Catalog};
    use sqlx::{sqlite::SqliteConnectOptions, Connection, SqliteConnection};
    use std::{path::Path, sync::Arc};

    #[tokio::test]
    async fn startup_scrubs_legacy_db_and_wal_and_new_snapshots_are_session_only() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("library.sqlite3");
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/authorized-media");
        let catalog = Arc::new(Catalog::load(&root).unwrap());
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
        let mut old = SqliteConnection::connect_with(&options).await.unwrap();
        let legacy = "synthetic-legacy-youtube-private-marker";
        sqlx::raw_sql(
            "PRAGMA secure_delete = OFF;
             CREATE TABLE library_youtube_playlists (playlist_json TEXT);
             CREATE TABLE unrelated (data TEXT);
             INSERT INTO unrelated VALUES ('original local data');",
        )
        .execute(&mut old)
        .await
        .unwrap();
        sqlx::query("INSERT INTO library_youtube_playlists VALUES (?)")
            .bind(legacy.repeat(400))
            .execute(&mut old)
            .await
            .unwrap();
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&mut old)
            .await
            .unwrap();
        assert!(std::fs::read(&path)
            .unwrap()
            .windows(legacy.len())
            .any(|b| b == legacy.as_bytes()));
        sqlx::query("DELETE FROM library_youtube_playlists")
            .execute(&mut old)
            .await
            .unwrap();
        let wal = folder.path().join("library.sqlite3-wal");
        assert!(std::fs::read(&wal)
            .unwrap()
            .windows(legacy.len())
            .any(|b| b == legacy.as_bytes()));
        let library = Library::open(&path, Arc::clone(&catalog)).await.unwrap();
        let mut playlist = YoutubePlaylist {
            summary: YoutubePlaylistSummary {
                id: "PLtest".into(),
                title: "synthetic-current-session-only-marker".into(),
                description: String::new(),
                privacy: YoutubePrivacy::Private,
                can_edit: true,
            },
            items: Vec::new(),
            imported_at: None,
        };
        library
            .save_youtube_playlist(&mut playlist, Some("channel-a"))
            .unwrap();
        assert!(library.youtube_imports(None).unwrap().is_empty());
        assert!(library
            .saved_youtube_playlist("PLtest", Some("channel-b"))
            .unwrap()
            .is_none());
        assert!(library
            .saved_youtube_playlist("PLtest", Some("channel-a"))
            .unwrap()
            .is_some());
        for name in [
            "library.sqlite3",
            "library.sqlite3-wal",
            "library.sqlite3-shm",
        ] {
            let bytes = std::fs::read(folder.path().join(name)).unwrap_or_default();
            for needle in [legacy, playlist.summary.title.as_str()] {
                assert!(
                    !bytes.windows(needle.len()).any(|b| b == needle.as_bytes()),
                    "{name}"
                );
            }
        }
        let original: String = sqlx::query_scalar("SELECT data FROM unrelated")
            .fetch_one(&library.pool)
            .await
            .unwrap();
        assert_eq!(original, "original local data");
        library.pool.close().await;
        old.close().await.unwrap();
        let reopened = Library::open(&path, catalog).await.unwrap();
        assert!(reopened
            .youtube_imports(Some("channel-a"))
            .unwrap()
            .is_empty());
    }
}
