use crate::models::{AppUpdateInstallArgs, AppUpdateStatus};

#[cfg(target_os = "macos")]
use {
    crate::runtime_storage::RuntimeStorage,
    std::sync::Arc,
    tauri::{AppHandle, State},
    tokio::sync::Mutex,
};

#[cfg(target_os = "macos")]
#[derive(Default)]
pub(crate) struct AppUpdater {
    // A successful bundle replacement makes another update unsafe until relaunch.
    installed: Arc<Mutex<bool>>,
}

#[cfg(target_os = "macos")]
const UPDATE_NEEDS_RESTART: &str = "更新已安装，请重新打开 Nasti";

#[cfg(target_os = "macos")]
#[tauri::command]
pub(crate) async fn app_update_check(
    app: AppHandle,
    updater: State<'_, AppUpdater>,
) -> Result<AppUpdateStatus, String> {
    let operation = updater
        .installed
        .try_lock()
        .map_err(|_| "Nasti 正在检查或安装更新".to_owned())?;
    if *operation {
        return Err(UPDATE_NEEDS_RESTART.into());
    }
    let current_version = &app.package_info().version;
    let client = github::client()?;
    let available_version = match github::discover(&client, current_version).await? {
        Some(release) => Some(github::resolve(&app, &client, &release).await?.version),
        None => None,
    };
    Ok(AppUpdateStatus {
        current_version: current_version.to_string(),
        available_version,
    })
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub(crate) async fn app_update_install(
    app: AppHandle,
    updater: State<'_, AppUpdater>,
    storage: State<'_, RuntimeStorage>,
    args: AppUpdateInstallArgs,
) -> Result<(), String> {
    let mut operation = updater
        .installed
        .clone()
        .try_lock_owned()
        .map_err(|_| "Nasti 正在检查或安装更新".to_owned())?;
    if *operation {
        return Err(UPDATE_NEEDS_RESTART.into());
    }
    let client = github::client()?;
    let release = github::discover(&client, &app.package_info().version)
        .await?
        .filter(|release| release.version.to_string() == args.version)
        .ok_or("可用更新已变更，请重新检查更新")?;
    let update = github::resolve(&app, &client, &release).await?;
    let archive = github::download(&client, &release).await?;
    let version = update.version.clone();
    let updater_dir = storage.updater_dir.clone();
    // Keep the operation guard during the blocking install even if the WebView closes.
    tauri::async_runtime::spawn_blocking(move || {
        update
            .verify(&archive)
            .map_err(|_| "更新包的签名或签名版本无效".to_owned())?;
        let bundle = crate::app_update_install::install_verified(&archive, &version, &updater_dir)?;
        *operation = true;
        crate::app_update_install::relaunch_after_update(&app, &bundle)
    })
    .await
    .map_err(|_| "无法完成 Nasti 更新安装".to_owned())?
}

#[cfg(not(target_os = "macos"))]
const UPDATES_UNAVAILABLE: &str = "应用内更新当前仅支持 Apple Silicon Mac";

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub(crate) async fn app_update_check() -> Result<AppUpdateStatus, String> {
    Err(UPDATES_UNAVAILABLE.into())
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub(crate) async fn app_update_install(args: AppUpdateInstallArgs) -> Result<(), String> {
    let _ = args;
    Err(UPDATES_UNAVAILABLE.into())
}

#[cfg(target_os = "macos")]
mod github {
    use reqwest::{redirect::Policy, Client, RequestBuilder, Url};
    use semver::Version;
    use serde::Deserialize;
    use serde_json::Value;
    use std::time::Duration;
    use tauri_plugin_updater::{Update, UpdaterExt};

    const RELEASES_ENDPOINT: &str = "https://api.github.com/repos/xiaotwu/Nasti/releases";
    const DOWNLOAD_BASE: &str = "https://github.com/xiaotwu/Nasti/releases/download";
    const MANIFEST_NAME: &str = "latest.json";
    const ARCHIVE_NAME: &str = "Nasti.app.tar.gz";
    const UPDATER_TARGET: &str = "darwin-aarch64";
    const RELEASES_PER_PAGE: usize = 30;
    const MAX_RELEASE_PAGES: usize = 4;
    const RELEASE_LIST_LIMIT: usize = 3 * 1024 * 1024;
    const MANIFEST_LIMIT: usize = 64 * 1024;
    const ARCHIVE_LIMIT: usize = 96 * 1024 * 1024;

    #[derive(Deserialize)]
    struct GithubRelease {
        tag_name: String,
        draft: bool,
        prerelease: bool,
        assets: Vec<GithubAsset>,
    }

    #[derive(Deserialize)]
    struct GithubAsset {
        name: String,
        state: String,
        size: usize,
        browser_download_url: String,
    }

    pub(super) struct Release {
        pub(super) version: Version,
        manifest: Url,
        manifest_size: usize,
        archive: Url,
        archive_size: usize,
    }

    pub(super) fn client() -> Result<Client, String> {
        Client::builder()
            .user_agent("Nasti-Updater")
            .https_only(true)
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(90))
            .redirect(Policy::custom(|attempt| {
                if attempt.previous().len() <= 4
                    && attempt
                        .previous()
                        .first()
                        .is_some_and(|first| download_redirect_allowed(first, attempt.url()))
                {
                    attempt.follow()
                } else {
                    attempt.error("Unsupported Nasti release redirect")
                }
            }))
            .build()
            .map_err(|_| "无法初始化更新连接".to_owned())
    }

    fn download_redirect_allowed(initial: &Url, target: &Url) -> bool {
        initial.host_str() == Some("github.com")
            && target.scheme() == "https"
            && target.port().is_none()
            && target.username().is_empty()
            && target.password().is_none()
            && target.fragment().is_none()
            && matches!(
                target.host_str(),
                Some("release-assets.githubusercontent.com" | "objects.githubusercontent.com")
            )
    }

    pub(super) async fn discover(
        client: &Client,
        current: &Version,
    ) -> Result<Option<Release>, String> {
        if !cfg!(target_arch = "aarch64") {
            return Err("应用内更新当前仅支持 Apple Silicon Mac".into());
        }
        let mut remaining = RELEASE_LIST_LIMIT;
        let mut latest = None;
        for page in 1..=MAX_RELEASE_PAGES {
            let mut url = Url::parse(RELEASES_ENDPOINT).expect("constant GitHub Releases endpoint");
            url.query_pairs_mut()
                .append_pair("per_page", &RELEASES_PER_PAGE.to_string())
                .append_pair("page", &page.to_string());
            let request = client
                .get(url)
                .header(reqwest::header::ACCEPT, "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28");
            let bytes = read_bounded(request, remaining).await?;
            remaining -= bytes.len();
            let releases: Vec<GithubRelease> =
                serde_json::from_slice(&bytes).map_err(|_| "GitHub 发行列表无效".to_owned())?;
            let count = releases.len();
            if let Some(candidate) = select_release(releases, current) {
                if latest.as_ref().is_none_or(|selected: &Release| {
                    candidate.version.cmp_precedence(&selected.version).is_gt()
                }) {
                    latest = Some(candidate);
                }
            }
            if count < RELEASES_PER_PAGE {
                return Ok(latest);
            }
        }
        Err("GitHub 发行列表超出检查范围，请稍后重试".into())
    }

    fn select_release(releases: Vec<GithubRelease>, current: &Version) -> Option<Release> {
        releases
            .into_iter()
            .filter_map(|release| release.candidate())
            .filter(|release| release.version.cmp_precedence(current).is_gt())
            .max_by(|left, right| left.version.cmp_precedence(&right.version))
    }

    impl GithubRelease {
        fn candidate(self) -> Option<Release> {
            let version = Version::parse(self.tag_name.strip_prefix('v')?).ok()?;
            let is_prerelease = !version.pre.is_empty();
            if self.draft
                || self.prerelease != is_prerelease
                || !version.build.is_empty()
                || self.tag_name != format!("v{version}")
            {
                return None;
            }
            let (manifest, manifest_size) = self.uploaded_asset(MANIFEST_NAME, MANIFEST_LIMIT)?;
            let (archive, archive_size) = self.uploaded_asset(ARCHIVE_NAME, ARCHIVE_LIMIT)?;
            Some(Release {
                version,
                manifest,
                manifest_size,
                archive,
                archive_size,
            })
        }

        fn uploaded_asset(&self, name: &str, limit: usize) -> Option<(Url, usize)> {
            let mut assets = self.assets.iter().filter(|asset| asset.name == name);
            let asset = assets.next()?;
            let expected = format!("{DOWNLOAD_BASE}/{}/{name}", self.tag_name);
            if assets.next().is_some()
                || asset.state != "uploaded"
                || asset.size == 0
                || asset.size > limit
                || asset.browser_download_url != expected
            {
                return None;
            }
            Some((Url::parse(&expected).ok()?, asset.size))
        }
    }

    pub(super) async fn resolve(
        app: &tauri::AppHandle,
        client: &Client,
        release: &Release,
    ) -> Result<Update, String> {
        let bytes =
            read_bounded(client.get(release.manifest.clone()), release.manifest_size).await?;
        if bytes.len() != release.manifest_size {
            return Err("更新清单大小与 GitHub 发行记录不符".into());
        }
        let raw = serde_json::from_slice::<Value>(&bytes)
            .map_err(|_| "GitHub 更新清单无效".to_owned())?;
        if !manifest_matches(&raw, release) {
            return Err("更新清单与 GitHub 发行记录不符".into());
        }
        let updater = app
            .updater_builder()
            .endpoints(vec![release.manifest.clone()])
            .map_err(|_| "更新源无效".to_owned())?
            .target(UPDATER_TARGET)
            .build()
            .map_err(|_| "无法初始化 Nasti 更新器".to_owned())?;
        let update = updater
            .restore_update(raw)
            .map_err(|_| "GitHub 更新清单无效".to_owned())?
            .ok_or("可用更新已变更，请重新检查更新")?;
        if update.version != release.version.to_string() || update.download_url != release.archive {
            return Err("更新清单与 GitHub 发行记录不符".into());
        }
        Ok(update)
    }

    fn manifest_matches(raw: &Value, release: &Release) -> bool {
        let Some(version) = raw["version"].as_str() else {
            return false;
        };
        let version = version.strip_prefix('v').unwrap_or(version);
        let target = &raw["platforms"][UPDATER_TARGET];
        version == release.version.to_string()
            && target["url"].as_str() == Some(release.archive.as_str())
            && target["signature"]
                .as_str()
                .is_some_and(|signature| !signature.trim().is_empty())
    }

    pub(super) async fn download(client: &Client, release: &Release) -> Result<Vec<u8>, String> {
        let bytes = read_bounded(client.get(release.archive.clone()), release.archive_size).await?;
        if bytes.len() != release.archive_size {
            return Err("更新包大小与 GitHub 发行记录不符".into());
        }
        Ok(bytes)
    }

    async fn read_bounded(request: RequestBuilder, limit: usize) -> Result<Vec<u8>, String> {
        let mut response = request
            .send()
            .await
            .map_err(|_| "无法连接 GitHub 检查或下载更新".to_owned())?;
        if matches!(response.status().as_u16(), 403 | 429) {
            return Err("GitHub 暂时限制更新请求，请稍后重试".into());
        }
        if !response.status().is_success() {
            return Err("无法读取 GitHub 更新文件".into());
        }
        if response
            .content_length()
            .is_some_and(|length| length > limit as u64)
        {
            return Err("GitHub 更新文件超出大小上限".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "读取 GitHub 更新文件时连接中断".to_owned())?
        {
            if chunk.len() > limit - bytes.len() {
                return Err("GitHub 更新文件超出大小上限".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use serde_json::json;

        fn release(tag: &str, draft: bool, prerelease: bool) -> GithubRelease {
            GithubRelease {
                tag_name: tag.into(),
                draft,
                prerelease,
                assets: [MANIFEST_NAME, ARCHIVE_NAME]
                    .into_iter()
                    .map(|name| GithubAsset {
                        name: name.into(),
                        state: "uploaded".into(),
                        size: 50,
                        browser_download_url: format!("{DOWNLOAD_BASE}/{tag}/{name}"),
                    })
                    .collect(),
            }
        }

        #[test]
        fn selects_highest_newer_beta_with_complete_canonical_assets() {
            let current = Version::parse("0.1.1-beta.1").unwrap();
            let mut untrusted_url = release("v0.1.5-beta.1", false, true);
            untrusted_url.assets[1].browser_download_url =
                "https://github.com/someone-else/Nasti/releases/download/v0.1.5-beta.1/Nasti.app.tar.gz"
                    .into();
            let mut unfinished = release("v0.1.4-beta.1", false, true);
            unfinished.assets[0].state = "new".into();
            let selected = select_release(
                vec![
                    release("v0.1.0-beta.1", false, true),
                    release("v0.1.3-beta.10", false, true),
                    release("v0.1.3-beta.9", false, true),
                    release("v0.1.3", true, false),
                    release("v0.2.0-beta.1", false, false),
                    untrusted_url,
                    unfinished,
                ],
                &current,
            )
            .unwrap();
            assert_eq!(selected.version.to_string(), "0.1.3-beta.10");
            assert!(
                select_release(vec![release("v0.1.1-beta.1", false, true)], &current).is_none()
            );
        }

        #[test]
        fn manifest_cannot_substitute_version_or_download_url() {
            let release = release("v0.1.2-beta.1", false, true).candidate().unwrap();
            let mut raw = json!({
                "version": "0.1.2-beta.1",
                "platforms": {
                    "darwin-aarch64": {
                        "url": release.archive.as_str(),
                        "signature": "an unverified signature"
                    }
                }
            });
            assert!(manifest_matches(&raw, &release));
            raw["version"] = json!("0.1.3-beta.1");
            assert!(!manifest_matches(&raw, &release));
            raw["version"] = json!("0.1.2-beta.1");
            raw["platforms"]["darwin-aarch64"]["url"] =
                json!("https://example.org/Nasti.app.tar.gz");
            assert!(!manifest_matches(&raw, &release));
        }

        #[test]
        fn permits_only_https_redirects_to_github_release_cdn() {
            let initial = Url::parse(DOWNLOAD_BASE).unwrap();
            let allowed = Url::parse("https://release-assets.githubusercontent.com/path").unwrap();
            assert!(download_redirect_allowed(&initial, &allowed));
            for url in [
                "http://release-assets.githubusercontent.com/path",
                "https://release-assets.githubusercontent.com.example.org/path",
                "https://example.org/path",
            ] {
                assert!(!download_redirect_allowed(
                    &initial,
                    &Url::parse(url).unwrap()
                ));
            }
        }
    }
}
