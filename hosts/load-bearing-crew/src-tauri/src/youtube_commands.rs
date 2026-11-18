use crate::models::{
    SearchResults, YoutubePlaylist, YoutubePlaylistSummary, YoutubePolicyPage, YoutubePrivacy,
    YoutubeReply, YoutubeRequest, YoutubeStatus,
};
use crate::youtube_oauth::{open_external, YoutubeOAuth};
use reqwest::Url;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    RwLock,
};
use std::time::Duration;
use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex;
use uraniborg_core::{playlist_id, video_id, Library, YoutubeAccess, YoutubeApi, YoutubeError};

pub(crate) struct YoutubeService(pub Result<YoutubeServices, String>);

pub(crate) struct YoutubeServices {
    api: YoutubeApi,
    account: Mutex<Account>,
    snapshot: RwLock<YoutubeStatus>,
    warming: AtomicBool,
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
        let oauth = YoutubeOAuth::new(use_system_keychain, client_file)?;
        let account = Account {
            channel: oauth.channel().map(str::to_owned),
            oauth,
        };
        let snapshot = RwLock::new(Self::account_status(&account));
        Ok(Self {
            api: YoutubeApi::new().map_err(message)?,
            account: Mutex::new(account),
            snapshot,
            warming: AtomicBool::new(false),
        })
    }

    fn account_status(account: &Account) -> YoutubeStatus {
        YoutubeStatus {
            can_connect: account.oauth.configured(),
            connected: account.oauth.has_session() && account.channel.is_some(),
            channel_id: account.channel.clone(),
        }
    }

    pub(crate) fn status(&self) -> YoutubeStatus {
        self.snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn publish(&self, account: &Account) -> YoutubeReply {
        let status = Self::account_status(account);
        *self.snapshot.write().unwrap_or_else(|e| e.into_inner()) = status.clone();
        YoutubeReply::Status(status)
    }

    pub(crate) async fn search(&self, query: &str, filter: &str) -> Result<SearchResults, String> {
        let mut account = self.account.lock().await;
        let user = self.require_authorized(&mut account).await?;
        self.api
            .search(query, filter, &user.token)
            .await
            .map_err(|err| api_message(&mut account, err))
    }

    async fn authorize(&self, account: &mut Account) -> Result<Option<AuthorizedAccess>, String> {
        let Some(token) = account.oauth.access_token().await? else {
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
                account.oauth.remember_channel(channel.clone()).await?;
                account.channel = Some(channel.clone());
                self.publish(account);
                channel
            }
        };
        Ok(Some(AuthorizedAccess { token, channel }))
    }

    async fn require_authorized(&self, account: &mut Account) -> Result<AuthorizedAccess, String> {
        self.authorize(account)
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
        if matches!(request, YoutubeRequest::Status) {
            return Ok(YoutubeReply::Status(self.status()));
        }
        if let YoutubeRequest::OpenPolicy { page } = &request {
            let url = match page {
                YoutubePolicyPage::Privacy => "https://xiaotwu.github.io/Nasti/privacy.html",
                YoutubePolicyPage::NastiTerms => "https://xiaotwu.github.io/Nasti/terms.html",
                YoutubePolicyPage::YoutubeTerms => "https://www.youtube.com/t/terms",
                YoutubePolicyPage::GooglePermissions => "https://myaccount.google.com/permissions",
            };
            open_external(&Url::parse(url).expect("known policy URL"))?;
            return Ok(YoutubeReply::Done);
        }
        let mut account = self.account.lock().await;
        match request {
            YoutubeRequest::Configure { json } => {
                account.oauth.configure(&json)?;
                Ok(self.publish(&account))
            }
            YoutubeRequest::Connect => {
                account.oauth.connect().await?;
                account.channel = None;
                self.require_authorized(&mut account).await?;
                Ok(self.publish(&account))
            }
            YoutubeRequest::Sync => {
                let user = self.require_authorized(&mut account).await?;
                let owned = self
                    .api
                    .mine(user.access())
                    .await
                    .map_err(|error| api_message(&mut account, error))?;
                for summary in &owned {
                    let mut playlist = self
                        .api
                        .playlist(&summary.id, user.access())
                        .await
                        .map_err(|error| api_message(&mut account, error))?;
                    if playlist.summary.can_edit {
                        library
                            .save_youtube_playlist(&mut playlist, Some(&user.channel))
                            .await
                            .map_err(message)?;
                    }
                }
                let ids = owned
                    .into_iter()
                    .map(|summary| summary.id)
                    .collect::<Vec<_>>();
                library
                    .remove_missing_youtube_playlists(&user.channel, &ids)
                    .await
                    .map_err(message)?;
                let visible = library
                    .youtube_imports(Some(&user.channel))
                    .await
                    .map_err(message)?;
                let mut failed = None;
                for subscription in visible.iter().filter(|summary| !summary.can_edit) {
                    match self.api.playlist(&subscription.id, user.access()).await {
                        Ok(mut playlist) if playlist.summary.privacy == YoutubePrivacy::Public => {
                            library
                                .save_youtube_playlist(&mut playlist, None)
                                .await
                                .map_err(message)?;
                        }
                        Ok(_) => {
                            failed.get_or_insert_with(|| "部分订阅歌单已改为非公开".to_owned());
                        }
                        Err(error) => {
                            failed.get_or_insert_with(|| api_message(&mut account, error));
                        }
                    }
                }
                if let Some(error) = failed {
                    return Err(error);
                }
                Ok(YoutubeReply::Lists(
                    library
                        .youtube_imports(Some(&user.channel))
                        .await
                        .map_err(message)?,
                ))
            }
            YoutubeRequest::Cached { playlist_id: id } => {
                let id = playlist_id(&id).map_err(message)?;
                let playlist = library
                    .saved_youtube_playlist(&id, account.channel.as_deref())
                    .await
                    .map_err(message)?
                    .ok_or("歌单还未同步到曲库")?;
                Ok(YoutubeReply::Playlist(playlist))
            }
            YoutubeRequest::Disconnect => {
                let revoked = account.oauth.disconnect().await;
                if !account.oauth.has_session() {
                    account.channel = None;
                    self.publish(&account);
                    library
                        .clear_youtube_account_data()
                        .await
                        .map_err(message)?;
                }
                let status = self.publish(&account);
                revoked?;
                Ok(status)
            }
            YoutubeRequest::Imported => Ok(YoutubeReply::Lists(
                library
                    .youtube_imports(account.channel.as_deref())
                    .await
                    .map_err(message)?,
            )),
            YoutubeRequest::Forget { playlist_id: id } => {
                let id = playlist_id(&id).map_err(message)?;
                if library
                    .saved_youtube_playlist(&id, None)
                    .await
                    .map_err(message)?
                    .is_some()
                {
                    library
                        .forget_youtube_playlist(&id)
                        .await
                        .map_err(message)?;
                }
                Ok(YoutubeReply::Done)
            }
            YoutubeRequest::Mine => {
                let user = self.require_authorized(&mut account).await?;
                Ok(YoutubeReply::Lists(
                    self.api
                        .mine(user.access())
                        .await
                        .map_err(|error| api_message(&mut account, error))?,
                ))
            }
            YoutubeRequest::Import { reference } => {
                let id = playlist_id(&reference).map_err(message)?;
                let user = self.require_authorized(&mut account).await?;
                let mut playlist = self
                    .api
                    .playlist(&id, user.access())
                    .await
                    .map_err(|error| api_message(&mut account, error))?;
                if !playlist.summary.can_edit && playlist.summary.privacy != YoutubePrivacy::Public
                {
                    return Err("只能订阅他人的公开 YouTube 歌单".into());
                }
                let owner = playlist.summary.can_edit.then_some(user.channel.as_str());
                library
                    .save_youtube_playlist(&mut playlist, owner)
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
                let user = self.require_authorized(&mut account).await?;
                let fetched = self.api.playlist(&id, user.access()).await;
                match (fetched, saved) {
                    (Ok(mut playlist), saved) => {
                        if saved.is_some()
                            && (playlist.summary.can_edit
                                || playlist.summary.privacy == YoutubePrivacy::Public)
                        {
                            let owner = playlist.summary.can_edit.then_some(user.channel.as_str());
                            library
                                .save_youtube_playlist(&mut playlist, owner)
                                .await
                                .map_err(message)?;
                        }
                        Ok(YoutubeReply::Playlist(playlist))
                    }
                    (Err(error), _) => Err(api_message(&mut account, error)),
                }
            }
            mutation => {
                let user = self.require_authorized(&mut account).await?;
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
                let mut playlist = self
                    .api
                    .create(&title, &description, privacy, access)
                    .await
                    .map_err(|error| api_message(account, error))?;
                library
                    .save_youtube_playlist(&mut playlist, Some(&user.channel))
                    .await
                    .map_err(message)?;
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
        "请先连接 Google 账号".into()
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
    app: AppHandle,
    args: YoutubeRequest,
    service: State<'_, YoutubeService>,
    media: State<'_, crate::catalog_commands::MediaServices>,
) -> Result<YoutubeReply, String> {
    let youtube = service.0.as_ref().map_err(Clone::clone)?;
    if matches!(args, YoutubeRequest::Status) {
        let status = youtube.status();
        if status.can_connect && !status.connected {
            if let Ok(mut account) = youtube.account.try_lock() {
                if account.oauth.may_restore_legacy() {
                    let _ = tokio::time::timeout(
                        Duration::from_secs(8),
                        youtube.authorize(&mut account),
                    )
                    .await;
                    return Ok(YoutubeReply::Status(youtube.status()));
                }
            }
        }
        if status.can_connect && !youtube.warming.swap(true, Ordering::AcqRel) {
            tauri::async_runtime::spawn(async move {
                let service = app.state::<YoutubeService>();
                if let Ok(youtube) = &service.0 {
                    let mut account = youtube.account.lock().await;
                    let _ = youtube.authorize(&mut account).await;
                    youtube.warming.store(false, Ordering::Release);
                }
            });
        }
        return Ok(YoutubeReply::Status(status));
    }
    youtube.run(args, &media.library).await
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
