use crate::ui_locale::Language;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{ErrorKind, Read, Write};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use uuid::Uuid;

const CONSENT_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const OAUTH_API_ORIGIN: &str = "https://oauth2.googleapis.com/";
const PLAYLIST_SCOPE: &str = "https://www.googleapis.com/auth/youtube.force-ssl";
const CALLBACK_PATH: &str = "/";
const CALLBACK_LIMIT: usize = 8192;
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(180);
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);
const CLIENT_FILE_LIMIT: u64 = 64 * 1024;

pub(crate) struct YoutubeOAuth {
    client: Client,
    api_origin: Url,
    config: Option<DesktopClient>,
    use_system_keychain: bool,
    client_file: PathBuf,
    session_file: PathBuf,
    channel: Option<String>,
    legacy_checked: bool,
    local_session_seen: bool,
    refresh_token: Option<String>,
    access: Option<AccessToken>,
}

#[derive(Deserialize, Serialize)]
struct LocalGrant {
    client_id: String,
    refresh_token: String,
    channel_id: String,
}

#[derive(Deserialize)]
struct DesktopClient {
    #[serde(rename = "client_id")]
    id: String,
    #[serde(rename = "client_secret")]
    secret: Option<String>,
}

#[derive(Deserialize)]
struct DesktopClientDownload {
    installed: DesktopClient,
}

struct AccessToken {
    value: String,
    expires_at: Instant,
}

#[derive(Deserialize)]
struct TokenReply {
    access_token: String,
    token_type: String,
    refresh_token: Option<String>,
    expires_in: u64,
    scope: Option<String>,
}

#[derive(Deserialize)]
struct OAuthError {
    error: String,
}

