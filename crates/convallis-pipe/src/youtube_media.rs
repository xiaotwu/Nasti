use reqwest::{
    header::{ACCEPT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, RANGE},
    Client, Response, StatusCode, Url,
};
use serde::Deserialize;
use std::{io, path::PathBuf, process::Stdio, sync::Arc, time::Duration};
use tokio::process::Command;

#[derive(Clone)]
pub(crate) struct YoutubeHttp {
    client: Client,
    search_slots: Arc<tokio::sync::Semaphore>,
}

#[derive(Deserialize)]
pub struct YoutubeSearchHit {
    pub id: String,
    pub title: Option<String>,
    pub channel: Option<String>,
    pub duration: Option<f64>,
    pub url: Option<String>,
}

#[derive(Default)]
pub struct YoutubeSearchResults {
    pub songs: Vec<YoutubeSearchHit>,
    pub videos: Vec<YoutubeSearchHit>,
    pub playlists: Vec<YoutubeSearchHit>,
}

#[derive(Deserialize)]
struct FlatPlaylist {
    entries: Vec<Option<YoutubeSearchHit>>,
}

pub(crate) struct YoutubeStream {
    url: Url,
    pub(crate) size: u64,
}

impl YoutubeHttp {
    pub(crate) fn new() -> Result<Self, reqwest::Error> {
        Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(8))
            .read_timeout(Duration::from_secs(20))
            .build()
            .map(|client| Self {
                client,
                search_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            })
    }

    pub(crate) async fn search(
        &self,
        query: &str,
        filter: &str,
    ) -> io::Result<YoutubeSearchResults> {
        let _slot = self
            .search_slots
            .try_acquire()
            .map_err(|_| io::Error::other("YouTube Music 正在搜索，请稍后重试"))?;
        let search = |kind: &str, section| {
            let query = query.trim();
            let enabled = filter == "all" || kind == filter;
            async move {
                if enabled {
                    self.search_section(query, section).await
                } else {
                    Ok(vec![])
                }
            }
        };
        let (songs, videos, playlists) = tokio::try_join!(
            search("songs", "songs"),
            search("videos", "videos"),
            search("playlists", "playlists"),
        )?;
        Ok(YoutubeSearchResults {
            songs,
            videos,
            playlists,
        })
    }

    async fn search_section(
        &self,
        query: &str,
        section: &str,
    ) -> io::Result<Vec<YoutubeSearchHit>> {
        let playlists = section == "playlists";
        let mut url = Url::parse(if playlists {
            "https://www.youtube.com/results"
        } else {
            "https://music.youtube.com/search"
        })
        .expect("known search origins");
        let query = query.chars().take(256).collect::<String>();
        url.query_pairs_mut().append_pair("q", &query);
        if playlists {
            // The flat Music playlist results omit titles. The public playlist filter returns them.
            url.query_pairs_mut().append_pair("sp", "EgIQAw%3D%3D");
        } else {
            url.set_fragment(Some(section));
        }
        let output = run_extractor(
            &[
                "--flat-playlist",
                "--playlist-end",
                "16",
                "--dump-single-json",
                "--",
                url.as_str(),
            ],
            Duration::from_secs(15),
            "YouTube Music 搜索暂不可用",
        )
        .await?;
        if output.len() > 1024 * 1024 {
            return Err(io::Error::other("YouTube Music 返回了过大的搜索结果"));
        }
        let playlist: FlatPlaylist = serde_json::from_slice(&output)
            .map_err(|_| io::Error::other("YouTube Music 返回了无效的搜索结果"))?;
        Ok(playlist.entries.into_iter().flatten().take(16).collect())
    }

    pub(crate) async fn resolve(&self, id: &str, mode: &str) -> io::Result<YoutubeStream> {
        let format = match mode {
            "audio" => "bestaudio[ext=m4a][protocol=https]",
            "video" => "best[ext=mp4][vcodec^=avc1][protocol=https]",
            _ => return Err(io::Error::new(io::ErrorKind::InvalidInput, "媒体模式无效")),
        };
        let urls = self.extract(id, format).await?;
        self.probe(urls.into_iter().next().unwrap()).await
    }

    #[cfg(target_os = "macos")]
    pub(crate) async fn resolve_video(
        &self,
        id: &str,
    ) -> io::Result<(YoutubeStream, Option<YoutubeStream>)> {
        let mut urls = self
            .extract(
                id,
                concat!(
                    "bestvideo[ext=mp4][vcodec^=avc1][protocol=https]+bestaudio[ext=m4a][protocol=https]",
                    "/best[ext=mp4][vcodec^=avc1][protocol=https]"
                ),
            )
            .await?
            .into_iter();
        let video = urls.next().unwrap();
        match urls.next() {
            Some(audio) => {
                let (video, audio) = tokio::try_join!(self.probe(video), self.probe(audio))?;
                Ok((video, Some(audio)))
            }
            None => Ok((self.probe(video).await?, None)),
        }
    }

    async fn extract(&self, id: &str, format: &str) -> io::Result<Vec<Url>> {
        if !youtube_id(id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "YouTube 视频 ID 无效",
            ));
        }
        let watch = format!("https://www.youtube.com/watch?v={id}");
        let output = run_extractor(
            &[
                "--no-playlist",
                "--format",
                format,
                "--get-url",
                "--",
                &watch,
            ],
            Duration::from_secs(35),
            "YouTube 暂未提供可在播放器内使用的媒体格式",
        )
        .await?;
        let value = std::str::from_utf8(&output)
            .ok()
            .map(str::trim)
            .filter(|value| !value.is_empty() && value.len() <= 32_768)
            .ok_or_else(|| io::Error::other("YouTube 媒体地址无效"))?;
        let urls = value
            .lines()
            .map(|value| Url::parse(value).ok().filter(allowed_cdn))
            .collect::<Option<Vec<_>>>()
            .filter(|urls| matches!(urls.len(), 1 | 2))
            .ok_or_else(|| io::Error::other("YouTube 媒体地址无效"))?;
        Ok(urls)
    }

    async fn probe(&self, url: Url) -> io::Result<YoutubeStream> {
        let probe = self.request(&url, 0, 0).await?;
        let size = probe
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|header| header.to_str().ok())
            .and_then(|header| header.strip_prefix("bytes 0-0/"))
            .and_then(|total| total.parse::<u64>().ok())
            .filter(|size| *size > 0)
            .ok_or_else(|| io::Error::other("YouTube 媒体未提供可寻址长度"))?;
        Ok(YoutubeStream { url, size })
    }

    pub(crate) async fn get(
        &self,
        stream: &YoutubeStream,
        start: u64,
        end: u64,
    ) -> io::Result<Response> {
        let response = self.request(&stream.url, start, end).await?;
        let expected = format!("bytes {start}-{end}/{}", stream.size);
        let length = response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        if response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            != Some(&expected)
            || length.is_some_and(|length| length != end - start + 1)
        {
            return Err(io::Error::other("YouTube 返回了不匹配的媒体范围"));
        }
        Ok(response)
    }

    async fn request(&self, url: &Url, start: u64, end: u64) -> io::Result<Response> {
        let response = self
            .client
            .get(url.clone())
            .header(ACCEPT_ENCODING, "identity")
            .header(RANGE, format!("bytes={start}-{end}"))
            .send()
            .await
            .map_err(|error| io::Error::other(error.without_url()))?;
        if response.status() != StatusCode::PARTIAL_CONTENT {
            return Err(io::Error::other(format!(
                "YouTube 媒体暂不可读（HTTP {}）",
                response.status().as_u16()
            )));
        }
        Ok(response)
    }
}

