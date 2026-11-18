//! Loopback Range transport and bounded offline copies for registered media.
#[path = "../../../contracts/ipc-models.rs"]
pub mod models;

mod archive_media;
mod pipe;
mod vault;
mod youtube_media;

pub use archive_media::{ArchiveMedia, ArchiveMediaError};
pub use pipe::{MediaPipe, MediaSource, PlaybackSource};
pub use vault::{Vault, VaultError};
pub use youtube_media::{
    youtube_id, youtube_id_from_vault_key, youtube_vault_key, YoutubeSearchHit,
    YoutubeSearchResults,
};
