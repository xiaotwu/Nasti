use super::{Library, LibraryError};
use crate::models::{PlaylistColumn, TrackItem, YoutubePlaylist, YoutubePlaylistSummary};
use std::time::{SystemTime, UNIX_EPOCH};

const SNAPSHOT_MAX_AGE_MS: i64 = 30 * 24 * 60 * 60 * 1000;

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

    pub async fn youtube_columns(
        &self,
        account: Option<&str>,
    ) -> Result<Vec<PlaylistColumn>, LibraryError> {
        let rows = sqlx::query_scalar::<_, String>(
            "SELECT playlist_json FROM library_youtube_playlists
             WHERE account_id IS NULL OR account_id = ?
             ORDER BY account_id IS NULL, id",
        )
        .bind(account)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .enumerate()
            .map(|(index, row)| {
                let playlist: YoutubePlaylist = serde_json::from_str(row)?;
                let mut column = crate::youtube::playlist_column(playlist.summary, index);
                column.tracks = playlist
                    .items
                    .into_iter()
                    .filter(|item| crate::youtube::video_id(&item.video_id).is_ok())
                    .map(|item| {
                        let video = !item.artist.is_empty() && !item.artist.ends_with(" - Topic");
                        let artist = item.artist.trim_end_matches(" - Topic").to_owned();
                        let mut track =
                            crate::youtube::music_track(&item.video_id, item.title, artist, video);
                        track.entry_id = Some(item.entry_id);
                        track.track_number = Some(item.position as u32 + 1);
                        track
                    })
                    .collect();
                Ok(column)
            })
            .collect()
    }

    pub async fn save_youtube_track(&self, track: &TrackItem) -> Result<(), LibraryError> {
        sqlx::query(
            "INSERT INTO library_youtube_media (id, track_json, saved_at) VALUES (?, ?, ?)
                     ON CONFLICT(id) DO UPDATE SET track_json = excluded.track_json, saved_at = excluded.saved_at",
        )
        .bind(&track.video_id)
        .bind(serde_json::to_string(track)?)
            .bind(now_ms())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn forget_youtube_track(&self, id: &str) -> Result<(), LibraryError> {
        sqlx::query("DELETE FROM library_youtube_media WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn saved_youtube_track(&self, id: &str) -> Result<Option<TrackItem>, LibraryError> {
        let row = sqlx::query_scalar::<_, String>(
            "SELECT track_json FROM library_youtube_media WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.as_deref().map(serde_json::from_str).transpose()?)
    }

    pub async fn remove_missing_youtube_playlists(
        &self,
        account: &str,
        ids: &[String],
    ) -> Result<(), LibraryError> {
        sqlx::query(
            "DELETE FROM library_youtube_playlists WHERE account_id = ?
                     AND id NOT IN (SELECT value FROM json_each(?))",
        )
        .bind(account)
        .bind(serde_json::to_string(ids)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn expire_youtube_snapshots(&self) -> Result<bool, LibraryError> {
        let cutoff = now_ms() - SNAPSHOT_MAX_AGE_MS;
        let mut tx = self.pool.begin().await?;
        let expired = sqlx::query("DELETE FROM library_youtube_playlists WHERE imported_at <= ?")
            .bind(cutoff)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        sqlx::query("DELETE FROM library_youtube_media WHERE saved_at <= ?")
            .bind(cutoff)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(expired > 0)
    }

    pub(super) async fn migrate_youtube_subscriptions(&self) -> Result<(), LibraryError> {
        let columns = sqlx::query_scalar::<_, String>(
            "SELECT name FROM pragma_table_info('library_youtube_media')",
        )
        .fetch_all(&self.pool)
        .await?;
        if !columns.iter().any(|name| name == "saved_at") {
            sqlx::query(
                "ALTER TABLE library_youtube_media ADD COLUMN saved_at INTEGER NOT NULL DEFAULT 0",
            )
            .execute(&self.pool)
            .await?;
        }
        sqlx::query(
            "UPDATE library_youtube_playlists SET account_id = NULL
                     WHERE account_id IS NOT NULL AND json_extract(playlist_json, '$.canEdit') = 0
                     AND json_extract(playlist_json, '$.privacy') IN ('public', 'unlisted')",
        )
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
    async fn columns_include_more_than_nine_owned_lists_and_public_subscriptions() {
        let folder = tempfile::tempdir().unwrap();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/authorized-media");
        let catalog = Arc::new(Catalog::load(&root).unwrap());
        let library = Library::open(&folder.path().join("library.sqlite3"), catalog)
            .await
            .unwrap();
        for index in 0..15 {
            let owned = index < 14;
            let mut list = YoutubePlaylist {
                summary: YoutubePlaylistSummary {
                    id: format!("PLfixture{index:02}"),
                    title: format!("Playlist {index}"),
                    description: String::new(),
                    privacy: if owned {
                        YoutubePrivacy::Private
                    } else {
                        YoutubePrivacy::Public
                    },
                    can_edit: owned,
                },
                items: vec![YoutubePlaylistItem {
                    entry_id: "unique-entry".into(),
                    video_id: "AbcDef_h-j0".into(),
                    title: "Fixture".into(),
                    artist: "Artist - Topic".into(),
                    position: 0,
                }],
                imported_at: None,
            };
            library
                .save_youtube_playlist(&mut list, owned.then_some("channel-a"))
                .await
                .unwrap();
        }
        let columns = library.youtube_columns(Some("channel-a")).await.unwrap();
        assert_eq!(columns.len(), 15);
        assert!(columns[13].is_editable);
        assert_eq!(columns[13].column_index, 13);
        assert_eq!(
            columns[13].tracks[0].entry_id.as_deref(),
            Some("unique-entry")
        );
        assert!(!columns[14].is_editable);
        assert_eq!(
            library
                .youtube_columns(Some("channel-b"))
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(library.youtube_columns(None).await.unwrap().len(), 1);
    }

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
            .bind(0_i64)
            .execute(&library.pool)
            .await
            .unwrap();
        assert_eq!(library.youtube_imports(None).await.unwrap().len(), 1);
        assert!(library.expire_youtube_snapshots().await.unwrap());
        assert!(library.youtube_imports(None).await.unwrap().is_empty());
    }
}
