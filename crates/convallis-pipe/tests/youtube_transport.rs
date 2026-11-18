use convallis_pipe::{MediaPipe, PlaybackSource, Vault};
use reqwest::{Client, StatusCode};
use std::sync::Arc;

async fn check_range(client: &Client, url: &str) -> Vec<u8> {
    let response = client
        .get(url)
        .header("Range", "bytes=32-63")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert!(response.headers()["content-range"]
        .to_str()
        .unwrap()
        .starts_with("bytes 32-63/"));
    let bytes = response.bytes().await.unwrap().to_vec();
    assert_eq!(bytes.len(), 32);
    bytes
}

async fn check_source(client: &Client, source: &PlaybackSource) -> Vec<u8> {
    let response = client.head(source.url()).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["accept-ranges"], "bytes");
    if let Some(companion) = source.companion_url() {
        check_range(client, companion).await;
    }
    check_range(client, source.url()).await
}

#[tokio::test]
#[ignore = "requires local yt-dlp, internet, and a public test video"]
async fn public_video_uses_loopback_ranges_and_audio_survives_offline() {
    let id =
        std::env::var("NASTI_TEST_YOUTUBE_ID").expect("provide a public video you can download");
    let root = tempfile::tempdir().unwrap();
    let vault = Arc::new(Vault::open(root.path().join("vault")).unwrap());
    let pipe = MediaPipe::start(vec![], vault.clone()).await.unwrap();
    let client = Client::builder().no_proxy().build().unwrap();
    let audio = pipe.youtube_playback(&id, "audio").await.unwrap();
    let video = pipe.youtube_playback(&id, "video").await.unwrap();
    let bytes = check_source(&client, &audio).await;
    let video_bytes = check_source(&client, &video).await;
    println!(
        "Range transport verified, separate MV audio: {}",
        video.companion_url().is_some()
    );

    let download_video = std::env::var_os("NASTI_TEST_VIDEO_VAULT").is_some();
    if download_video {
        let vault = Arc::clone(&vault);
        tokio::task::spawn_blocking(move || vault.set_limit(320 * 1024 * 1024))
            .await
            .unwrap()
            .unwrap();
    }
    pipe.save_youtube(id.clone(), download_video).await.unwrap();
    drop(video);
    drop(audio);
    drop(pipe);
    let offline = MediaPipe::start(vec![], vault).await.unwrap();
    let source = offline.youtube_playback(&id, "audio").await.unwrap();
    assert_eq!(check_source(&client, &source).await, bytes);
    if download_video {
        let video = offline.youtube_playback(&id, "video").await.unwrap();
        assert_eq!(check_source(&client, &video).await, video_bytes);
        println!("Vault reopened the downloaded MV and its companion audio");
    }
}

#[tokio::test]
#[ignore = "requires local yt-dlp and internet"]
async fn public_keyword_search_returns_songs_videos_and_playlists_without_credentials() {
    let root = tempfile::tempdir().unwrap();
    let vault = Arc::new(Vault::open(root.path().join("vault")).unwrap());
    let pipe = MediaPipe::start(vec![], vault).await.unwrap();
    let result = pipe
        .youtube_search("Beethoven Moonlight Sonata", "all")
        .await
        .unwrap();
    assert!(result
        .songs
        .iter()
        .any(|hit| hit.title.is_some() && hit.id.len() == 11));
    assert!(result
        .videos
        .iter()
        .any(|hit| hit.title.is_some() && hit.id.len() == 11));
    assert!(result
        .playlists
        .iter()
        .any(|hit| hit.title.is_some()
            && hit.url.as_deref().is_some_and(|url| url.contains("list="))));
    println!(
        "anonymous search returned {} song, {} video, and {} playlist hits",
        result.songs.len(),
        result.videos.len(),
        result.playlists.len()
    );
}