impl YoutubeOAuth {
    pub(crate) fn new(use_system_keychain: bool, client_file: &Path) -> Result<Self, String> {
        let config = DesktopClient::load(client_file)?;
        let client = Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|_| "无法初始化 Google 授权连接".to_owned())?;
        let session_file = client_file.with_file_name("youtube-session.json");
        let local_session_seen = session_file.exists();
        let saved = read_local_grant(&session_file)?.filter(|grant| {
            config
                .as_ref()
                .is_some_and(|config| config.id == grant.client_id)
        });
        Ok(Self {
            client,
            api_origin: Url::parse(OAUTH_API_ORIGIN).expect("constant Google OAuth origin"),
            config,
            use_system_keychain: use_system_keychain && cfg!(target_os = "macos"),
            client_file: client_file.to_owned(),
            session_file,
            local_session_seen,
            legacy_checked: false,
            channel: saved.as_ref().map(|grant| grant.channel_id.clone()),
            refresh_token: saved.map(|grant| grant.refresh_token),
            access: None,
        })
    }

    pub(crate) fn configured(&self) -> bool {
        self.config.is_some()
    }

    pub(crate) fn has_session(&self) -> bool {
        self.refresh_token.is_some()
    }

    pub(crate) fn may_restore_legacy(&self) -> bool {
        self.configured()
            && self.use_system_keychain
            && !self.local_session_seen
            && !self.legacy_checked
            && !self.has_session()
    }

    pub(crate) fn channel(&self) -> Option<&str> {
        self.channel.as_deref()
    }

    pub(crate) fn configure(&mut self, json: &str) -> Result<(), String> {
        if self.has_session() || self.configured() {
            return Err("Google OAuth 客户端已配置".into());
        }
        let client = DesktopClient::parse(json.as_bytes())?;
        write_private(&self.client_file, json.as_bytes())?;
        self.config = Some(client);
        self.legacy_checked = false;
        Ok(())
    }

    pub(crate) async fn remember_channel(&mut self, channel: String) -> Result<(), String> {
        let config = self.config.as_ref().ok_or("缺少 Google OAuth 客户端")?;
        let refresh = self.refresh_token.as_ref().ok_or("缺少 Google 持久授权")?;
        self.save_grant(&config.id, refresh, &channel).await?;
        self.local_session_seen = true;
        self.channel = Some(channel);
        Ok(())
    }

    pub(crate) fn invalidate_access_token(&mut self) {
        self.access = None;
    }

    pub(crate) async fn connect(&mut self) -> Result<(), String> {
        self.connect_with(open_external).await
    }

    async fn connect_with(
        &mut self,
        open: impl FnOnce(&Url) -> Result<(), String>,
    ) -> Result<(), String> {
        let config = self
            .config
            .as_ref()
            .ok_or("尚未配置 Google 桌面 OAuth 客户端")?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| "无法打开本机授权回调".to_owned())?;
        let address = listener
            .local_addr()
            .map_err(|_| "无法读取本机授权回调地址".to_owned())?;
        let redirect = Url::parse(&format!("http://{address}{CALLBACK_PATH}")).unwrap();
        let verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let state = Uuid::new_v4().simple().to_string();
        let browser_url = consent_url(&config.id, &redirect, &state, &verifier);
        open(&browser_url)?;
        let code = receive_code(listener, &redirect, &state, Language::current()).await?;
        let reply = self
            .request_tokens(
                config,
                &[
                    ("grant_type", "authorization_code"),
                    ("code", &code),
                    ("code_verifier", &verifier),
                    ("redirect_uri", redirect.as_str()),
                ],
            )
            .await?
            .ok_or("Google 授权码已过期，请重新连接")?;
        let access = AccessToken::from_reply(&reply)?;
        let refresh = reply
            .refresh_token
            .filter(|value| !value.is_empty())
            .ok_or("Google 未返回持久授权，请重新同意歌单权限")?;
        self.refresh_token = Some(refresh);
        self.channel = None;
        self.legacy_checked = true;
        self.access = Some(access);
        Ok(())
    }

    pub(crate) async fn access_token(&mut self) -> Result<Option<String>, String> {
        self.restore_grant().await?;
        if let Some(access) = self
            .access
            .as_ref()
            .filter(|token| token.expires_at > Instant::now())
        {
            return Ok(Some(access.value.clone()));
        }
        let (Some(config), Some(refresh)) = (self.config.as_ref(), self.refresh_token.as_ref())
        else {
            return Ok(None);
        };
        let reply = self
            .request_tokens(
                config,
                &[("grant_type", "refresh_token"), ("refresh_token", refresh)],
            )
            .await?;
        let Some(reply) = reply else {
            self.access = None;

            return Ok(None);
        };
        let access = AccessToken::from_reply(&reply)?;
        let value = access.value.clone();
        self.access = Some(access);
        if let Some(rotated) = reply.refresh_token.filter(|value| !value.is_empty()) {
            if let Some(channel) = self.channel.as_deref() {
                self.save_grant(&config.id, &rotated, channel).await?;
            }
            self.refresh_token = Some(rotated);
        }
        Ok(Some(value))
    }

    pub(crate) async fn disconnect(&mut self) -> Result<(), String> {
        self.restore_grant().await?;
        let Some(config) = self.config.as_ref() else {
            return Ok(());
        };
        let Some(refresh) = self.refresh_token.as_deref() else {
            return self.delete_refresh_token(&config.id).await;
        };
        let response = self
            .client
            .post(
                self.api_origin
                    .join("revoke")
                    .expect("fixed revocation path"),
            )
            .form(&[("token", refresh)])
            .send()
            .await
            .map_err(|_| "暂时无法联系 Google 撤销授权，请重试".to_owned())?;
        let status = response.status();
        let already_revoked = status == StatusCode::BAD_REQUEST
            && response.json::<OAuthError>().await.is_ok_and(|reply| {
                matches!(reply.error.as_str(), "invalid_token" | "invalid_grant")
            });
        if !status.is_success() && !already_revoked {
            return Err("Google 未能确认授权已撤销，请重试".into());
        }
        // Keep a tombstone so that an inaccessible or old keychain entry cannot be restored.
        write_private(&self.session_file, b"null")?;
        self.local_session_seen = true;
        self.access = None;
        self.refresh_token = None;
        self.channel = None;
        self.legacy_checked = true;
        self.delete_refresh_token(&config.id)
            .await
            .map_err(|_| "Google 授权已撤销。无法移除钥匙串条目，请重试移除".to_owned())
    }

    async fn restore_grant(&mut self) -> Result<(), String> {
        if self.refresh_token.is_some() || self.local_session_seen || self.legacy_checked {
            return Ok(());
        }
        if let Some(config) = self.config.as_ref().filter(|_| self.use_system_keychain) {
            let id = config.id.clone();
            if let Ok(Ok(token)) = tokio::task::spawn_blocking(move || keychain::load(&id)).await {
                self.refresh_token = token;
                self.legacy_checked = true;
            }
        }
        Ok(())
    }

    async fn save_grant(&self, client_id: &str, token: &str, channel: &str) -> Result<(), String> {
        let data = serde_json::to_vec(&LocalGrant {
            client_id: client_id.into(),
            refresh_token: token.into(),
            channel_id: channel.into(),
        })
        .map_err(|_| "无法编码 Google 会话".to_owned())?;
        write_private(&self.session_file, &data)?;
        if self.use_system_keychain {
            let id = client_id.to_owned();
            let token = token.to_owned();
            // Local persistence already succeeded. A locked or re-signed keychain is recoverable.
            let _ = tokio::task::spawn_blocking(move || keychain::save(&id, &token)).await;
        }
        Ok(())
    }

    async fn delete_refresh_token(&self, client_id: &str) -> Result<(), String> {
        if self.use_system_keychain {
            let id = client_id.to_owned();
            tokio::task::spawn_blocking(move || keychain::delete(&id))
                .await
                .map_err(|_| "无法移除 Google 授权".to_owned())??;
        }
        Ok(())
    }

    async fn request_tokens(
        &self,
        config: &DesktopClient,
        grant: &[(&str, &str)],
    ) -> Result<Option<TokenReply>, String> {
        let mut form = grant.to_vec();
        form.push(("client_id", config.id.as_str()));
        if let Some(secret) = config.secret.as_deref() {
            form.push(("client_secret", secret));
        }
        let response = self
            .client
            .post(self.api_origin.join("token").expect("fixed token path"))
            .form(&form)
            .send()
            .await
            .map_err(|_| "暂时无法连接 Google 授权服务".to_owned())?;
        let status = response.status();
        if status.is_success() {
            return response
                .json::<TokenReply>()
                .await
                .map(Some)
                .map_err(|_| "Google 返回的授权信息无法读取".to_owned());
        }
        let error = response.json::<OAuthError>().await.ok();
        match error.as_ref().map(|failure| failure.error.as_str()) {
            Some("invalid_grant") if status == StatusCode::BAD_REQUEST => Ok(None),
            Some("invalid_client" | "unauthorized_client") => {
                Err("请检查 Google 桌面 OAuth 客户端配置".into())
            }
            _ => Err(format!("Google 授权请求失败（HTTP {}）", status.as_u16())),
        }
    }
}

