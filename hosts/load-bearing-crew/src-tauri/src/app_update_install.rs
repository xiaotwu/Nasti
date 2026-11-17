use flate2::read::GzDecoder;
use rustix::fs::{open, renameat_with, Mode, OFlags, RenameFlags, CWD};
use semver::Version;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

const MIB: u64 = 1024 * 1024;
const APP_BYTES: u64 = 120 * MIB;
const TEMP_BYTES: u64 = 256 * MIB;
const MAX_ENTRIES: usize = 4096;
const APP_NAME: &str = "Nasti.app";
const APP_ID: &str = "dev.nasti.terminal.beta";
const SIGNING_REQUIREMENT: &str = r#"identifier "dev.nasti.terminal.beta" and anchor apple generic and certificate leaf[subject.OU] = "9URWGD9Q86" and certificate leaf[field.1.2.840.113635.100.6.1.13] exists"#;
const UPDATE_MARKER: &str = ".nasti-update-stage";
const UPDATE_MARKER_CONTENTS: &[u8] = b"Nasti update staging v1\n";

#[derive(serde::Deserialize)]
struct BundleInfo {
    #[serde(rename = "CFBundleIdentifier")]
    identifier: String,
    #[serde(rename = "CFBundleShortVersionString")]
    version: String,
}

fn invalid_bundle() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "The update archive is not a valid Nasti bundle",
    )
}

fn archive_path(path: &Path) -> io::Result<()> {
    let mut parts = path.components();
    if parts.next() != Some(Component::Normal(OsStr::new(APP_NAME)))
        || !parts.all(|part| matches!(part, Component::Normal(_)))
    {
        return Err(invalid_bundle());
    }
    Ok(())
}

fn unpack_bundle(bytes: &[u8], destination: &Path) -> io::Result<()> {
    let expanded = GzDecoder::new(bytes).take(APP_BYTES + 8 * MIB);
    let mut archive = tar::Archive::new(expanded);
    let mut payload_bytes: u64 = 0;
    for (index, entry) in archive.entries()?.enumerate() {
        if index >= MAX_ENTRIES {
            return Err(invalid_bundle());
        }
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        archive_path(&path)?;
        let output = destination.join(path);
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            fs::create_dir_all(output)?;
        } else if kind.is_file() {
            let size = entry.size();
            payload_bytes = payload_bytes.checked_add(size).ok_or_else(invalid_bundle)?;
            if payload_bytes > APP_BYTES {
                return Err(invalid_bundle());
            }
            fs::create_dir_all(output.parent().ok_or_else(invalid_bundle)?)?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(entry.header().mode()? & 0o755)
                .open(output)?;
            if io::copy(&mut entry, &mut file)? != size {
                return Err(invalid_bundle());
            }
            file.sync_all()?;
        } else {
            // The signed release bundle contains only ordinary files and directories.
            return Err(invalid_bundle());
        }
    }
    Ok(())
}

fn bundle_info(bundle: &Path) -> Result<BundleInfo, String> {
    let info: BundleInfo = plist::from_file(bundle.join("Contents/Info.plist"))
        .map_err(|_| "无法读取 Nasti 应用版本".to_owned())?;
    if info.identifier != APP_ID {
        return Err("更新包不是当前 Nasti 应用".into());
    }
    Ok(info)
}

fn verify_bundle(bundle: &Path) -> Result<(), String> {
    let verify = Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict", "--deep", "-R", SIGNING_REQUIREMENT])
        .arg(bundle)
        .output();
    if !verify.is_ok_and(|out| out.status.success()) {
        return Err("更新包未通过 Nasti Developer ID 签名校验".into());
    }
    let assess = Command::new("/usr/sbin/spctl")
        .args(["--assess", "--type", "execute"])
        .arg(bundle)
        .output();
    if !assess.is_ok_and(|out| out.status.success()) {
        return Err("macOS 未接受此更新包".into());
    }
    Ok(())
}

fn disk_bytes(directory: &Path) -> io::Result<u64> {
    let mut total: u64 = 0;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let size = if kind.is_dir() {
            disk_bytes(&entry.path())?
        } else {
            fs::symlink_metadata(entry.path())?.len()
        };
        total = total.saturating_add(size);
        if total >= TEMP_BYTES {
            return Ok(total);
        }
    }
    Ok(total)
}

fn reserve_work_directory(root: &Path, current_size: u64) -> Result<tempfile::TempDir, String> {
    // The serialized installer owns marked staging directories, including crash residues.
    for entry in fs::read_dir(root).map_err(|_| "无法读取 Nasti 更新工作目录")? {
        let entry = entry.map_err(|_| "无法读取 Nasti 更新工作目录")?;
        let name = entry.file_name();
        let Some(suffix) = name.to_str().and_then(|s| s.strip_prefix("stage-")) else {
            continue;
        };
        if suffix.len() != 6 || !suffix.bytes().all(|b| b.is_ascii_alphanumeric()) {
            continue;
        }
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let marker = entry.path().join(UPDATE_MARKER);
        if fs::symlink_metadata(&marker)
            .is_ok_and(|meta| meta.is_file() && meta.len() == UPDATE_MARKER_CONTENTS.len() as u64)
            && fs::read(marker).is_ok_and(|data| data == UPDATE_MARKER_CONTENTS)
        {
            fs::remove_dir_all(entry.path()).map_err(|_| "无法清理上次更新的临时文件")?;
        }
    }
    let present = disk_bytes(root).map_err(|_| "无法计量 Nasti 更新工作目录")?;
    if present
        .saturating_add(current_size)
        .saturating_add(APP_BYTES)
        > TEMP_BYTES
    {
        return Err("Nasti 更新临时目录已达容量上限".into());
    }
    let work = tempfile::Builder::new()
        .prefix("stage-")
        .tempdir_in(root)
        .map_err(|_| "无法创建 Nasti 更新工作目录")?;
    fs::write(work.path().join(UPDATE_MARKER), UPDATE_MARKER_CONTENTS)
        .map_err(|_| "无法标记 Nasti 更新工作目录")?;
    Ok(work)
}

