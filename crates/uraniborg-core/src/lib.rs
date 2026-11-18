//! Authorized catalog, timed lyrics, and durable playlist occurrences.
#[path = "../../../contracts/ipc-models.rs"]
pub mod models;

mod archive;
mod catalog;
mod library;
mod youtube;

pub use archive::{ArchiveAlbum, ArchiveCatalog, ArchiveError, ArchiveRecording};
pub use catalog::{Catalog, CatalogError};
pub use library::{Library, LibraryError};
pub use youtube::{
    music_track, playlist_column, playlist_id, video_id, YoutubeAccess, YoutubeApi, YoutubeError,
};
