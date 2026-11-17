use crate::models::{
    YoutubePlaylist, YoutubePlaylistSummary, YoutubeReply, YoutubeRequest, YoutubeStatus,
};
use crate::youtube_oauth::{open_external, YoutubeOAuth};
use reqwest::Url;
use std::path::Path;
use std::time::Duration;
use tauri::State;
use tokio::sync::Mutex;
use uraniborg_core::{playlist_id, video_id, Library, YoutubeAccess, YoutubeApi, YoutubeError};

pub(crate) struct YoutubeService(pub Result<YoutubeServices, String>);

pub(crate) struct YoutubeServices {
    api: YoutubeApi,
    account: Mutex<Account>,
}

struct Account {
    oauth: YoutubeOAuth,
    channel: Option<String>,
}

struct AuthorizedAccess {
    token: String,
    channel: String,
}

enum PlaylistChange {
    Metadata(YoutubePlaylistSummary),
    Added(String),
    Removed(String),
    Moved { entry_id: String, position: usize },
}

impl PlaylistChange {
    fn is_visible_in(&self, playlist: &YoutubePlaylist) -> bool {
        match self {
            Self::Metadata(expected) => {
                let actual = &playlist.summary;
                actual.title == expected.title
                    && actual.description == expected.description
                    && actual.privacy == expected.privacy
            }
            Self::Added(entry_id) => playlist.items.iter().any(|item| item.entry_id == *entry_id),
            Self::Removed(entry_id) => {
                !playlist.items.iter().any(|item| item.entry_id == *entry_id)
            }
            Self::Moved { entry_id, position } => playlist
                .items
                .iter()
                .any(|item| item.entry_id == *entry_id && item.position == *position),
        }
    }
}

impl YoutubeServices {
    pub(crate) fn new(use_system_keychain: bool, client_file: &Path) -> Result<Self, String> {
        Ok(Self {
            api: YoutubeApi::new().map_err(message)?,
            account: Mutex::new(Account {
                oauth: YoutubeOAuth::new(use_system_keychain, client_file)?,
                channel: None,
            }),
        })
    }

    fn status(&self, account: &Account) -> YoutubeReply {
        let connected = account.oauth.has_session() && account.channel.is_some();
        YoutubeReply::Status(YoutubeStatus {
            can_connect: account.oauth.configured(),
            connected,
        })
    }

    async fn authorize(
        &self,
        library: &Library,
        account: &mut Account,
    ) -> Result<Option<AuthorizedAccess>, String> {
        let Some(token) = account.oauth.access_token().await? else {
            account.channel = None;
            if account.oauth.configured() {
                library
                    .clear_youtube_account_data()
                    .await
                    .map_err(message)?;
            }
            return Ok(None);
        };
        let channel = match &account.channel {
            Some(channel) => channel.clone(),
            None => {
                let channel = self
                    .api
                    .channel_id(&token)
                    .await
                    .map_err(|error| api_message(account, error))?;
                account.channel = Some(channel.clone());
                channel
            }
        };
        Ok(Some(AuthorizedAccess { token, channel }))
    }

    async fn require_authorized(
        &self,
        library: &Library,
        account: &mut Account,
    ) -> Result<AuthorizedAccess, String> {
        self.authorize(library, account)
            .await?
            .ok_or_else(|| connect_required(account.oauth.configured()))
    }

