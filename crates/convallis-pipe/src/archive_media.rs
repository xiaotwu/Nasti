use reqwest::header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_RANGE, CONTENT_TYPE, RANGE};
use reqwest::{Client, Response, StatusCode, Url};
use std::time::Duration;

const UA: &str = concat!("NASTI/", env!("CARGO_PKG_VERSION"), " (unfoundsound audio)");
pub(crate) const MP3_MIME: &str = "audio/mpeg";

#[derive(Clone)]
pub struct ArchiveMedia {
    url: Url,
    pub(crate) size: u64,
    pub(crate) sha1: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ArchiveMediaError {
    #[error("Internet Archive 曲目文件信息无效")]
    InvalidFile,
    #[error("无法连接 Internet Archive 音源")]
    Network(#[source] reqwest::Error),
    #[error("Internet Archive 请求过多，请稍后重试")]
    RateLimited,
    #[error("Internet Archive 未返回与已登记曲目匹配的音频")]
    UnexpectedResponse,
}

impl From<reqwest::Error> for ArchiveMediaError {
    fn from(error: reqwest::Error) -> Self {
        Self::Network(error.without_url())
    }
}

impl ArchiveMedia {
    pub fn new(
        item: &str,
        filename: &str,
        size: u64,
        sha1: &str,
    ) -> Result<Self, ArchiveMediaError> {
        let valid_item = item.len() <= 100
            && item
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
            && item
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'));
        let valid_file = filename.to_ascii_lowercase().ends_with(".mp3")
            && !filename.contains('\\')
            && !filename.chars().any(char::is_control)
            && !filename
                .split('/')
                .any(|part| matches!(part, "" | "." | ".."));
        if !valid_item
            || !valid_file
            || size == 0
            || sha1.len() != 40
            || !sha1.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(ArchiveMediaError::InvalidFile);
        }
        let mut url = Url::parse("https://archive.org/download/").unwrap();
        url.path_segments_mut()
            .unwrap()
            .pop_if_empty()
            .push(item)
            .extend(filename.split('/'));
        Ok(Self {
            url,
            size,
            sha1: sha1.to_ascii_lowercase(),
        })
    }
}

pub(crate) struct ArchiveHttp {
    client: Client,
}

impl ArchiveHttp {
    pub(crate) fn new() -> Result<Self, ArchiveMediaError> {
        let client = Client::builder()
            .user_agent(UA)
            .https_only(true)
            .connect_timeout(Duration::from_secs(8))
            .read_timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::custom(|redirect| {
                if redirect.previous().len() >= 5 || !storage_url(redirect.url()) {
                    return redirect.error("Unexpected Internet Archive redirect");
                }
                redirect.follow()
            }))
            .build()?;
        Ok(Self { client })
    }

    /// The declared size must match the archive file even if the caller only requested a probe.
    pub(crate) async fn get(
        &self,
        media: &ArchiveMedia,
        range: Option<(u64, u64)>,
    ) -> Result<Response, ArchiveMediaError> {
        let mut request = self
            .client
            .get(media.url.clone())
            .header(ACCEPT_ENCODING, "identity");
        if let Some((start, end)) = range {
            request = request.header(RANGE, format!("bytes={start}-{end}"));
        }
        let response = request.send().await?;
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            return Err(ArchiveMediaError::RateLimited);
        }
        let size = match range {
            Some((start, end)) => {
                let expected_range = format!("bytes {start}-{end}/{}", media.size);
                if response.status() != StatusCode::PARTIAL_CONTENT
                    || header(&response, CONTENT_RANGE) != Some(expected_range.as_str())
                {
                    return Err(ArchiveMediaError::UnexpectedResponse);
                }
                end - start + 1
            }
            None if response.status() == StatusCode::OK => media.size,
            _ => return Err(ArchiveMediaError::UnexpectedResponse),
        };
        let mime = header(&response, CONTENT_TYPE)
            .and_then(|value| value.split(';').next())
            .map(str::trim);
        if !matches!(mime, Some(MP3_MIME | "application/octet-stream"))
            || header(&response, CONTENT_ENCODING).is_some_and(|encoding| encoding != "identity")
            || response
                .content_length()
                .is_some_and(|length| length != size)
        {
            return Err(ArchiveMediaError::UnexpectedResponse);
        }
        Ok(response)
    }
}

fn header(response: &Response, key: reqwest::header::HeaderName) -> Option<&str> {
    response.headers().get(key)?.to_str().ok()
}

fn storage_url(url: &Url) -> bool {
    if url.scheme() != "https"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    if host == "archive.org" {
        return url.path().starts_with("/download/");
    }
    let mut parts = host.split('.');
    let (Some(node), Some(region), Some("archive"), Some("org"), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return false;
    };
    let Some(number) = node.strip_prefix("ia").or_else(|| node.strip_prefix("dn")) else {
        return false;
    };
    if number.is_empty()
        || !number.bytes().all(|value| value.is_ascii_digit())
        || region.len() != 2
        || !region.bytes().all(|value| value.is_ascii_lowercase())
    {
        return false;
    }
    let Some(mut segments) = url.path_segments() else {
        return false;
    };
    segments
        .next()
        .is_some_and(|dir| !dir.is_empty() && dir.bytes().all(|value| value.is_ascii_digit()))
        && segments.next() == Some("items")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_media_url_and_confines_redirects_to_the_archive() {
        let digest = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let media = ArchiveMedia::new("unfound88", "Tracks/1 # name.mp3", 10, digest).unwrap();
        assert_eq!(
            media.url.as_str(),
            "https://archive.org/download/unfound88/Tracks/1%20%23%20name.mp3"
        );
        assert!(ArchiveMedia::new("unfound88", "../../music.mp3", 10, digest).is_err());
        assert!(ArchiveMedia::new("unfound88", "music.wav", 10, digest).is_err());
        for allowed in [
            "https://dn711202.ca.archive.org/0/items/unfound88/music.mp3",
            "https://ia802304.us.archive.org/30/items/unfound88/music.mp3",
        ] {
            assert!(storage_url(&Url::parse(allowed).unwrap()));
        }
        for unexpected in [
            "http://dn711202.ca.archive.org/0/items/unfound88/music.mp3",
            "https://dn711202.ca.archive.org.example.com/0/items/unfound88/music.mp3",
            "https://www.archive.org/0/items/unfound88/music.mp3",
            "https://dn711202.ca.archive.org/other/internal",
            "https://archive.org/account/login",
        ] {
            assert!(!storage_url(&Url::parse(unexpected).unwrap()));
        }
    }
}
