use crate::models::{
    YoutubePlaylist, YoutubePlaylistSummary, YoutubePolicyPage, YoutubeReply, YoutubeRequest,
    YoutubeStatus,
};
use crate::youtube_oauth::{launch_browser, open_external, SessionTime, YoutubeOAuth};
use reqwest::Url;
use std::path::Path;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{watch, Mutex};
use uraniborg_core::{playlist_id, video_id, Library, YoutubeAccess, YoutubeApi, YoutubeError};

const CHECK_TICK: Duration = Duration::from_secs(60);
const UNVERIFIED_LIMIT: Duration = Duration::from_secs(60 * 60);
const DATA_LIMIT: Duration = Duration::from_secs(24 * 60 * 60);

pub(crate) struct YoutubeService(pub Result<YoutubeServices, String>);

pub(crate) struct YoutubeServices {
    api: YoutubeApi,
    account: Mutex<Account>,
    cancel_connect: watch::Sender<u64>,
    data_cleared: watch::Sender<u64>,
}

struct Account {
    oauth: YoutubeOAuth,
    channel: Option<String>,
    data_since: Option<SessionTime>,
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
    pub(crate) fn new(client_file: &Path, staging_dir: &Path) -> Result<Self, String> {
        Ok(Self {
            api: YoutubeApi::new().map_err(message)?,
            account: Mutex::new(Account {
                oauth: YoutubeOAuth::new(client_file, staging_dir)?,
                channel: None,
                data_since: None,
            }),
            cancel_connect: watch::channel(0).0,
            data_cleared: watch::channel(0).0,
        })
    }

    fn status(&self, account: &Account) -> YoutubeReply {
        let connected = account.oauth.has_session() && account.channel.is_some();
        YoutubeReply::Status(YoutubeStatus {
            can_connect: account.oauth.configured(),
            connected,
        })
    }

    fn clear_data(&self, library: &Library, account: &mut Account) {
        library.clear_youtube_account_data();
        if account.data_since.take().is_some() {
            self.data_cleared
                .send_modify(|revision| *revision = revision.wrapping_add(1));
        }
    }

    fn expire_data(&self, library: &Library, account: &mut Account) {
        if account
            .data_since
            .is_some_and(|time| time.elapsed() >= DATA_LIMIT)
        {
            self.clear_data(library, account);
        }
    }

    async fn check_access(
        &self,
        library: &Library,
        account: &mut Account,
    ) -> Result<Option<String>, String> {
        let unverified = account
            .oauth
            .verified
            .is_some_and(|time| time.elapsed() >= UNVERIFIED_LIMIT);
        if unverified {
            account.channel = None;
            self.clear_data(library, account);
        }
        let access = account.oauth.access_token().await;
        if matches!(access, Ok(None)) || !account.oauth.has_session() {
            account.channel = None;
            self.clear_data(library, account);
        }
        access
    }

