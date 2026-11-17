use std::fs::{self, DirBuilder, Metadata};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

pub(crate) const OAUTH_PENDING: &str = ".oauth-client-pending";

pub(crate) struct RuntimeStorage {
    pub(crate) updater_dir: PathBuf,
    pub(crate) vault_dir: PathBuf,
    pub(crate) oauth_dir: PathBuf,
}

impl RuntimeStorage {
    /// Set up owned directories once the application's data lock is held.
    pub(crate) fn open(base: &Path, bundle_id: &str) -> io::Result<Self> {
        if bundle_id.len() > 128
            || !bundle_id.split('.').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
        {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "Invalid Nasti bundle identifier",
            ));
        }

        let base = base.canonicalize()?;
        let metadata = fs::symlink_metadata(&base)?;
        if !is_owned_directory(&metadata, false) {
            return Err(private_directory_error(&base));
        }

        let root = private_child(&base, ".nasti")?;
        let cache = private_child(&root, "cache")?;
        // No native media/cache writer uses this namespace yet. The Vault has
        // its own aggregate cap, including any copies in temporary storage.
        private_child(&cache, bundle_id)?;
        let tmp = private_child(&root, "tmp")?;
        let tmp = private_child(&tmp, bundle_id)?;
        let updater_dir = private_child(&tmp, "updater")?;
        let vault_dir = private_child(&tmp, "vault")?;
        let oauth_dir = private_child(&tmp, "oauth")?;
        recover_oauth(&oauth_dir)?;

        Ok(Self {
            updater_dir,
            vault_dir,
            oauth_dir,
        })
    }
}

fn private_child(parent: &Path, name: &str) -> io::Result<PathBuf> {
    let path = parent.join(name);
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = fs::symlink_metadata(&path)?;
    if !is_owned_directory(&metadata, true) {
        return Err(private_directory_error(&path));
    }
    Ok(path)
}

fn private_directory_error(path: &Path) -> io::Error {
    io::Error::new(
        ErrorKind::PermissionDenied,
        format!(
            "Nasti requires a private, user-owned directory at {}",
            path.display()
        ),
    )
}

#[cfg(unix)]
fn is_owned_directory(metadata: &Metadata, private: bool) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let mode = metadata.permissions().mode() & 0o777;
    metadata.is_dir()
        && is_owned(metadata)
        && if private {
            mode == 0o700
        } else {
            mode & 0o022 == 0
        }
}

#[cfg(windows)]
fn is_owned_directory(metadata: &Metadata, _: bool) -> bool {
    use std::os::windows::fs::MetadataExt;
    const REPARSE_POINT: u32 = 0x400;
    metadata.is_dir() && metadata.file_attributes() & REPARSE_POINT == 0
}

#[cfg(not(any(unix, windows)))]
fn is_owned_directory(metadata: &Metadata, _: bool) -> bool {
    metadata.is_dir()
}

#[cfg(unix)]
fn is_owned(metadata: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: geteuid takes no arguments and has no memory side effects.
    metadata.uid() == unsafe { libc::geteuid() }
}

#[cfg(not(unix))]
fn is_owned(_: &Metadata) -> bool {
    true
}

fn recover_oauth(oauth_dir: &Path) -> io::Result<()> {
    let pending = oauth_dir.join(OAUTH_PENDING);
    match fs::symlink_metadata(&pending) {
        Ok(meta) if meta.is_file() && is_owned(&meta) => fs::remove_file(pending),
        Ok(_) => Err(io::Error::new(
            ErrorKind::PermissionDenied,
            "Unexpected Google OAuth staging file",
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
pub(crate) fn same_volume(first: &Path, second: &Path) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    Ok(fs::metadata(first)?.dev() == fs::metadata(second)?.dev())
}

#[cfg(windows)]
pub(crate) fn same_volume(first: &Path, second: &Path) -> io::Result<bool> {
    Ok(first.canonicalize()?.components().next() == second.canonicalize()?.components().next())
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn same_volume(_: &Path, _: &Path) -> io::Result<bool> {
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporary_scope_is_isolated_and_only_known_oauth_staging_is_recovered() {
        let base = tempfile::tempdir().unwrap();
        let scope = RuntimeStorage::open(base.path(), "dev.nasti.test").unwrap();
        let final_config = base.path().join("youtube-oauth-client.json");
        let unrelated_file = scope.oauth_dir.join("keep");
        let stage = scope.oauth_dir.join(OAUTH_PENDING);
        fs::write(&final_config, "final").unwrap();
        fs::write(&unrelated_file, "keep").unwrap();
        fs::write(&stage, "unfinished").unwrap();
        RuntimeStorage::open(base.path(), "dev.nasti.test").unwrap();

        assert!(!stage.exists());
        assert_eq!(fs::read_to_string(final_config).unwrap(), "final");
        assert_eq!(fs::read_to_string(unrelated_file).unwrap(), "keep");
        assert!(base.path().join(".nasti/cache/dev.nasti.test").is_dir());
        assert!(same_volume(&scope.oauth_dir, base.path()).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(scope.oauth_dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn unmanaged_symlink_is_rejected_without_writing_to_its_target() {
        use std::os::unix::fs::symlink;
        let base = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        symlink(other.path(), base.path().join(".nasti")).unwrap();
        assert!(RuntimeStorage::open(base.path(), "dev.nasti.test").is_err());
        assert_eq!(fs::read_dir(other.path()).unwrap().count(), 0);
    }
}
