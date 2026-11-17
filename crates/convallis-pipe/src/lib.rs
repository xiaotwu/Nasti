//! Loopback Range transport and bounded offline copies for registered media.
#[path = "../../../contracts/ipc-models.rs"]
pub mod models;

mod archive_media;
mod pipe;
mod vault;

pub use archive_media::{ArchiveMedia, ArchiveMediaError};
pub use pipe::{MediaPipe, MediaSource, PlaybackSource};
pub use vault::{Vault, VaultError};
