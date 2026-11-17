use crate::archive_media::{ArchiveHttp, ArchiveMedia, ArchiveMediaError};
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, Weak};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use uuid::Uuid;

const MIB: u64 = 1024 * 1024;
const DEFAULT_LIMIT: u64 = 256 * MIB;
const MAX_LIMIT: u64 = 2048 * MIB;
const AUDIO: &str = "audio.m4a";
const MP3: &str = "audio.mp3";
const VIDEO: &str = "video.mp4";
const LIMIT: &str = "limit-bytes";
const LIMIT_PENDING: &str = ".limit-pending";

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("Vault 存储操作失败：{0}")]
    Io(#[from] io::Error),
    #[error("无效的曲目 ID")]
    InvalidId,
    #[error("Vault 已达存储上限。可移除下载，或在设置中提高上限")]
    Full,
    #[error("存储上限不得超过 2 GiB")]
    LimitTooHigh,
    #[error("存储上限不得小于已下载大小")]
    LimitBelowUsage,
    #[error("Vault 暂时不可用，请重启应用")]
    Busy,
    #[error("Vault 临时目录与离线储藏库不在同一卷，无法安全写入")]
    StagingVolume,
    #[error(transparent)]
    Archive(#[from] ArchiveMediaError),
    #[error("下载的大小或校验和与授权曲目记录不符")]
    Verification,
}

pub struct Vault {
    root: PathBuf,
    staging: PathBuf,
    staging_is_atomic: bool,
    write_lock: Mutex<()>,
    playing_copies: StdMutex<HashMap<String, Weak<VaultCopy>>>,
}

struct VaultCopy {
    location: StdMutex<CopyLocation>,
}

struct CopyLocation {
    path: PathBuf,
    removed: bool,
}

impl Drop for VaultCopy {
    fn drop(&mut self) {
        let location = self
            .location
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        if location.removed {
            if let Err(error) = fs::remove_dir_all(&location.path) {
                tracing::warn!(%error, "Could not release removed Vault copy; startup will retry");
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct VaultPlaybackFile {
    copy: Arc<VaultCopy>,
    name: &'static str,
}

impl VaultPlaybackFile {
    pub(crate) fn open(&self) -> io::Result<File> {
        let location = self
            .copy
            .location
            .lock()
            .map_err(|_| io::Error::other("Vault copy is unavailable"))?;
        File::open(location.path.join(self.name))
    }
}

impl Vault {
    pub fn open(root: PathBuf) -> Result<Self, VaultError> {
        fs::create_dir_all(&root)?;
        let root = root.canonicalize()?;
        Self::open_directories(root.clone(), root, true)
    }

    /// Open a Vault with a private staging directory.
    ///
    /// The caller must hold the library lock before opening either location.
    pub fn open_with_staging(root: PathBuf, staging: PathBuf) -> Result<Self, VaultError> {
        fs::create_dir_all(&root)?;
        let root = root.canonicalize()?;
        if !fs::symlink_metadata(&staging)?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Vault staging must be a directory",
            )
            .into());
        }
        let staging = staging.canonicalize()?;
        if staging.starts_with(&root) || root.starts_with(&staging) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Vault staging and offline storage must be separate directories",
            )
            .into());
        }
        // On an unusual cross-volume layout, existing copies must remain
        // available to play. Reject writes that cannot be committed atomically.
        let staging_is_atomic = same_volume(&root, &staging)?;
        Self::open_directories(root, staging, staging_is_atomic)
    }

    fn open_directories(
        root: PathBuf,
        staging: PathBuf,
        staging_is_atomic: bool,
    ) -> Result<Self, VaultError> {
        // A prior version staged within the Vault. Clean those same owned names
        // on upgrade, while leaving finalized media and limit settings in place.
        recover_staging(&root)?;
        if staging != root {
            recover_staging(&staging)?;
        }
        Ok(Self {
            root,
            staging,
            staging_is_atomic,
            write_lock: Mutex::new(()),
            playing_copies: StdMutex::new(HashMap::new()),
        })
    }

    pub fn path(&self, id: &str, mode: &str) -> Option<PathBuf> {
        let dir = self.track_dir(id).ok()?;
        Some(dir.join(media_name(&dir, mode)?))
    }

    pub(crate) fn playback_file(&self, id: &str, mode: &str) -> Option<VaultPlaybackFile> {
        let dir = self.track_dir(id).ok()?;
        let mut playing = self.playing_copies.lock().ok()?;
        let name = media_name(&dir, mode)?;
        playing.retain(|_, copy| copy.strong_count() > 0);
        let copy = playing.get(id).and_then(Weak::upgrade).unwrap_or_else(|| {
            let copy = Arc::new(VaultCopy {
                location: StdMutex::new(CopyLocation {
                    path: dir,
                    removed: false,
                }),
            });
            playing.insert(id.to_owned(), Arc::downgrade(&copy));
            copy
        });
        Some(VaultPlaybackFile { copy, name })
    }

    pub fn stats(&self) -> Result<(u64, u64, usize), VaultError> {
        let _guard = self.write_lock.blocking_lock();
        self.read_stats()
    }

    pub fn items(&self) -> Result<Vec<(String, u64)>, VaultError> {
        let _guard = self.write_lock.blocking_lock();
        Ok(self.scan()?.1)
    }

    pub fn set_limit(&self, limit: u64) -> Result<(), VaultError> {
        if limit > MAX_LIMIT {
            return Err(VaultError::LimitTooHigh);
        }
        let _guard = self.write_lock.blocking_lock();
        if limit < self.read_stats()?.0 {
            return Err(VaultError::LimitBelowUsage);
        }
        self.require_atomic_staging()?;
        let staged = self.staging.join(LIMIT_PENDING);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)?;
        let result = (|| -> io::Result<()> {
            writeln!(file, "{limit}")?;
            file.sync_all()?;
            fs::rename(&staged, self.root.join(LIMIT))?;
            self.sync_commit()
        })();
        if result.is_err() && staged.exists() {
            fs::remove_file(staged)?;
        }
        Ok(result?)
    }

    pub fn download(&self, id: &str, audio: &Path, video: Option<&Path>) -> Result<(), VaultError> {
        let dir = self.track_dir(id)?;
        let _guard = self.write_lock.blocking_lock();
        let audio_needed = self.path(id, "audio").is_none();
        let video_needed = video.filter(|_| self.path(id, "video").is_none());
        if !audio_needed && video_needed.is_none() {
            return Ok(());
        }
        self.require_atomic_staging()?;
        let audio_length = if audio_needed {
            fs::metadata(audio)?.len()
        } else {
            0
        };
        let video_length = video_needed
            .map(fs::metadata)
            .transpose()?
            .map_or(0, |meta| meta.len());
        let (used, limit, _) = self.read_stats()?;
        if used
            .saturating_add(audio_length)
            .saturating_add(video_length)
            > limit
        {
            return Err(VaultError::Full);
        }

        let stage = self.staging.join(format!(".pending-{id}"));
        fs::create_dir(&stage)?;
        let result = (|| -> io::Result<()> {
            if audio_needed {
                copy_exact(audio, &stage.join(AUDIO), audio_length)?;
            }
            if let Some(video) = video_needed {
                copy_exact(video, &stage.join(VIDEO), video_length)?;
            }
            sync_directory(&stage)?;
            if audio_needed {
                fs::rename(&stage, &dir)?;
            } else {
                fs::rename(stage.join(VIDEO), dir.join(VIDEO))?;
                sync_directory(&dir)?;
            }
            self.sync_commit()
        })();
        if stage.exists() {
            fs::remove_dir_all(&stage)?;
        }
        Ok(result?)
    }

    pub(crate) async fn download_archive(
        &self,
        id: &str,
        media: &ArchiveMedia,
        http: &ArchiveHttp,
    ) -> Result<(), VaultError> {
        let dir = self.track_dir(id)?;
        let _guard = self.write_lock.lock().await;
        if self.path(id, "audio").is_some() {
            return Ok(());
        }
        self.require_atomic_staging()?;
        let (used, limit, _) = self.read_stats()?;
        if used.saturating_add(media.size) > limit {
            return Err(VaultError::Full);
        }

        let stage = self.staging.join(format!(".pending-{id}"));
        tokio::fs::create_dir(&stage).await?;
        let result = self.save_archive(&stage, &dir, media, http).await;
        if result.is_err() && stage.exists() {
            tokio::fs::remove_dir_all(&stage).await?;
        }
        result
    }

    async fn save_archive(
        &self,
        stage: &Path,
        dest: &Path,
        media: &ArchiveMedia,
        http: &ArchiveHttp,
    ) -> Result<(), VaultError> {
        let mut response = http.get(media, None).await?;
        let mut output = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(stage.join(MP3))
            .await?;
        let mut received = 0;
        let mut hash = Sha1::new();
        while let Some(chunk) = response.chunk().await.map_err(ArchiveMediaError::from)? {
            received += chunk.len() as u64;
            if received > media.size {
                return Err(VaultError::Verification);
            }
            hash.update(&chunk);
            output.write_all(&chunk).await?;
        }
        if received != media.size || format!("{:x}", hash.finalize()) != media.sha1 {
            return Err(VaultError::Verification);
        }
        output.sync_all().await?;
        drop(output);
        sync_directory(stage)?;
        tokio::fs::rename(stage, dest).await?;
        self.sync_commit()?;
        Ok(())
    }

    pub fn remove(&self, id: &str) -> Result<bool, VaultError> {
        self.remove_with(id, || Ok(()))
    }

    pub(crate) fn remove_with(
        &self,
        id: &str,
        retain_playing_copy: impl FnOnce() -> Result<(), VaultError>,
    ) -> Result<bool, VaultError> {
        let dir = self.track_dir(id)?;
        let _guard = self.write_lock.blocking_lock();
        if self.path(id, "audio").is_none() {
            return Ok(false);
        }
        self.require_atomic_staging()?;
        retain_playing_copy()?;
        let mut playing = self.playing_copies.lock().map_err(|_| VaultError::Busy)?;
        let removed = self
            .staging
            .join(format!(".removed-{}", Uuid::new_v4().simple()));
        let playing_copy = playing.get(id).and_then(Weak::upgrade);
        if let Some(copy) = playing_copy {
            let mut location = copy.location.lock().map_err(|_| VaultError::Busy)?;
            fs::rename(&dir, &removed)?;
            *location = CopyLocation {
                path: removed,
                removed: true,
            };
        } else {
            fs::rename(&dir, &removed)?;
            fs::remove_dir_all(&removed)?;
        }
        playing.remove(id);
        self.sync_commit()?;
        Ok(true)
    }

    fn sync_commit(&self) -> io::Result<()> {
        sync_directory(&self.root)?;
        if self.root != self.staging {
            sync_directory(&self.staging)?;
        }
        Ok(())
    }

    fn require_atomic_staging(&self) -> Result<(), VaultError> {
        if !self.staging_is_atomic {
            return Err(VaultError::StagingVolume);
        }
        Ok(())
    }

    fn track_dir(&self, id: &str) -> Result<PathBuf, VaultError> {
        valid_media_id(id)
            .then(|| self.root.join(id))
            .ok_or(VaultError::InvalidId)
    }

    fn read_stats(&self) -> Result<(u64, u64, usize), VaultError> {
        let limit = match fs::read_to_string(self.root.join(LIMIT)) {
            Ok(value) => value
                .trim()
                .parse::<u64>()
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
                .min(MAX_LIMIT),
            Err(error) if error.kind() == io::ErrorKind::NotFound => DEFAULT_LIMIT,
            Err(error) => return Err(error.into()),
        };
        let (used, items) = self.scan()?;
        Ok((used, limit, items.len()))
    }

    fn scan(&self) -> Result<(u64, Vec<(String, u64)>), VaultError> {
        let mut items = Vec::new();
        let mut used = scan_media(&self.root, Some(&mut items))?;
        if self.root != self.staging {
            used = used.saturating_add(scan_media(&self.staging, None)?);
        }
        items.sort_unstable_by(|first, second| first.0.cmp(&second.0));
        Ok((used, items))
    }
}