    async fn authorize(
        &self,
        library: &Library,
        account: &mut Account,
    ) -> Result<Option<AuthorizedAccess>, String> {
        let Some(token) = self.check_access(library, account).await? else {
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
        account.data_since.get_or_insert_with(SessionTime::now);
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
            .map_err(message)?
            .is_some()
        {
            library
                .save_youtube_playlist(&mut playlist, Some(&user.channel))
                .map_err(|error| format!("YouTube 已保存更改，本地更新失败。{error}"))?;
        }
        Ok(YoutubeReply::Playlist(playlist))
    }

    async fn run(
        &self,
        request: YoutubeRequest,
        library: &Library,
    ) -> Result<YoutubeReply, String> {
        // Cancellation must bypass the account lock held by a pending browser login.
        if matches!(request, YoutubeRequest::CancelConnect) {
            self.cancel_connect
                .send_modify(|attempt| *attempt = attempt.wrapping_add(1));
            return Ok(YoutubeReply::Done);
        }
        if let YoutubeRequest::OpenPolicy { page } = &request {
            let url = match page {
                YoutubePolicyPage::Privacy => "https://xiaotwu.github.io/Nasti/privacy.html",
                YoutubePolicyPage::NastiTerms => "https://xiaotwu.github.io/Nasti/terms.html",
                YoutubePolicyPage::YoutubeTerms => "https://www.youtube.com/t/terms",
                YoutubePolicyPage::GooglePermissions => "https://myaccount.google.com/permissions",
            };
            launch_browser(&Url::parse(url).expect("static policy URL"))?;
            return Ok(YoutubeReply::Done);
        }
        let mut cancellation = self.cancel_connect.subscribe();
        let mut account = self.account.lock().await;
        self.expire_data(library, &mut account);
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
                account.oauth.connect(&mut cancellation).await?;
                account.channel = None;
                self.require_authorized(library, &mut account).await?;
                Ok(self.status(&account))
            }
            YoutubeRequest::CancelConnect | YoutubeRequest::OpenPolicy { .. } => {
                unreachable!("handled before the account lock")
            }
            YoutubeRequest::Disconnect => {
                account.channel = None;
                self.clear_data(library, &mut account);
                account.oauth.disconnect().await?;
                Ok(self.status(&account))
            }
            YoutubeRequest::Imported => {
                if account.oauth.has_session() {
                    let _ = self.check_access(library, &mut account).await;
                }
                Ok(YoutubeReply::Lists(
                    library
                        .youtube_imports(account.channel.as_deref())
                        .map_err(message)?,
                ))
            }
            YoutubeRequest::Forget { playlist_id: id } => {
                library.forget_youtube_playlist(&playlist_id(&id).map_err(message)?);
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
                    .map_err(message)?;
                Ok(YoutubeReply::Playlist(playlist))
            }
            YoutubeRequest::Load { playlist_id: id } => {
                let id = playlist_id(&id).map_err(message)?;
                let authorized = self.require_authorized(library, &mut account).await;
                let saved = library
                    .saved_youtube_playlist(&id, account.channel.as_deref())
                    .map_err(message)?;
                let user = match (authorized, &saved) {
                    (Ok(user), _) => user,
                    (Err(_), Some(saved)) => return Ok(YoutubeReply::Playlist(saved.clone())),
                    (Err(error), None) => return Err(error),
                };
                let fetched = self.api.playlist(&id, user.access()).await;
                match (fetched, saved) {
                    (Ok(mut playlist), saved) => {
                        if saved.is_some() {
                            library
                                .save_youtube_playlist(&mut playlist, Some(&user.channel))
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
                library.forget_youtube_playlist(&id);
                return Ok(YoutubeReply::Done);
            }
            _ => unreachable!("Only remote playlist mutations reach this handler"),
        };
        self.refresh_after_write(library, &id, user, account, change)
            .await
    }
}

pub(crate) fn start_session_checks(app: AppHandle) {
    let service = app.state::<YoutubeService>();
    let Ok(service) = &service.0 else { return };
    let mut changes = service.data_cleared.subscribe();
    tauri::async_runtime::spawn(async move {
        let mut ticks = tokio::time::interval(CHECK_TICK);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let state = app.state::<YoutubeService>();
        let Ok(service) = &state.0 else { return };
        let media = app.state::<crate::catalog_commands::MediaServices>();
        loop {
            tokio::select! {
                _ = ticks.tick() => {
                    // Regular UI requests share this lock. Do not queue work behind browser
                    // consent or concurrent writes, and never use a token pending revocation.
                    if let Ok(mut account) = service.account.try_lock() {
                        service.expire_data(&media.library, &mut account);
                        if account.oauth.has_session() {
                            let _ = service.check_access(&media.library, &mut account).await;
                        } else {
                            let _ = account.oauth.retry_revoke().await;
                        }
                    }
                },
                changed = changes.changed() => {
                    if changed.is_err() { break; }
                    let _ = app.emit_to("main", "youtube-session-cleared", ());
                }
            }
        }
    });
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
        "此版本尚未启用 Google 登录".into()
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
    async fn cancellation_bypasses_the_account_lock_and_stops_a_queued_login() {
        let folder = tempfile::tempdir().unwrap();
        let catalog_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../assets/authorized-media");
        let library = Library::open(
            &folder.path().join("library.sqlite3"),
            Arc::new(Catalog::load(&catalog_path).unwrap()),
        )
        .await
        .unwrap();
        let service =
            YoutubeServices::new(&folder.path().join("absent.json"), folder.path()).unwrap();
        let account = service.account.lock().await;
        let mut pending = std::pin::pin!(service.run(YoutubeRequest::Connect, &library));
        std::future::poll_fn(|context| {
            assert!(std::future::Future::poll(pending.as_mut(), context).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        let cancelled = tokio::time::timeout(
            Duration::from_secs(1),
            service.run(YoutubeRequest::CancelConnect, &library),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(matches!(cancelled, YoutubeReply::Done));
        drop(account);
        assert_eq!(pending.await.unwrap_err(), "已取消 Google 登录");
        assert!(!service.account.lock().await.oauth.has_session());
    }

    #[tokio::test]
    async fn signed_out_actions_expose_no_retained_youtube_data() {
        let folder = tempfile::tempdir().unwrap();
        let catalog_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../assets/authorized-media");
        let catalog = Arc::new(Catalog::load(&catalog_path).unwrap());
        let library = Library::open(&folder.path().join("library.sqlite3"), catalog)
            .await
            .unwrap();
        let service = YoutubeServices::new(
            &folder.path().join("youtube-oauth-client.json"),
            folder.path(),
        )
        .unwrap();
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
        library.save_youtube_playlist(&mut saved, None).unwrap();
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
        assert!(matches!(lists, YoutubeReply::Lists(lists) if lists.is_empty()));
        assert_eq!(
            service
                .run(
                    YoutubeRequest::Load {
                        playlist_id: saved_id.into()
                    },
                    &library
                )
                .await
                .unwrap_err(),
            authorization_error,
        );
    }

    #[tokio::test]
    async fn transient_refresh_preserves_grant_and_suspend_expires_data_before_network_returns() {
        use crate::youtube_oauth::tests::{connect_in_mock_browser, mock_google, FIRST_GRANT};
        let (mut oauth, google) = mock_google(vec![
            ("200 OK", FIRST_GRANT),
            ("503 Service Unavailable", r#"{"error":"invalid_grant"}"#),
            ("429 Too Many Requests", "{}"),
            ("200 OK", FIRST_GRANT),
            ("400 Bad Request", r#"{"error":"invalid_grant"}"#),
        ])
        .await;
        connect_in_mock_browser(&mut oauth, ("code", "synthetic"))
            .await
            .0
            .unwrap();

        let temp = tempfile::tempdir().unwrap();
        let catalog = Arc::new(
            Catalog::load(
                &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../assets/authorized-media"),
            )
            .unwrap(),
        );
        let library = Library::open(&temp.path().join("library.sqlite3"), catalog)
            .await
            .unwrap();
        let service = YoutubeServices::new(&temp.path().join("no-config"), temp.path()).unwrap();
        let mut account = service.account.lock().await;
        account.oauth = oauth;
        account.channel = Some("channel".into());
        account.data_since = Some(SessionTime::now());
        let mut playlist = YoutubePlaylist {
            summary: YoutubePlaylistSummary {
                id: "PLsynthetic".into(),
                title: "Synthetic private data".into(),
                description: String::new(),
                privacy: YoutubePrivacy::Private,
                can_edit: true,
            },
            items: vec![],
            imported_at: None,
        };
        library
            .save_youtube_playlist(&mut playlist, Some("channel"))
            .unwrap();
        let mut changed = service.data_cleared.subscribe();

        account.oauth.invalidate_access_token();
        assert!(service.check_access(&library, &mut account).await.is_err());
        assert!(account.oauth.has_session());
        assert!(library
            .saved_youtube_playlist("PLsynthetic", Some("channel"))
            .unwrap()
            .is_some());
        assert!(!changed.has_changed().unwrap());

        // Simulate a sleep that advances wall time without advancing the scheduler's clock.
        account.oauth.verified = Some(SessionTime::after_suspension(UNVERIFIED_LIMIT));
        assert!(service.check_access(&library, &mut account).await.is_err());
        assert!(account.oauth.has_session());
        assert!(account.channel.is_none());
        assert!(library.youtube_imports(Some("channel")).unwrap().is_empty());
        assert!(changed.has_changed().unwrap());
        changed.borrow_and_update();

        assert!(service
            .check_access(&library, &mut account)
            .await
            .unwrap()
            .is_some());
        account.channel = Some("channel".into());
        account.data_since = Some(SessionTime::after_suspension(DATA_LIMIT));
        library
            .save_youtube_playlist(&mut playlist, Some("channel"))
            .unwrap();
        service.expire_data(&library, &mut account);
        assert!(account.oauth.has_session());
        assert!(library.youtube_imports(Some("channel")).unwrap().is_empty());
        assert!(changed.has_changed().unwrap());
        changed.borrow_and_update();

        account.data_since = Some(SessionTime::now());
        library
            .save_youtube_playlist(&mut playlist, Some("channel"))
            .unwrap();
        account.oauth.invalidate_access_token();
        assert!(service
            .check_access(&library, &mut account)
            .await
            .unwrap()
            .is_none());
        assert!(!account.oauth.has_session());
        assert!(account.channel.is_none());
        assert!(library.youtube_imports(Some("channel")).unwrap().is_empty());
        assert!(changed.has_changed().unwrap());
        assert_eq!(google.await.unwrap().len(), 5);
    }
    #[tokio::test]
    #[ignore = "only run as the isolated child of process_exit_discards_youtube_data"]
    async fn session_process_fixture() {
        use crate::youtube_oauth::tests::{connect_in_mock_browser, mock_oauth};
        use std::io::{BufRead, Write};

        let data = std::path::PathBuf::from(std::env::var_os("NASTI_TEST_SESSION_DIR").unwrap());
        let catalog = Arc::new(
            Catalog::load(
                &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../assets/authorized-media"),
            )
            .unwrap(),
        );
        let library = Library::open(&data.join("library.sqlite3"), catalog)
            .await
            .unwrap();
        let service = YoutubeServices::new(&data.join("no-config"), &data).unwrap();
        let origin = Url::parse(&std::env::var("NASTI_TEST_AUTH_ORIGIN").unwrap()).unwrap();
        let mut oauth = mock_oauth(origin);
        if std::env::var_os("NASTI_TEST_READBACK").is_some() {
            assert!(library
                .youtube_imports(Some("fixture-channel"))
                .unwrap()
                .is_empty());
            assert!(oauth.access_token().await.unwrap().is_none());
            return;
        }

        connect_in_mock_browser(&mut oauth, ("code", "synthetic"))
            .await
            .0
            .unwrap();
        let mut account = service.account.lock().await;
        account.channel = Some("fixture-channel".into());
        account.oauth = oauth;
        let mut playlist = YoutubePlaylist {
            summary: YoutubePlaylistSummary {
                id: "PLsyntheticExit".into(),
                title: "synthetic-private-session-exit-marker".into(),
                description: String::new(),
                privacy: YoutubePrivacy::Private,
                can_edit: true,
            },
            items: Vec::new(),
            imported_at: None,
        };
        library
            .save_youtube_playlist(&mut playlist, account.channel.as_deref())
            .unwrap();
        assert!(account.oauth.has_session());
        std::io::stdout().write_all(b"SESSION_READY\n").unwrap();
        std::io::stdout().flush().unwrap();
        let mut input = String::new();
        std::io::stdin().lock().read_line(&mut input).unwrap();
        assert_eq!(input.trim(), "quit");
    }

    #[tokio::test]
    async fn process_exit_discards_youtube_data_after_normal_and_forced_exit() {
        use crate::youtube_oauth::tests::{mock_google, mock_origin, FIRST_GRANT};
        use std::process::Stdio;
        use tokio::{
            io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
            process::Command,
            time::timeout,
        };

        let fixture = || {
            let mut cmd = Command::new(std::env::current_exe().unwrap());
            cmd.args([
                "--ignored",
                "--exact",
                "youtube_commands::tests::session_process_fixture",
                "--nocapture",
            ])
            .kill_on_drop(true);
            cmd
        };
        for force in [false, true] {
            let data = tempfile::tempdir().unwrap();
            let (mock, google) = mock_google(vec![("200 OK", FIRST_GRANT), ("200 OK", "{}")]).await;
            let origin = mock_origin(&mock);
            let mut child = fixture()
                .env("NASTI_TEST_SESSION_DIR", data.path())
                .env("NASTI_TEST_AUTH_ORIGIN", origin.as_str())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
            timeout(Duration::from_secs(10), async {
                loop {
                    let line = output
                        .next_line()
                        .await
                        .unwrap()
                        .expect("fixture exited before ready");
                    if line.contains("SESSION_READY") {
                        break;
                    }
                }
            })
            .await
            .unwrap();
            if force {
                child.kill().await.unwrap();
            } else {
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(b"quit\n")
                    .await
                    .unwrap();
            }
            assert_eq!(
                timeout(Duration::from_secs(10), child.wait())
                    .await
                    .unwrap()
                    .unwrap()
                    .success(),
                !force
            );

            // Trigger the account-side revocation at the test service after the application dies.
            let response = reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap()
                .post(origin.join("account/revoke").unwrap())
                .form(&[("token", "first-refresh")])
                .send()
                .await
                .unwrap();
            assert!(response.status().is_success());
            let requests = google.await.unwrap();
            assert_eq!(requests[0].path, "/token");
            assert_eq!(requests[1].path, "/account/revoke");
            assert_eq!(requests[1].form["token"], "first-refresh");
            for name in [
                "library.sqlite3",
                "library.sqlite3-wal",
                "library.sqlite3-shm",
            ] {
                let bytes = std::fs::read(data.path().join(name)).unwrap_or_default();
                for marker in [
                    b"first-refresh".as_slice(),
                    b"fixture-channel",
                    b"synthetic-private-session-exit-marker",
                ] {
                    assert!(
                        !bytes.windows(marker.len()).any(|window| window == marker),
                        "{name}"
                    );
                }
            }
            let new_process = timeout(
                Duration::from_secs(10),
                fixture()
                    .env("NASTI_TEST_SESSION_DIR", data.path())
                    .env("NASTI_TEST_AUTH_ORIGIN", origin.as_str())
                    .env("NASTI_TEST_READBACK", "1")
                    .output(),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(
                new_process.status.success(),
                "{}",
                String::from_utf8_lossy(&new_process.stdout)
            );
        }
    }
}