    async fn refresh_after_write(
        &self,
        library: &Library,
        id: &str,
        user: &AuthorizedAccess,
        account: &mut Account,
        change: PlaylistChange,
    ) -> Result<YoutubeReply, String> {
        let mut confirmed = None;
        // YouTube's list endpoints may briefly return an earlier version after acknowledging a write.
        for delay in [0, 300, 700, 1500, 2500] {
            if delay != 0 {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            match self.api.playlist(id, user.access()).await {
                Ok(playlist) if change.is_visible_in(&playlist) => {
                    confirmed = Some(playlist);
                    break;
                }
                Ok(_) | Err(YoutubeError::PlaylistNotFound | YoutubeError::PlaylistChanged) => {}
                Err(error) => {
                    let error = api_message(account, error);
                    return Err(format!(
                        "YouTube 已保存更改，歌单刷新失败。可重新读取歌单 {id}。{error}"
                    ));
                }
            }
        }
        let mut playlist = confirmed
            .ok_or_else(|| "YouTube 已保存更改，列表暂未更新。请刷新歌单确认。".to_owned())?;
        if library
            .saved_youtube_playlist(id, Some(&user.channel))
            .await
            .map_err(message)?
            .is_some()
        {
            library
                .save_youtube_playlist(&mut playlist, Some(&user.channel))
                .await
                .map_err(|error| format!("YouTube 已保存更改，本地更新失败。{error}"))?;
        }
        Ok(YoutubeReply::Playlist(playlist))
    }

    async fn run(
        &self,
        request: YoutubeRequest,
        library: &Library,
    ) -> Result<YoutubeReply, String> {
        let mut account = self.account.lock().await;
        match request {
            YoutubeRequest::Status => {
                if let Err(error) = self.authorize(library, &mut account).await {
                    if account.channel.is_none() || !account.oauth.has_session() {
                        return Err(error);
                    }
                }
                Ok(self.status(&account))
            }
            YoutubeRequest::ConfigureClient { client_json } => {
                account.oauth.configure_client(&client_json)?;
                Ok(self.status(&account))
            }
            YoutubeRequest::Connect => {
                if self.authorize(library, &mut account).await?.is_some() {
                    return Ok(self.status(&account));
                }
                account.oauth.connect().await?;
                account.channel = None;
                self.require_authorized(library, &mut account).await?;
                Ok(self.status(&account))
            }
            YoutubeRequest::Disconnect => {
                let revoked = account.oauth.disconnect().await;
                if !account.oauth.has_session() {
                    account.channel = None;
                    library
                        .clear_youtube_account_data()
                        .await
                        .map_err(message)?;
                }
                revoked?;
                Ok(self.status(&account))
            }
            YoutubeRequest::Imported => Ok(YoutubeReply::Lists(
                library
                    .youtube_imports(account.channel.as_deref())
                    .await
                    .map_err(message)?,
            )),
            YoutubeRequest::Forget { playlist_id: id } => {
                library
                    .forget_youtube_playlist(&playlist_id(&id).map_err(message)?)
                    .await
                    .map_err(message)?;
                Ok(YoutubeReply::Done)
            }
            YoutubeRequest::Open {
                playlist_id: id,
                video_id: video,
            } => {
                let id = playlist_id(&id).map_err(message)?;
                let mut url = Url::parse(if video.is_some() {
                    "https://music.youtube.com/watch"
                } else {
                    "https://music.youtube.com/playlist"
                })
                .map_err(message)?;
                if let Some(video) = video {
                    url.query_pairs_mut()
                        .append_pair("v", &video_id(&video).map_err(message)?);
                }
                url.query_pairs_mut().append_pair("list", &id);
                open_external(&url)?;
                Ok(YoutubeReply::Done)
            }
            YoutubeRequest::Mine => {
                let user = self.require_authorized(library, &mut account).await?;
                Ok(YoutubeReply::Lists(
                    self.api
                        .mine(user.access())
                        .await
                        .map_err(|error| api_message(&mut account, error))?,
                ))
            }
            YoutubeRequest::Import { reference } => {
                let id = playlist_id(&reference).map_err(message)?;
                let user = self.require_authorized(library, &mut account).await?;
                let mut playlist = self
                    .api
                    .playlist(&id, user.access())
                    .await
                    .map_err(|error| api_message(&mut account, error))?;
                library
                    .save_youtube_playlist(&mut playlist, Some(&user.channel))
                    .await
                    .map_err(message)?;
                Ok(YoutubeReply::Playlist(playlist))
            }
            YoutubeRequest::Load { playlist_id: id } => {
                let id = playlist_id(&id).map_err(message)?;
                let saved = library
                    .saved_youtube_playlist(&id, account.channel.as_deref())
                    .await
                    .map_err(message)?;
                let saved = match saved {
                    Some(saved) if !account.oauth.has_session() => {
                        return Ok(YoutubeReply::Playlist(saved));
                    }
                    saved => saved,
                };
                let user = self.require_authorized(library, &mut account).await?;
                let fetched = self.api.playlist(&id, user.access()).await;
                match (fetched, saved) {
                    (Ok(mut playlist), saved) => {
                        if saved.is_some() {
                            library
                                .save_youtube_playlist(&mut playlist, Some(&user.channel))
                                .await
                                .map_err(message)?;
                        }
                        Ok(YoutubeReply::Playlist(playlist))
                    }
                    (Err(YoutubeError::Network(_)), Some(saved)) => {
                        Ok(YoutubeReply::Playlist(saved))
                    }
                    (Err(error), _) => Err(api_message(&mut account, error)),
                }
            }
            mutation => {
                let user = self.require_authorized(library, &mut account).await?;
                self.mutate(mutation, library, &user, &mut account).await
            }
        }
    }

    async fn mutate(
        &self,
        request: YoutubeRequest,
        library: &Library,
        user: &AuthorizedAccess,
        account: &mut Account,
    ) -> Result<YoutubeReply, String> {
        let access = user.access();
        let (id, change) = match request {
            YoutubeRequest::Create {
                title,
                description,
                privacy,
            } => {
                let playlist = self
                    .api
                    .create(&title, &description, privacy, access)
                    .await
                    .map_err(|error| api_message(account, error))?;
                return Ok(YoutubeReply::Playlist(playlist));
            }
            YoutubeRequest::Update {
                playlist_id: id,
                title,
                description,
                privacy,
            } => {
                let id = playlist_id(&id).map_err(message)?;
                let summary = self
                    .api
                    .update(&id, &title, &description, privacy, access)
                    .await
                    .map_err(|error| api_message(account, error))?;
                (id, PlaylistChange::Metadata(summary))
            }
            YoutubeRequest::Add {
                playlist_id: id,
                reference,
            } => {
                let id = playlist_id(&id).map_err(message)?;
                let entry_id = self
                    .api
                    .add(&id, &video_id(&reference).map_err(message)?, access)
                    .await
                    .map_err(|error| api_message(account, error))?;
                (id, PlaylistChange::Added(entry_id))
            }
            YoutubeRequest::Remove {
                playlist_id: id,
                entry_id,
            } => {
                let id = playlist_id(&id).map_err(message)?;
                self.api
                    .remove(&id, &entry_id, access)
                    .await
                    .map_err(|error| api_message(account, error))?;
                (id, PlaylistChange::Removed(entry_id))
            }
            YoutubeRequest::Move {
                playlist_id: id,
                entry_id,
                position,
            } => {
                let id = playlist_id(&id).map_err(message)?;
                self.api
                    .move_entry(&id, &entry_id, position, access)
                    .await
                    .map_err(|error| api_message(account, error))?;
                (id, PlaylistChange::Moved { entry_id, position })
            }
            YoutubeRequest::Delete { playlist_id: id } => {
                let id = playlist_id(&id).map_err(message)?;
                self.api
                    .delete(&id, access)
                    .await
                    .map_err(|error| api_message(account, error))?;
                library
                    .forget_youtube_playlist(&id)
                    .await
                    .map_err(|error| format!("YouTube 已删除歌单，本地副本清理失败。{error}"))?;
                return Ok(YoutubeReply::Done);
            }
            _ => unreachable!("Only remote playlist mutations reach this handler"),
        };
        self.refresh_after_write(library, &id, user, account, change)
            .await
    }
}

impl AuthorizedAccess {
    fn access(&self) -> YoutubeAccess<'_> {
        YoutubeAccess {
            token: &self.token,
            channel_id: &self.channel,
        }
    }
}