#[cfg(unix)]
fn same_volume(root: &Path, staging: &Path) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    Ok(fs::metadata(root)?.dev() == fs::metadata(staging)?.dev())
}

#[cfg(windows)]
fn same_volume(root: &Path, staging: &Path) -> io::Result<bool> {
    // Canonicalized paths expose distinct drive/UNC prefixes. Rename still
    // checks the volume for rarer mounted volumes within one drive.
    Ok(root.components().next() == staging.components().next())
}

#[cfg(not(any(unix, windows)))]
fn same_volume(_: &Path, _: &Path) -> io::Result<bool> {
    Ok(true)
}

fn is_staged_media(name: &str) -> bool {
    name.strip_prefix(".pending-").is_some_and(valid_media_id)
        || name.strip_prefix(".removed-").is_some_and(|id| {
            id.len() == 32
                && id
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        })
}

fn recover_staging(directory: &Path) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let kind = entry.file_type()?;
        if is_staged_media(&name) && kind.is_dir() {
            fs::remove_dir_all(entry.path())?;
        } else if name == LIMIT_PENDING && kind.is_file() {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn scan_media(directory: &Path, mut items: Option<&mut Vec<(String, u64)>>) -> io::Result<u64> {
    let mut used = 0u64;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let active = items.is_some() && valid_media_id(&name);
        if !active && !is_staged_media(&name) {
            continue;
        }
        let mut has_audio = false;
        let mut bytes = 0u64;
        for file in [AUDIO, MP3, VIDEO] {
            match fs::symlink_metadata(entry.path().join(file)) {
                Ok(meta) if meta.is_file() => {
                    bytes = bytes.saturating_add(meta.len());
                    has_audio |= file != VIDEO;
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        if let Some(items) = items.as_deref_mut().filter(|_| active && has_audio) {
            items.push((name.into_owned(), bytes));
        }
        used = used.saturating_add(bytes);
    }
    Ok(used)
}

fn media_name(dir: &Path, mode: &str) -> Option<&'static str> {
    let names: &[&str] = match mode {
        "audio" => &[AUDIO, MP3],
        "video" => &[VIDEO],
        _ => return None,
    };
    if !fs::symlink_metadata(dir).ok()?.is_dir() {
        return None;
    }
    names
        .iter()
        .copied()
        .find(|name| fs::symlink_metadata(dir.join(name)).is_ok_and(|metadata| metadata.is_file()))
}

pub(crate) fn valid_media_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn copy_exact(source: &Path, dest: &Path, length: u64) -> io::Result<()> {
    let mut input = File::open(source)?;
    let mut output = OpenOptions::new().create_new(true).write(true).open(dest)?;
    // A source that grows after its size has been reserved cannot exceed the quota.
    let copied = io::copy(&mut (&mut input).take(length), &mut output)?;
    if copied != length || input.read(&mut [0; 1])? != 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Media size changed during copy",
        ));
    }
    output.sync_all()
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relocated_staging_counts_leases_and_recovers_only_owned_scratch() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().join("vault");
        let staging = scratch.path().join("tmp").join("vault");
        let audio = scratch.path().join("audio-fixture");
        fs::write(&audio, [1; 16]).unwrap();
        fs::create_dir_all(root.join(".pending-legacy")).unwrap();
        fs::create_dir_all(staging.join(".pending-abandoned")).unwrap();
        fs::create_dir(staging.join(".removed-keep")).unwrap();
        fs::write(staging.join(LIMIT_PENDING), "abandoned").unwrap();

        let vault = Vault::open_with_staging(root.clone(), staging.clone()).unwrap();
        assert!(!root.join(".pending-legacy").exists());
        assert!(!staging.join(".pending-abandoned").exists());
        assert!(!staging.join(LIMIT_PENDING).exists());
        assert!(staging.join(".removed-keep").exists());
        vault.set_limit(16).unwrap();
        vault.download("alpha", &audio, None).unwrap();
        let lease = vault.playback_file("alpha", "audio").unwrap();

        assert!(vault.remove("alpha").unwrap());
        assert!(!root.join("alpha").exists());
        assert_eq!(vault.stats().unwrap(), (16, 16, 0));
        assert!(matches!(
            vault.download("beta", &audio, None),
            Err(VaultError::Full)
        ));
        assert!(fs::read_dir(&staging)
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| is_staged_media(&entry.file_name().to_string_lossy())));
        let mut leased_bytes = Vec::new();
        lease
            .open()
            .unwrap()
            .read_to_end(&mut leased_bytes)
            .unwrap();
        assert_eq!(leased_bytes, [1; 16]);

        drop(lease);
        assert_eq!(vault.stats().unwrap(), (0, 16, 0));
        vault.download("beta", &audio, None).unwrap();
        assert!(root.join("beta").join(AUDIO).is_file());
    }

    #[test]
    fn read_only_vault_on_an_incompatible_volume_keeps_final_media() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().join("vault");
        let staging = scratch.path().join("tmp");
        fs::create_dir_all(root.join("alpha")).unwrap();
        fs::create_dir(&staging).unwrap();
        fs::write(root.join("alpha").join(AUDIO), [1; 16]).unwrap();
        let vault = Vault::open_directories(root.clone(), staging.clone(), false).unwrap();

        assert!(vault
            .playback_file("alpha", "audio")
            .unwrap()
            .open()
            .is_ok());
        assert_eq!(vault.stats().unwrap(), (16, DEFAULT_LIMIT, 1));
        assert!(matches!(
            vault.remove("alpha"),
            Err(VaultError::StagingVolume)
        ));
        assert!(matches!(
            vault.set_limit(32),
            Err(VaultError::StagingVolume)
        ));
        assert!(matches!(
            vault.download("beta", &root.join("alpha").join(AUDIO), None),
            Err(VaultError::StagingVolume)
        ));
        assert!(root.join("alpha").join(AUDIO).exists());
        assert_eq!(fs::read_dir(&staging).unwrap().count(), 0);
    }

    #[test]
    fn vault_enforces_total_cap_and_can_upgrade_an_existing_audio_copy() {
        let scratch = tempfile::tempdir().unwrap();
        let audio = scratch.path().join("audio-fixture");
        let video = scratch.path().join("video-fixture");
        fs::write(&audio, [1; 16]).unwrap();
        fs::write(&video, [2; 48]).unwrap();
        let root = scratch.path().join("vault");
        let vault = Vault::open(root.clone()).unwrap();
        vault.set_limit(32).unwrap();

        assert!(matches!(
            vault.download("alpha", &audio, Some(&video)),
            Err(VaultError::Full)
        ));
        assert_eq!(vault.stats().unwrap(), (0, 32, 0));
        vault.download("alpha", &audio, None).unwrap();
        assert_eq!(vault.stats().unwrap(), (16, 32, 1));
        assert!(matches!(
            vault.set_limit(10),
            Err(VaultError::LimitBelowUsage)
        ));
        assert!(matches!(
            vault.download("alpha", &audio, Some(&video)),
            Err(VaultError::Full)
        ));
        assert!(vault.path("alpha", "audio").is_some());
        assert!(vault.path("alpha", "video").is_none());

        vault.set_limit(64).unwrap();
        vault.download("alpha", &audio, Some(&video)).unwrap();
        assert_eq!(vault.stats().unwrap(), (64, 64, 1));
        drop(vault);
        let vault = Vault::open(root).unwrap();
        assert_eq!(vault.stats().unwrap(), (64, 64, 1));
        assert!(vault.remove("alpha").unwrap());
        assert_eq!(vault.stats().unwrap(), (0, 64, 0));
        assert!(!vault.remove("alpha").unwrap());
        assert!(matches!(
            vault.download("../private", &audio, None),
            Err(VaultError::InvalidId)
        ));
    }
}