fn read_local_grant(path: &Path) -> Result<Option<LocalGrant>, String> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("无法读取本地 Google 会话".into()),
    };
    let metadata = file.metadata().map_err(|_| "无法读取会话权限")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("本地 Google 会话文件权限必须为 0600".into());
        }
    }
    if !metadata.is_file() || metadata.len() > CLIENT_FILE_LIMIT {
        return Err("本地 Google 会话文件无效".into());
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(CLIENT_FILE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取本地 Google 会话")?;
    let grant: Option<LocalGrant> =
        serde_json::from_slice(&bytes).map_err(|_| "本地 Google 会话文件无效")?;
    Ok(grant.filter(|grant| !grant.refresh_token.is_empty() && !grant.channel_id.is_empty()))
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("无法定位应用数据目录")?;
    let mut file =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| "无法创建受保护的 Google 配置")?;
    file.write_all(bytes)
        .and_then(|()| file.as_file().sync_all())
        .map_err(|_| "无法保存 Google 配置")?;
    file.persist(path).map_err(|_| "无法更新 Google 配置")?;
    Ok(())
}

impl DesktopClient {
    fn load(default_path: &Path) -> Result<Option<Self>, String> {
        if let Some(id) = env_value("NASTI_YOUTUBE_CLIENT_ID") {
            return Ok(Some(Self {
                id,
                secret: env_value("NASTI_YOUTUBE_CLIENT_SECRET"),
            }));
        }
        match std::env::var_os("NASTI_YOUTUBE_CLIENT_FILE").filter(|path| !path.is_empty()) {
            Some(path) => Self::read_file(Path::new(&path), true),
            None => Ok(Self::read_file(default_path, false)?.or_else(|| {
                let id = option_env!("NASTI_APP_YOUTUBE_CLIENT_ID")?.trim();
                let secret = option_env!("NASTI_APP_YOUTUBE_CLIENT_SECRET")?.trim();
                if id.is_empty() || secret.is_empty() { return None; }
                Some(Self { id: id.to_owned(), secret: Some(secret.to_owned()) })
            })),
        }
    }

