use crate::models::{LyricLine, MediaMode, PlaylistColumn, SearchResults, TrackItem};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

pub(crate) const COLUMN_COUNT: usize = 9;

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("无法打开授权媒体目录：{0}")]
    Io(#[from] std::io::Error),
    #[error("授权媒体目录格式错误：{0}")]
    Json(#[from] serde_json::Error),
    #[error("授权媒体目录记录无效：{0}")]
    Invalid(String),
    #[error("授权媒体目录中没有此曲目：{0}")]
    UnknownTrack(String),
    #[error("所选曲目没有此媒体版本")]
    MissingVariant,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourcePlaylist {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub video_ids: Vec<String>,
}

#[derive(Deserialize)]
struct Manifest {
    tracks: Vec<TrackItem>,
    playlists: Vec<SourcePlaylist>,
    lyrics: BTreeMap<String, Vec<LyricLine>>,
}

pub struct Catalog {
    root: PathBuf,
    tracks: BTreeMap<String, TrackItem>,
    pub(crate) playlists: Vec<SourcePlaylist>,
    lyrics: BTreeMap<String, Vec<LyricLine>>,
}

impl Catalog {
    pub fn load(root: &Path) -> Result<Self, CatalogError> {
        let root = root.canonicalize()?;
        let manifest: Manifest =
            serde_json::from_slice(&std::fs::read(root.join("catalog.json"))?)?;
        let mut tracks = BTreeMap::new();
        for mut track in manifest.tracks {
            let id = track.video_id.clone();
            if !valid_id(&id)
                || track.title.trim().is_empty()
                || track.artists.is_empty()
                || track.duration_sec == 0
                || track.license.as_deref().is_none_or(str::is_empty)
                || track.mv_video_id.as_deref() != track.mv_url.as_ref().map(|_| id.as_str())
            {
                return Err(CatalogError::Invalid(id));
            }
            asset_path(&root, Some(&track.thumbnail_square_url))?;
            asset_path(&root, track.audio_url.as_deref())?;
            if track.mv_url.is_some() {
                asset_path(&root, track.mv_url.as_deref())?;
            }
            track.entry_id = None;
            track.is_vault_downloaded = false;
            track.track_number = None;
            if tracks.insert(id.clone(), track).is_some() {
                return Err(CatalogError::Invalid(id));
            }
        }

        let mut playlist_ids = BTreeSet::new();
        for playlist in &manifest.playlists {
            if !valid_id(&playlist.id)
                || !playlist_ids.insert(&playlist.id)
                || playlist.title.trim().is_empty()
                || playlist.video_ids.iter().any(|id| !tracks.contains_key(id))
            {
                return Err(CatalogError::Invalid(playlist.id.clone()));
            }
        }

        let is_empty_release = tracks.is_empty()
            && manifest.playlists.is_empty()
            && manifest.lyrics.is_empty();
        if !is_empty_release && (tracks.is_empty() || manifest.playlists.is_empty()) {
            return Err(CatalogError::Invalid("目录为空".into()));
        }

        Ok(Self {
            root,
            tracks,
            playlists: manifest.playlists,
            lyrics: manifest.lyrics,
        })
    }

    pub fn tracks(&self) -> impl Iterator<Item = &TrackItem> {
        self.tracks.values()
    }

    pub fn track(&self, id: &str) -> Result<&TrackItem, CatalogError> {
        self.tracks
            .get(id)
            .ok_or_else(|| CatalogError::UnknownTrack(id.into()))
    }

    pub fn media_path(&self, id: &str, mode: MediaMode) -> Result<PathBuf, CatalogError> {
        let track = self.track(id)?;
        let url = match mode {
            MediaMode::Audio => track.audio_url.as_deref(),
            MediaMode::Video => track.mv_url.as_deref(),
        };
        asset_path(&self.root, url)
    }

    pub fn lyrics(&self, id: &str) -> Result<Vec<LyricLine>, CatalogError> {
        self.track(id)?;
        Ok(self.lyrics.get(id).cloned().unwrap_or_default())
    }

    pub fn search(&self, query: &str, filter: &str) -> SearchResults {
        let query = query.trim().to_lowercase();
        let matches = |value: &str| value.to_lowercase().contains(&query);
        let tracks: Vec<_> = self
            .tracks()
            .filter(|track| matches_track(track, &query))
            .cloned()
            .collect();
        SearchResults {
            videos: if matches!(filter, "all" | "videos") {
                tracks
                    .iter()
                    .filter(|track| track.mv_video_id.is_some())
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            },
            songs: if matches!(filter, "all" | "songs") {
                tracks
            } else {
                Vec::new()
            },
            playlists: if matches!(filter, "all" | "playlists") {
                self.playlists
                    .iter()
                    .enumerate()
                    .filter(|(_, playlist)| {
                        matches(&playlist.title)
                            || playlist.subtitle.as_deref().is_some_and(matches)
                            || matches(&playlist.id)
                    })
                    .map(|(index, source)| PlaylistColumn {
                        playlist_id: source.id.clone(),
                        column_index: index % COLUMN_COUNT,
                        code_name: format!("RL-{:02}", index + 1),
                        title: source.title.clone(),
                        subtitle: source.subtitle.clone(),
                        is_editable: false,
                        tracks: source
                            .video_ids
                            .iter()
                            .filter_map(|id| self.tracks.get(id).cloned())
                            .collect(),
                    })
                    .collect()
            } else {
                Vec::new()
            },
        }
    }
}

pub(crate) fn matches_track(track: &TrackItem, query: &str) -> bool {
    let matches = |value: &str| value.to_lowercase().contains(query);
    matches(&track.title)
        || track.artists.iter().any(|artist| matches(&artist.name))
        || track
            .album
            .as_ref()
            .is_some_and(|album| matches(&album.name))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn asset_path(root: &Path, url: Option<&str>) -> Result<PathBuf, CatalogError> {
    let url = url.ok_or(CatalogError::MissingVariant)?;
    let relative = url
        .strip_prefix("/authorized-media/")
        .ok_or_else(|| CatalogError::Invalid(url.into()))?;
    let relative = Path::new(relative);
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(CatalogError::Invalid(url.into()));
    }
    let path = root.join(relative).canonicalize()?;
    if path.is_file() && path.starts_with(root) {
        Ok(path)
    } else {
        Err(CatalogError::Invalid(url.into()))
    }
}