fn current_bundle() -> Result<PathBuf, String> {
    let binary = std::env::current_exe().map_err(|_| "无法定位当前 Nasti")?;
    let bundle = binary
        .ancestors()
        .nth(3)
        .filter(|path| path.file_name() == Some(OsStr::new(APP_NAME)))
        .ok_or("请先用 DMG 将 Nasti 安装到应用程序文件夹")?;
    if bundle
        .components()
        .any(|component| component.as_os_str() == "AppTranslocation")
    {
        return Err("请先用 DMG 将 Nasti 安装到应用程序文件夹".into());
    }
    Ok(bundle.to_owned())
}

pub(crate) fn install_verified(
    archive: &[u8],
    version: &str,
    updater_dir: &Path,
) -> Result<PathBuf, String> {
    let bundle = current_bundle()?;
    let bundle = bundle.as_path();
    let installed = fs::symlink_metadata(bundle).map_err(|_| "无法读取当前 Nasti")?;
    let scratch = fs::symlink_metadata(updater_dir).map_err(|_| "无法读取 Nasti 更新工作目录")?;
    if !installed.is_dir() || !scratch.is_dir() || installed.dev() != scratch.dev() {
        return Err("当前安装与更新工作目录不在同一磁盘。请使用 DMG 更新".into());
    }
    let lock: File = open(
        updater_dir.join(".install.lock"),
        OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )
    .map(File::from)
    .map_err(|_| "无法锁定 Nasti 更新工作目录")?;
    lock.try_lock()
        .map_err(|_| "另一个 Nasti 实例正在安装更新")?;
    let latest = Version::parse(version).map_err(|_| "更新版本编号无效")?;
    let current_version =
        Version::parse(&bundle_info(bundle)?.version).map_err(|_| "当前应用版本编号无效")?;
    if latest <= current_version {
        return Err("当前安装的 Nasti 已更新，请重新打开应用".into());
    }
    let current_size = disk_bytes(bundle).map_err(|_| "无法计量当前 Nasti 安装")?;
    if current_size > APP_BYTES {
        return Err("当前应用超过自动更新容量上限。请使用 DMG 更新".into());
    }
    let work = reserve_work_directory(updater_dir, current_size)?;
    unpack_bundle(archive, work.path()).map_err(|_| "无法解包通过签名校验的 Nasti 更新")?;
    let candidate = work.path().join(APP_NAME);
    if bundle_info(&candidate)?.version != version {
        return Err("更新包内的版本号不匹配".into());
    }
    verify_bundle(&candidate)?;

    // Both paths are on the same APFS volume. Failure leaves the installed bundle intact.
    renameat_with(CWD, &candidate, CWD, bundle, RenameFlags::EXCHANGE)
        .map_err(|_| "无法替换当前 Nasti。请用 DMG 更新后重新打开".to_owned())?;
    Ok(bundle.to_owned())
}

pub(crate) fn relaunch_after_update(app: &tauri::AppHandle, bundle: &Path) -> Result<(), String> {
    // Let the old process exit and release its SQLite lock before opening the replacement.
    let script = r#"
i=0
while kill -0 "$1" 2>/dev/null; do
    [ "$i" -lt 100 ] || exit 1
    i=$((i + 1))
    /bin/sleep 0.1
done
if [ "$3" != "" ]; then
    exec /usr/bin/open -n --env "$3" "$2"
fi
exec /usr/bin/open -n "$2"
"#;
    let data_override = std::env::var_os("NASTI_DATA_DIR")
        .filter(|path| !path.is_empty())
        .map(|path| {
            let mut assignment = OsString::from("NASTI_DATA_DIR=");
            assignment.push(path);
            assignment
        })
        .unwrap_or_default();
    Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .arg("nasti-relaunch")
        .arg(std::process::id().to_string())
        .arg(bundle)
        .arg(data_override)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|_| "更新已安装，请重新打开 Nasti".to_owned())?;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // Let the IPC acknowledgement reach the settings UI first.
        tokio::time::sleep(std::time::Duration::from_millis(160)).await;
        app.exit(0);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_archive_paths_to_a_single_app_bundle() {
        assert!(archive_path(Path::new("Nasti.app/Contents/Info.plist")).is_ok());
        for unsafe_path in [
            "Other.app/Contents/Info.plist",
            "Nasti.app/../outside",
            "/Nasti.app",
        ] {
            assert!(archive_path(Path::new(unsafe_path)).is_err());
        }
    }
}