    fn read_file(path: &Path, required: bool) -> Result<Option<Self>, String> {
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if !required && error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err("找不到 Google 桌面 OAuth 客户端 JSON".into());
            }
            Err(_) => return Err("无法读取 Google 桌面 OAuth 客户端 JSON".into()),
        };
        if !metadata.is_file() || metadata.len() > CLIENT_FILE_LIMIT {
            return Err("Google 桌面 OAuth 客户端 JSON 文件无效".into());
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .and_then(|file| file.take(CLIENT_FILE_LIMIT + 1).read_to_end(&mut bytes))
            .map_err(|_| "无法读取 Google 桌面 OAuth 客户端 JSON".to_owned())?;
        if bytes.len() as u64 > CLIENT_FILE_LIMIT {
            return Err("Google 桌面 OAuth 客户端 JSON 文件无效".into());
        }
        Self::parse(&bytes).map(Some)
    }

    fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() as u64 > CLIENT_FILE_LIMIT {
            return Err("Google 桌面 OAuth 客户端 JSON 文件无效".into());
        }
        let DesktopClientDownload {
            installed: mut client,
        } = serde_json::from_slice(bytes)
            .map_err(|_| "请选择从 Google Cloud 下载的 Desktop app OAuth 客户端 JSON".to_owned())?;
        client.id = client.id.trim().to_owned();
        if client.id.is_empty() {
            return Err("Google 桌面 OAuth 客户端 JSON 缺少客户端 ID".into());
        }
        client.secret = client
            .secret
            .map(|secret| secret.trim().to_owned())
            .filter(|secret| !secret.is_empty());
        Ok(client)
    }
}