fn connect_required(configured: bool) -> String {
    if configured {
        "请先在 YouTube Music 面板连接 Google 账号".into()
    } else {
        "尚未配置 Google 桌面 OAuth 客户端".into()
    }
}

fn api_message(account: &mut Account, error: YoutubeError) -> String {
    if matches!(error, YoutubeError::Unauthorized) {
        account.oauth.invalidate_access_token();
    }
    error.to_string()
}

fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[tauri::command]
pub(crate) async fn youtube_playlist_action(
    args: YoutubeRequest,
    service: State<'_, YoutubeService>,
    media: State<'_, crate::catalog_commands::MediaServices>,
) -> Result<YoutubeReply, String> {
    service
        .0
        .as_ref()
        .map_err(Clone::clone)?
        .run(args, &media.library)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{YoutubePlaylistItem, YoutubePrivacy};
    use std::{path::Path, sync::Arc};
    use uraniborg_core::Catalog;

    #[test]
    fn write_confirmation_distinguishes_stale_rows_and_duplicate_occurrences() {
        let summary = YoutubePlaylistSummary {
            id: "PLtest".into(),
            title: "Before edit".into(),
            description: String::new(),
            privacy: YoutubePrivacy::Private,
            can_edit: true,
        };
        let entry = |entry_id: &str, position| YoutubePlaylistItem {
            entry_id: entry_id.into(),
            video_id: "same-video".into(),
            title: "Song".into(),
            artist: String::new(),
            position,
        };
        let mut playlist = YoutubePlaylist {
            summary: summary.clone(),
            items: vec![entry("first", 0), entry("second", 1)],
            imported_at: None,
        };
        let mut renamed = summary;
        renamed.title = "After edit".into();
        let edited = PlaylistChange::Metadata(renamed.clone());
        let added = PlaylistChange::Added("new".into());
        let removed = PlaylistChange::Removed("first".into());
        let moved = PlaylistChange::Moved {
            entry_id: "second".into(),
            position: 0,
        };
        for change in [&edited, &added, &removed, &moved] {
            assert!(!change.is_visible_in(&playlist));
        }
        playlist.summary = renamed;
        playlist.items = vec![entry("second", 0), entry("new", 1)];
        for change in [edited, added, removed, moved] {
            assert!(change.is_visible_in(&playlist));
        }
    }

    #[tokio::test]
    async fn signed_out_remote_actions_require_oauth_and_saved_public_snapshots_are_readable() {
        let folder = tempfile::tempdir().unwrap();
        let catalog_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../assets/authorized-media");
        let catalog = Arc::new(Catalog::load(&catalog_path).unwrap());
        let library = Library::open(&folder.path().join("library.sqlite3"), catalog)
            .await
            .unwrap();
        let service =
            YoutubeServices::new(false, &folder.path().join("youtube-oauth-client.json")).unwrap();
        let saved_id = "PLpreviouslyImported";
        let mut saved = YoutubePlaylist {
            summary: YoutubePlaylistSummary {
                id: saved_id.into(),
                title: "Existing local snapshot".into(),
                description: String::new(),
                privacy: YoutubePrivacy::Public,
                can_edit: false,
            },
            items: vec![],
            imported_at: None,
        };
        library
            .save_youtube_playlist(&mut saved, None)
            .await
            .unwrap();
        let authorization_error = connect_required(service.account.lock().await.oauth.configured());
        for request in [
            YoutubeRequest::Import {
                reference: format!("https://music.youtube.com/playlist?list={saved_id}"),
            },
            YoutubeRequest::Load {
                playlist_id: "PLnotYetImported".into(),
            },
            YoutubeRequest::Mine,
            YoutubeRequest::Create {
                title: "test".into(),
                description: String::new(),
                privacy: YoutubePrivacy::Private,
            },
        ] {
            assert_eq!(
                service.run(request, &library).await.unwrap_err(),
                authorization_error
            );
        }
        let lists = service
            .run(YoutubeRequest::Imported, &library)
            .await
            .unwrap();
        assert!(
            matches!(lists, YoutubeReply::Lists(lists) if lists.len() == 1 && lists[0].id == saved_id)
        );
        let loaded = service
            .run(
                YoutubeRequest::Load {
                    playlist_id: saved_id.into(),
                },
                &library,
            )
            .await
            .unwrap();
        assert!(
            matches!(loaded, YoutubeReply::Playlist(playlist) if playlist.summary.id == saved_id)
        );
    }
}
