use super::{Library, LibraryError};
use crate::models::{YoutubePlaylist, YoutubePlaylistSummary};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_AGE_MS: i64 = 30 * 24 * 60 * 60 * 1000;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |date| date.as_millis() as i64)
}

impl Library {
    pub async fn save_youtube_playlist(
        &self,
        playlist: &mut YoutubePlaylist,
        account: Option<&str>,
    ) -> Result<(), LibraryError> {
        let now = now_ms();
        playlist.imported_at = Some(now as u64);
        sqlx::query(
            "INSERT INTO library_youtube_playlists (id, account_id, imported_at, playlist_json)
             VALUES (?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET
             account_id = excluded.account_id, imported_at = excluded.imported_at,
             playlist_json = excluded.playlist_json",
        )
        .bind(&playlist.summary.id)
        .bind(account)
        .bind(now)
        .bind(serde_json::to_string(playlist)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn youtube_imports(
        &self,
        account: Option<&str>,
    ) -> Result<Vec<YoutubePlaylistSummary>, LibraryError> {
        self.prune_youtube_playlists().await?;
        let rows = sqlx::query_scalar::<_, String>(
            "SELECT playlist_json FROM library_youtube_playlists
             WHERE account_id IS NULL OR account_id = ? ORDER BY imported_at DESC, id",
        )
        .bind(account)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| Ok(serde_json::from_str::<YoutubePlaylist>(row)?.summary))
            .collect()
    }

    pub async fn saved_youtube_playlist(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<Option<YoutubePlaylist>, LibraryError> {
        self.prune_youtube_playlists().await?;
        let json = sqlx::query_scalar::<_, String>(
            "SELECT playlist_json FROM library_youtube_playlists
             WHERE id = ? AND (account_id IS NULL OR account_id = ?)",
        )
        .bind(id)
        .bind(account)
        .fetch_optional(&self.pool)
        .await?;
        Ok(json.as_deref().map(serde_json::from_str).transpose()?)
    }

    pub async fn forget_youtube_playlist(&self, id: &str) -> Result<(), LibraryError> {
        sqlx::query("DELETE FROM library_youtube_playlists WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn clear_youtube_account_data(&self) -> Result<(), LibraryError> {
        sqlx::query("DELETE FROM library_youtube_playlists WHERE account_id IS NOT NULL")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub(super) async fn prune_youtube_playlists(&self) -> Result<(), LibraryError> {
        sqlx::query("DELETE FROM library_youtube_playlists WHERE imported_at <= ?")
            .bind(now_ms() - MAX_AGE_MS)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        models::{YoutubePlaylistItem, YoutubePrivacy},
        Catalog,
    };
    use std::{path::Path, sync::Arc};

    #[tokio::test]
    async fn youtube_snapshot_refresh_reuses_remote_entry_ids_and_respects_account_access() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("library.sqlite3");
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/authorized-media");
        let catalog = Arc::new(Catalog::load(&root).unwrap());
        let library = Library::open(&path, Arc::clone(&catalog)).await.unwrap();
        let entry = |id: &str, position| YoutubePlaylistItem {
            entry_id: id.into(),
            video_id: "abcdefghijk".into(),
            title: "Same recording".into(),
            artist: "Artist".into(),
            position,
        };
        let mut playlist = YoutubePlaylist {
            summary: YoutubePlaylistSummary {
                id: "PLtest".into(),
                title: "Source".into(),
                description: String::new(),
                privacy: YoutubePrivacy::Private,
                can_edit: true,
            },
            items: vec![entry("first", 0), entry("second", 1)],
            imported_at: None,
        };
        library
            .save_youtube_playlist(&mut playlist, Some("channel-a"))
            .await
            .unwrap();
        playlist.items = vec![entry("second", 0), entry("first", 1)];
        library
            .save_youtube_playlist(&mut playlist, Some("channel-a"))
            .await
            .unwrap();
        library.pool.close().await;

        let library = Library::open(&path, catalog).await.unwrap();
        assert!(library.youtube_imports(None).await.unwrap().is_empty());
        assert!(library
            .saved_youtube_playlist("PLtest", Some("channel-b"))
            .await
            .unwrap()
            .is_none());
        let saved = library
            .saved_youtube_playlist("PLtest", Some("channel-a"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            saved
                .items
                .iter()
                .map(|item| item.entry_id.as_str())
                .collect::<Vec<_>>(),
            ["second", "first"]
        );
        assert!(saved.imported_at.is_some());
        library.clear_youtube_account_data().await.unwrap();
        assert!(library
            .youtube_imports(Some("channel-a"))
            .await
            .unwrap()
            .is_empty());

        playlist.summary.privacy = YoutubePrivacy::Public;
        library
            .save_youtube_playlist(&mut playlist, None)
            .await
            .unwrap();
        sqlx::query("UPDATE library_youtube_playlists SET imported_at = ?")
            .bind(now_ms() - MAX_AGE_MS)
            .execute(&library.pool)
            .await
            .unwrap();
        assert!(library.youtube_imports(None).await.unwrap().is_empty());
    }
}