impl AccessToken {
    fn from_reply(reply: &TokenReply) -> Result<Self, String> {
        if reply.access_token.is_empty()
            || !reply.token_type.eq_ignore_ascii_case("Bearer")
            || reply.scope.as_ref().is_some_and(|scopes| {
                !scopes.split_ascii_whitespace().any(|scope| {
                    matches!(
                        scope,
                        PLAYLIST_SCOPE | "https://www.googleapis.com/auth/youtube"
                    )
                })
            })
        {
            return Err("Google 尚未授予歌单权限，请重新连接".into());
        }
        let lifetime = Duration::from_secs(reply.expires_in.saturating_sub(30).min(86_400));
        Ok(Self {
            value: reply.access_token.clone(),
            expires_at: Instant::now() + lifetime,
        })
    }
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn consent_url(client_id: &str, redirect: &Url, state: &str, verifier: &str) -> Url {
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url = Url::parse(CONSENT_URL).unwrap();
    url.query_pairs_mut().extend_pairs([
        ("client_id", client_id),
        ("redirect_uri", redirect.as_str()),
        ("scope", PLAYLIST_SCOPE),
        ("response_type", "code"),
        ("access_type", "offline"),
        ("prompt", "consent select_account"),
        ("state", state),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
    ]);
    url
}

async fn receive_code(
    listener: TcpListener,
    redirect: &Url,
    state: &str,
    language: Language,
) -> Result<String, String> {
    tokio::time::timeout(CALLBACK_TIMEOUT, async {
        loop {
            let (mut stream, _) = listener.accept().await.map_err(|_| "本机授权回调已关闭".to_owned())?;
            let callback = tokio::time::timeout(CONNECTION_TIMEOUT, read_callback(&mut stream))
                .await.ok().flatten().and_then(|line| parse_callback(&line, redirect, state));
            let (status, message) = match &callback {
                Some(Ok(_)) => ("200 OK", language.choose("授权已返回 Nasti。可关闭此页。", "Authorization returned to Nasti. You can close this page.")),
                Some(Err(_)) => ("200 OK", language.choose("未授予 Nasti 歌单权限。可关闭此页。", "Nasti was not granted playlist access. You can close this page.")),
                None => ("404 Not Found", language.choose("无效的授权回调。", "Invalid authorization callback.")),
            };
            let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{message}", message.len());
            let _ = tokio::time::timeout(CONNECTION_TIMEOUT, stream.write_all(response.as_bytes())).await;
            if let Some(result) = callback {
                return result;
            }
        }
    }).await.map_err(|_| "等待 Google 授权超时，请重试".to_owned())?
}

async fn read_callback(stream: &mut TcpStream) -> Option<String> {
    let mut buffer = [0; CALLBACK_LIMIT];
    let mut used = 0;
    while used < buffer.len() {
        let count = stream.read(&mut buffer[used..]).await.ok()?;
        if count == 0 {
            return None;
        }
        used += count;
        let received = &buffer[..used];
        if received.windows(4).any(|window| window == b"\r\n\r\n") {
            let first = received.iter().position(|byte| *byte == b'\r')?;
            return std::str::from_utf8(&received[..first])
                .ok()
                .map(str::to_owned);
        }
    }
    None
}

fn parse_callback(line: &str, redirect: &Url, state: &str) -> Option<Result<String, String>> {
    let mut request = line.split_ascii_whitespace();
    if request.next()? != "GET" {
        return None;
    }
    let path = request.next()?;
    if path.split_once('?')?.0 != CALLBACK_PATH {
        return None;
    }
    let url = redirect.join(path).ok()?;
    let value = |key: &str| {
        let mut matches = url
            .query_pairs()
            .filter(|(name, _)| name == key)
            .map(|(_, value)| value);
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    };
    if value("state").as_deref() != Some(state) {
        return None;
    }
    if url.query_pairs().any(|(name, _)| name == "error") {
        return Some(Err("未授予 Google 歌单权限".into()));
    }
    value("code")
        .filter(|code| !code.is_empty())
        .map(|code| Ok(code.into_owned()))
}

pub(crate) fn open_external(url: &Url) -> Result<(), String> {
    let known_page = match url.host_str() {
        Some("accounts.google.com") => url.path() == "/o/oauth2/v2/auth",
        Some("www.youtube.com" | "youtube.com" | "music.youtube.com") => {
            matches!(url.path(), "/watch" | "/playlist")
        }
        _ => false,
    };
    if url.scheme() != "https"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || !known_page
    {
        return Err("无法打开此 YouTube 链接".into());
    }
    launch_browser(url)
}

#[cfg(target_os = "macos")]
fn launch_browser(url: &Url) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let result = Command::new("/usr/bin/open")
        .arg(url.as_str())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    result
        .is_ok_and(|status| status.success())
        .then_some(())
        .ok_or_else(|| "无法打开系统浏览器".to_owned())
}

#[cfg(target_os = "windows")]
fn launch_browser(url: &Url) -> Result<(), String> {
    use std::ptr::null;
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let operation = [b'o' as u16, b'p' as u16, b'e' as u16, b'n' as u16, 0];
    let target: Vec<u16> = url.as_str().encode_utf16().chain([0]).collect();
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            target.as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        )
    };
    ((result as isize) > 32)
        .then_some(())
        .ok_or_else(|| "无法打开系统浏览器".to_owned())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn launch_browser(_: &Url) -> Result<(), String> {
    Err("此宿主尚未接入系统浏览器".into())
}