async fn run_extractor(
    args: &[&str],
    timeout: Duration,
    failure: &'static str,
) -> io::Result<Vec<u8>> {
    let output = tokio::time::timeout(
        timeout,
        Command::new(extractor())
            .args([
                "--ignore-config",
                "--no-plugin-dirs",
                "--no-cache-dir",
                "--no-progress",
                "--socket-timeout",
                "10",
                "--retries",
                "1",
            ])
            .args(args)
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "YouTube 请求超时"))?
    .map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            io::Error::new(error.kind(), "未找到 yt-dlp，无法读取 YouTube 内容")
        } else {
            error
        }
    })?;
    if !output.status.success() {
        return Err(io::Error::other(failure));
    }
    Ok(output.stdout)
}

fn extractor() -> PathBuf {
    if let Some(path) = std::env::var_os("NASTI_YT_DLP").filter(|path| !path.is_empty()) {
        return path.into();
    }
    #[cfg(target_os = "macos")]
    for path in ["/opt/homebrew/bin/yt-dlp", "/usr/local/bin/yt-dlp"] {
        let path = PathBuf::from(path);
        if path.is_file() {
            return path;
        }
    }
    "yt-dlp".into()
}

fn allowed_cdn(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && url.path() == "/videoplayback"
        && url
            .host_str()
            .is_some_and(|host| host == "googlevideo.com" || host.ends_with(".googlevideo.com"))
}

pub fn youtube_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// Vault directories must remain unique on case-insensitive volumes.
pub fn youtube_vault_key(id: &str) -> Option<String> {
    youtube_id(id).then(|| {
        use std::fmt::Write;
        let mut key = String::from("yt-");
        for byte in id.bytes() {
            write!(&mut key, "{byte:02x}").expect("writing to String");
        }
        key
    })
}

pub fn youtube_id_from_vault_key(key: &str) -> Option<String> {
    let bytes = key.strip_prefix("yt-")?;
    if bytes.len() != 22 || !bytes.is_ascii() {
        return None;
    }
    let id: String = (0..22)
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&bytes[index..index + 2], 16)
                .ok()
                .map(char::from)
        })
        .collect::<Option<_>>()?;
    youtube_id(&id).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_keys_preserve_case_and_underscore() {
        let first = youtube_vault_key("AbcDef_h-j0").unwrap();
        let second = youtube_vault_key("abcDef_h-j0").unwrap();
        assert_ne!(first, second);
        assert_eq!(
            youtube_id_from_vault_key(&first).as_deref(),
            Some("AbcDef_h-j0")
        );
        assert!(youtube_vault_key("../bad-url!").is_none());
    }

    #[test]
    fn only_accepts_media_urls_from_the_youtube_cdn() {
        let allowed = |value| allowed_cdn(&Url::parse(value).unwrap());
        assert!(allowed(
            "https://r1.googlevideo.com/videoplayback?id=example"
        ));
        assert!(!allowed(
            "https://googlevideo.com.example.net/videoplayback"
        ));
        assert!(!allowed("https://user@googlevideo.com/videoplayback"));
        assert!(!allowed("https://googlevideo.com:1024/videoplayback"));
    }
}