#[cfg(target_os = "macos")]
mod keychain {
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };

    const SERVICE: &str = "dev.nasti.terminal.youtube-oauth";
    const ITEM_NOT_FOUND: i32 = -25300;

    pub(super) fn load(client: &str) -> Result<Option<String>, String> {
        match get_generic_password(SERVICE, client) {
            Ok(value) => String::from_utf8(value)
                .map(Some)
                .map_err(|_| "钥匙串中的 Google 授权记录无效".into()),
            Err(error) if error.code() == ITEM_NOT_FOUND => Ok(None),
            Err(_) => Err("无法读取钥匙串中的 Google 授权".into()),
        }
    }

    pub(super) fn save(client: &str, refresh: &str) -> Result<(), String> {
        set_generic_password(SERVICE, client, refresh.as_bytes())
            .map_err(|_| "无法将 Google 授权保存在钥匙串".into())
    }

    pub(super) fn delete(client: &str) -> Result<(), String> {
        match delete_generic_password(SERVICE, client) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == ITEM_NOT_FOUND => Ok(()),
            Err(_) => Err("无法移除钥匙串中的 Google 授权".into()),
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod keychain {
    pub(super) fn load(_: &str) -> Result<Option<String>, String> {
        Ok(None)
    }
    pub(super) fn save(_: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    pub(super) fn delete(_: &str) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tokio::task::JoinHandle;

    const TEST_TIMEOUT: Duration = Duration::from_secs(5);
    const FIRST_GRANT: &str = r#"{"token_type":"Bearer","access_token":"first-access","refresh_token":"first-refresh","expires_in":3600}"#;

    struct MockRequest {
        path: String,
        form: HashMap<String, String>,
    }

    async fn mock_google(
        replies: Vec<(&'static str, &'static str)>,
    ) -> (YoutubeOAuth, JoinHandle<Vec<MockRequest>>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let api_origin =
            Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, body) in replies {
                let (mut stream, _) = listener.accept().await.unwrap();
                requests.push(read_mock_request(&mut stream).await);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
            requests
        });
        let oauth = YoutubeOAuth {
            client: Client::builder()
                .no_proxy()
                .timeout(TEST_TIMEOUT)
                .build()
                .unwrap(),
            api_origin,
            config: Some(DesktopClient {
                id: "desktop-client".into(),
                secret: Some("desktop-secret".into()),
            }),
            use_system_keychain: false,
            client_file: PathBuf::new(),
            session_file: PathBuf::new(),
            local_session_seen: false,
            legacy_checked: false,
            channel: None,
            refresh_token: None,
            access: None,
        };
        (oauth, server)
    }

    async fn read_mock_request(stream: &mut TcpStream) -> MockRequest {
        let mut received = Vec::new();
        let mut buffer = [0; 4096];
        let header_end = loop {
            let count = stream.read(&mut buffer).await.unwrap();
            assert!(count > 0);
            received.extend_from_slice(&buffer[..count]);
            if let Some(position) = received.windows(4).position(|part| part == b"\r\n\r\n") {
                break position;
            }
        };
        let headers = std::str::from_utf8(&received[..header_end]).unwrap();
        let mut lines = headers.lines();
        let mut request = lines.next().unwrap().split_ascii_whitespace();
        assert_eq!(request.next(), Some("POST"));
        let path = request.next().unwrap().to_owned();
        let length: usize = lines
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("Content-Length")
                    .then(|| value.trim().parse().unwrap())
            })
            .unwrap();
        let body_start = header_end + 4;
        while received.len() < body_start + length {
            let count = stream.read(&mut buffer).await.unwrap();
            assert!(count > 0);
            received.extend_from_slice(&buffer[..count]);
        }
        let body = std::str::from_utf8(&received[body_start..body_start + length]).unwrap();
        let url = Url::parse(&format!("http://127.0.0.1/?{body}")).unwrap();
        let form = url.query_pairs().into_owned().collect();
        MockRequest { path, form }
    }

    async fn connect_in_mock_browser(
        oauth: &mut YoutubeOAuth,
        response: (&'static str, &'static str),
    ) -> (Result<(), String>, HashMap<String, String>) {
        let mut browser = None;
        let result = tokio::time::timeout(
            TEST_TIMEOUT,
            oauth.connect_with(|consent| {
                assert_eq!(consent.host_str(), Some("accounts.google.com"));
                assert_eq!(consent.path(), "/o/oauth2/v2/auth");
                let params: HashMap<_, _> = consent.query_pairs().into_owned().collect();
                assert_eq!(params["client_id"], "desktop-client");
                assert_eq!(params["scope"], PLAYLIST_SCOPE);
                assert_eq!(params["access_type"], "offline");
                assert_eq!(params["code_challenge_method"], "S256");
                assert!(!params.contains_key("client_secret"));
                let mut callback = Url::parse(&params["redirect_uri"]).unwrap();
                assert_eq!(callback.host_str(), Some("127.0.0.1"));
                callback
                    .query_pairs_mut()
                    .append_pair("state", &params["state"])
                    .append_pair(response.0, response.1);
                let task = tokio::spawn(async move {
                    let reply = Client::builder()
                        .no_proxy()
                        .timeout(TEST_TIMEOUT)
                        .build()
                        .unwrap()
                        .get(callback)
                        .send()
                        .await
                        .unwrap();
                    assert_eq!(reply.status(), StatusCode::OK);
                    assert_eq!(reply.headers()["Cache-Control"], "no-store");
                    assert!(!reply.text().await.unwrap().contains(response.1));
                });
                browser = Some((params, task));
                Ok(())
            }),
        )
        .await
        .unwrap();
        let (params, task) = browser.unwrap();
        task.await.unwrap();
        (result, params)
    }

    #[test]
    fn reads_google_desktop_client_download() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("youtube-oauth-client.json");
        std::fs::write(
            &path,
            r#"{"installed":{"client_id":"test.apps.googleusercontent.com","client_secret":"test-secret","token_uri":"https://ignored.example/token"}}"#,
        )
        .unwrap();
        let client = DesktopClient::read_file(&path, false).unwrap().unwrap();
        assert_eq!(client.id, "test.apps.googleusercontent.com");
        assert_eq!(client.secret.as_deref(), Some("test-secret"));

        std::fs::write(&path, r#"{"web":{"client_id":"wrong-type"}}"#).unwrap();
        assert!(DesktopClient::read_file(&path, false).is_err());
        let absent = directory.path().join("absent.json");
        assert!(DesktopClient::read_file(&absent, false).unwrap().is_none());
        assert!(DesktopClient::read_file(&absent, true).is_err());
    }

    #[test]
    fn consent_derives_pkce_challenge_without_sending_the_verifier() {
        let redirect = Url::parse("http://127.0.0.1:49152/").unwrap();
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let url = consent_url("client-id", &redirect, "expected-state", verifier);
        let params = url
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            params["code_challenge"],
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(params["state"], "expected-state");
        assert_eq!(params["redirect_uri"], redirect.as_str());
        assert!(!url.as_str().contains(verifier));
    }

    #[tokio::test]
    async fn loopback_checks_state_before_accepting_consent() {
        async fn request(address: std::net::SocketAddr, query: &str) -> String {
            let mut stream = TcpStream::connect(address).await.unwrap();
            stream
                .write_all(
                    format!("GET {CALLBACK_PATH}?{query} HTTP/1.1\r\nHost: {address}\r\n\r\n")
                        .as_bytes(),
                )
                .await
                .unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            response
        }
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let redirect = Url::parse(&format!("http://{address}{CALLBACK_PATH}")).unwrap();
        let receiver = tokio::spawn(async move {
            receive_code(listener, &redirect, "expected", Language::English).await
        });
        assert!(request(address, "state=wrong&code=never-use")
            .await
            .starts_with("HTTP/1.1 404"));
        assert!(
            request(address, "state=expected&state=duplicate&code=never-use")
                .await
                .starts_with("HTTP/1.1 404")
        );
        let response = request(address, "state=expected&code=return%2Fvalue").await;
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("Authorization returned to Nasti"));
        assert!(!response.contains("return/value"));
        assert_eq!(receiver.await.unwrap().unwrap(), "return/value");
    }

    #[tokio::test]
    async fn oauth_grant_refresh_and_revoke_follow_the_browser_and_google_contract() {
        let (mut oauth, google) = mock_google(vec![
            ("200 OK", FIRST_GRANT),
            ("200 OK", r#"{"token_type":"Bearer","access_token":"next-access","refresh_token":"next-refresh","expires_in":3600}"#),
            ("503 Service Unavailable", "{}"),
            ("200 OK", "{}"),
        ]).await;
        let directory = tempfile::tempdir().unwrap();
        oauth.session_file = directory.path().join("youtube-session.json");
        let (result, consent) =
            connect_in_mock_browser(&mut oauth, ("code", "granted/code+")).await;
        result.unwrap();
        assert!(oauth.has_session());
        assert_eq!(
            oauth.access_token().await.unwrap().as_deref(),
            Some("first-access")
        );

        oauth.remember_channel("channel-a".into()).await.unwrap();
        assert_eq!(
            read_local_grant(&oauth.session_file)
                .unwrap()
                .unwrap()
                .channel_id,
            "channel-a"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&oauth.session_file)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        let client_path = directory.path().join("youtube-oauth-client.json");
        write_private(
            &client_path,
            br#"{"installed":{"client_id":"desktop-client","client_secret":"desktop-secret"}}"#,
        )
        .unwrap();
        let restarted = YoutubeOAuth::new(false, &client_path).unwrap();
        assert!(restarted.has_session());
        assert_eq!(restarted.channel(), Some("channel-a"));
        assert!(restarted.access.is_none());
        oauth.invalidate_access_token();
        assert_eq!(
            oauth.access_token().await.unwrap().as_deref(),
            Some("next-access")
        );
        assert_eq!(
            read_local_grant(&oauth.session_file)
                .unwrap()
                .unwrap()
                .refresh_token,
            "next-refresh"
        );
        assert!(oauth.disconnect().await.is_err());
        assert!(oauth.has_session());
        oauth.disconnect().await.unwrap();
        assert!(read_local_grant(&oauth.session_file).unwrap().is_none());
        assert!(!oauth.has_session());
        assert!(oauth.access_token().await.unwrap().is_none());

        let requests = tokio::time::timeout(TEST_TIMEOUT, google)
            .await
            .unwrap()
            .unwrap();
        let grant = &requests[0];
        assert_eq!(grant.path, "/token");
        assert_eq!(grant.form["grant_type"], "authorization_code");
        assert_eq!(grant.form["code"], "granted/code+");
        assert_eq!(grant.form["redirect_uri"], consent["redirect_uri"]);
        assert_eq!(grant.form["client_id"], "desktop-client");
        assert_eq!(grant.form["client_secret"], "desktop-secret");
        let verifier = &grant.form["code_verifier"];
        assert!((43..=128).contains(&verifier.len()));
        assert_eq!(
            consent["code_challenge"],
            URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
        );
        assert_eq!(requests[1].path, "/token");
        assert_eq!(requests[1].form["grant_type"], "refresh_token");
        assert_eq!(requests[1].form["refresh_token"], "first-refresh");
        for revoke in &requests[2..] {
            assert_eq!(revoke.path, "/revoke");
            assert_eq!(revoke.form["token"], "next-refresh");
        }
    }

    #[tokio::test]
    async fn oauth_denial_and_failed_refresh_do_not_destroy_a_persisted_grant() {
        let (mut oauth, google) = mock_google(vec![
            ("200 OK", FIRST_GRANT),
            ("503 Service Unavailable", "{}"),
            ("400 Bad Request", r#"{"error":"invalid_grant"}"#),
            (
                "200 OK",
                r#"{"token_type":"Bearer","access_token":"recovered-access","expires_in":3600}"#,
            ),
        ])
        .await;
        let (result, _) = connect_in_mock_browser(&mut oauth, ("error", "access_denied")).await;
        assert_eq!(result.unwrap_err(), "未授予 Google 歌单权限");
        assert!(oauth.access_token().await.unwrap().is_none());
        let (result, _) = connect_in_mock_browser(&mut oauth, ("code", "consent-code")).await;
        result.unwrap();
        let directory = tempfile::tempdir().unwrap();
        oauth.session_file = directory.path().join("youtube-session.json");
        oauth.remember_channel("channel-a".into()).await.unwrap();
        oauth.invalidate_access_token();
        assert!(oauth.access_token().await.is_err());
        assert!(oauth.access_token().await.unwrap().is_none());
        assert!(oauth.has_session());
        assert_eq!(oauth.channel(), Some("channel-a"));
        assert_eq!(
            read_local_grant(&oauth.session_file)
                .unwrap()
                .unwrap()
                .refresh_token,
            "first-refresh"
        );
        assert_eq!(
            oauth.access_token().await.unwrap().as_deref(),
            Some("recovered-access")
        );
        let requests = tokio::time::timeout(TEST_TIMEOUT, google)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(requests[0].form["code"], "consent-code");
        assert_eq!(requests[1].form["refresh_token"], "first-refresh");
    }
}
