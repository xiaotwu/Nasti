use crate::models::{
    PlaybackCancelArgs, PlaybackCommitArgs, PlaybackLoadArgs, PlaybackOutputArgs, PlaybackOutputs,
    PlaybackPlayingArgs, PlaybackPrepareArgs, PlaybackSeekArgs, PlaybackSnapshot,
    PlaybackVideoFrameArgs, PlaybackVolumeArgs, PreparedPlayback,
};
use tauri::{AppHandle, WebviewWindow};

#[tauri::command]
pub(crate) async fn playback_load(
    app: AppHandle,
    args: PlaybackLoadArgs,
) -> Result<PlaybackSnapshot, String> {
    native::load(app, args).await
}

#[tauri::command]
pub(crate) async fn playback_get_state(app: AppHandle) -> Result<PlaybackSnapshot, String> {
    native::state(app).await
}

#[tauri::command]
pub(crate) async fn playback_set_playing(
    app: AppHandle,
    args: PlaybackPlayingArgs,
) -> Result<PlaybackSnapshot, String> {
    native::set_playing(app, args.playing).await
}

#[tauri::command]
pub(crate) async fn playback_seek(
    app: AppHandle,
    args: PlaybackSeekArgs,
) -> Result<PlaybackSnapshot, String> {
    native::seek(app, args.position_ms).await
}

#[tauri::command]
pub(crate) async fn playback_set_volume(
    app: AppHandle,
    args: PlaybackVolumeArgs,
) -> Result<PlaybackSnapshot, String> {
    native::set_volume(app, args.volume).await
}

#[tauri::command]
pub(crate) async fn playback_list_outputs(app: AppHandle) -> Result<PlaybackOutputs, String> {
    native::list_outputs(app).await
}

#[tauri::command]
pub(crate) async fn playback_set_output(
    app: AppHandle,
    args: PlaybackOutputArgs,
) -> Result<PlaybackOutputs, String> {
    native::set_output(app, args.device_id).await
}

#[tauri::command]
pub(crate) async fn playback_set_video_frame(
    window: WebviewWindow,
    args: PlaybackVideoFrameArgs,
) -> Result<(), String> {
    native::set_video_frame(window, args.frame).await
}

#[tauri::command]
pub(crate) async fn playback_prepare(
    app: AppHandle,
    args: PlaybackPrepareArgs,
) -> Result<(), String> {
    native::prepare(app, args).await
}

#[tauri::command]
pub(crate) async fn playback_commit(
    app: AppHandle,
    args: PlaybackCommitArgs,
) -> Result<PreparedPlayback, String> {
    native::commit(app, args).await
}

#[tauri::command]
pub(crate) async fn playback_cancel_preparation(
    app: AppHandle,
    args: PlaybackCancelArgs,
) -> Result<(), String> {
    native::cancel_preparation(app, args.switch_id).await
}

pub(crate) use native::shutdown;

#[cfg(target_os = "macos")]
mod native {
    use crate::catalog_commands;
    use crate::models::{
        MediaMode, PlaybackCommitArgs, PlaybackLoadArgs, PlaybackOutputs, PlaybackPrepareArgs,
        PlaybackSnapshot, PlaybackStatus, PlaybackVideoFrame, PreparedPlayback,
    };
    use convallis_pipe::PlaybackSource;
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::Duration;
    use tauri::{AppHandle, Manager, WebviewWindow};
    use tokio::sync::oneshot;

    static LOAD_REVISION: AtomicU64 = AtomicU64::new(0);
    static PREPARATION: AtomicU64 = AtomicU64::new(0);
    static WINDOW_CLOSED: AtomicBool = AtomicBool::new(false);

    #[repr(C)]
    struct NativePlaybackSource {
        context: *mut c_void,
        finish: extern "C" fn(*mut c_void),
        restart: extern "C" fn(*mut c_void),
        release: extern "C" fn(*mut c_void),
    }

    impl NativePlaybackSource {
        fn transfer(source: PlaybackSource) -> Self {
            // AppKit owns the source until it replaces this AVPlayer item, with
            // finish/restart retaining the URL for normal replay after an item ends.
            Self {
                context: Box::into_raw(Box::new(source)).cast(),
                finish: finish_source,
                restart: restart_source,
                release: release_source,
            }
        }
    }

    extern "C" fn finish_source(context: *mut c_void) {
        unsafe { &*context.cast::<PlaybackSource>() }.finish();
    }

    extern "C" fn restart_source(context: *mut c_void) {
        unsafe { &*context.cast::<PlaybackSource>() }.restart();
    }

    extern "C" fn release_source(context: *mut c_void) {
        drop(unsafe { Box::from_raw(context.cast::<PlaybackSource>()) });
    }

    unsafe extern "C" {
        fn nasti_native_load(
            url: *const c_char,
            audio_url: *const c_char,
            video_id: *const c_char,
            title: *const c_char,
            artist: *const c_char,
            is_video: bool,
            position_ms: u64,
            autoplay: bool,
            source: NativePlaybackSource,
        );
        fn nasti_native_set_playing(playing: bool);
        fn nasti_native_prepare(
            url: *const c_char,
            audio_url: *const c_char,
            video_id: *const c_char,
            title: *const c_char,
            artist: *const c_char,
            is_video: bool,
            source: NativePlaybackSource,
        );
        fn nasti_native_prepare_parked_audio(video_id: *const c_char) -> bool;
        fn nasti_native_copy_preparation() -> *mut c_char;
        fn nasti_native_seek_preparation(position_ms: u64);
        fn nasti_native_commit_preparation(
            autoplay: bool,
            align_time: bool,
            position_ms: u64,
        ) -> bool;
        fn nasti_native_cancel_preparation();
        fn nasti_native_seek(position_ms: u64);
        fn nasti_native_set_volume(volume: f32);
        fn nasti_native_copy_outputs() -> *mut c_char;
        fn nasti_native_set_output(identifier: *const c_char) -> bool;
        fn nasti_native_set_video_frame(
            window: *mut c_void,
            shown: bool,
            x: f64,
            y: f64,
            width: f64,
            height: f64,
        ) -> bool;
        fn nasti_native_copy_snapshot() -> *mut c_char;
        fn nasti_native_free_snapshot(json: *mut c_char);
        fn nasti_native_shutdown();
    }

    async fn on_main<T: Send + 'static>(
        app: AppHandle,
        action: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let (send, receive) = oneshot::channel();
        app.run_on_main_thread(move || {
            let result = if WINDOW_CLOSED.load(Ordering::Acquire) {
                Err("原生播放桥已关闭".to_owned())
            } else {
                action()
            };
            let _ = send.send(result);
        })
        .map_err(|error| error.to_string())?;
        receive.await.map_err(|_| "原生播放桥已关闭".to_owned())?
    }

    // The controller is confined to the AppKit main thread.
    fn snapshot() -> Result<PlaybackSnapshot, String> {
        read_snapshot(unsafe { nasti_native_copy_snapshot() })
    }

    fn read_snapshot(json: *mut c_char) -> Result<PlaybackSnapshot, String> {
        read_native_json(json, "原生播放状态读取失败")
    }

    fn read_native_json<T: serde::de::DeserializeOwned>(
        json: *mut c_char,
        unavailable: &str,
    ) -> Result<T, String> {
        unsafe {
            if json.is_null() {
                return Err(unavailable.into());
            }
            let result = serde_json::from_slice(CStr::from_ptr(json).to_bytes());
            nasti_native_free_snapshot(json);
            result.map_err(|error| error.to_string())
        }
    }

    fn media_label(value: Option<String>) -> CString {
        CString::new(value.unwrap_or_default().replace('\0', "")).expect("NUL was removed")
    }

    pub(super) async fn load(
        app: AppHandle,
        args: PlaybackLoadArgs,
    ) -> Result<PlaybackSnapshot, String> {
        let revision = LOAD_REVISION.fetch_add(1, Ordering::Relaxed) + 1;
        let source =
            catalog_commands::resolve_playback(&app, &args.video_id, args.mode.as_str()).await?;
        let url = CString::new(source.url()).map_err(|_| "授权曲目地址无效".to_owned())?;
        let audio_url = source
            .companion_url()
            .map(CString::new)
            .transpose()
            .map_err(|_| "视频音轨地址无效".to_owned())?;
        let identifier = CString::new(args.video_id).map_err(|_| "曲目标识无效".to_owned())?;
        let title = media_label(args.title);
        let artist = media_label(args.artist);
        on_main(app, move || {
            if LOAD_REVISION.load(Ordering::Relaxed) != revision {
                return Err("已切换到另一首曲目".into());
            }
            // The native controller copies both strings before this call returns.
            unsafe {
                PREPARATION.store(0, Ordering::Release);
                nasti_native_cancel_preparation();
                nasti_native_load(
                    url.as_ptr(),
                    audio_url
                        .as_ref()
                        .map_or(std::ptr::null(), |url| url.as_ptr()),
                    identifier.as_ptr(),
                    title.as_ptr(),
                    artist.as_ptr(),
                    args.mode == MediaMode::Video,
                    args.position_ms.unwrap_or(0),
                    args.autoplay.unwrap_or(false),
                    NativePlaybackSource::transfer(source),
                );
            }
            snapshot()
        })
        .await
    }

    fn check_preparation(id: u64) -> Result<(), String> {
        if id == 0 || PREPARATION.load(Ordering::Acquire) != id {
            return Err("媒体切换已取消".into());
        }
        Ok(())
    }

    async fn wait_until_ready(app: AppHandle, id: u64) -> Result<(), String> {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let state = on_main(app.clone(), move || {
                    check_preparation(id)?;
                    read_snapshot(unsafe { nasti_native_copy_preparation() })
                })
                .await?;
                match state.status {
                    PlaybackStatus::Ready | PlaybackStatus::Paused | PlaybackStatus::Ended => {
                        return Ok(())
                    }
                    PlaybackStatus::Error => {
                        return Err(state.error.unwrap_or_else(|| "媒体暂时无法准备".into()))
                    }
                    _ => tokio::time::sleep(Duration::from_millis(45)).await,
                }
            }
        })
        .await
        .map_err(|_| "媒体准备超时".to_owned())?
    }

    pub(super) async fn prepare(app: AppHandle, args: PlaybackPrepareArgs) -> Result<(), String> {
        let id = args.switch_id;
        if id == 0 {
            return Err("媒体切换标识无效".into());
        }
        PREPARATION.store(id, Ordering::Release);
        let result = async {
            if args.mode == MediaMode::Audio {
                let id_text =
                    CString::new(args.video_id.as_str()).map_err(|_| "曲目标识无效".to_owned())?;
                let parked = on_main(app.clone(), move || {
                    check_preparation(id)?;
                    Ok(unsafe { nasti_native_prepare_parked_audio(id_text.as_ptr()) })
                })
                .await?;
                if parked {
                    return wait_until_ready(app.clone(), id).await;
                }
            }
            let source =
                catalog_commands::resolve_playback(&app, &args.video_id, args.mode.as_str())
                    .await?;
            let url = CString::new(source.url()).map_err(|_| "授权曲目地址无效".to_owned())?;
            let audio_url = source
                .companion_url()
                .map(CString::new)
                .transpose()
                .map_err(|_| "视频音轨地址无效".to_owned())?;
            let video_id = CString::new(args.video_id).map_err(|_| "曲目标识无效".to_owned())?;
            let title = media_label(args.title);
            let artist = media_label(args.artist);
            on_main(app.clone(), move || {
                check_preparation(id)?;
                unsafe {
                    nasti_native_prepare(
                        url.as_ptr(),
                        audio_url
                            .as_ref()
                            .map_or(std::ptr::null(), |url| url.as_ptr()),
                        video_id.as_ptr(),
                        title.as_ptr(),
                        artist.as_ptr(),
                        args.mode == MediaMode::Video,
                        NativePlaybackSource::transfer(source),
                    )
                };
                Ok(())
            })
            .await?;
            wait_until_ready(app.clone(), id).await
        }
        .await;
        if result.is_err() {
            let _ = cancel_preparation(app, id).await;
        }
        result
    }

    pub(super) async fn commit(
        app: AppHandle,
        args: PlaybackCommitArgs,
    ) -> Result<PreparedPlayback, String> {
        let id = args.switch_id;
        let result = async {
            on_main(app.clone(), move || {
                check_preparation(id)?;
                let target = read_snapshot(unsafe { nasti_native_copy_preparation() })?;
                let current = snapshot()?;
                let same = args.align_time
                    && current.video_id == target.video_id
                    && current.status != PlaybackStatus::Ended;
                let position = if same {
                    current.position_ms
                } else {
                    args.position_ms
                };
                if position > 0 {
                    unsafe { nasti_native_seek_preparation(position) };
                }
                Ok(())
            })
            .await?;
            wait_until_ready(app.clone(), id).await?;
            on_main(app.clone(), move || {
                check_preparation(id)?;
                let target = read_snapshot(unsafe { nasti_native_copy_preparation() })?;
                let previous = snapshot()?;
                let align = args.align_time
                    && previous.video_id == target.video_id
                    && previous.status != PlaybackStatus::Ended;
                PREPARATION
                    .compare_exchange(id, 0, Ordering::AcqRel, Ordering::Acquire)
                    .map_err(|_| "媒体切换已取消".to_owned())?;
                let autoplay = args.autoplay.unwrap_or(matches!(
                    previous.status,
                    PlaybackStatus::Playing | PlaybackStatus::Loading
                ));
                if !unsafe {
                    nasti_native_commit_preparation(autoplay, align, previous.position_ms)
                } {
                    unsafe { nasti_native_cancel_preparation() };
                    return Err("媒体还未就绪".into());
                }
                Ok(PreparedPlayback {
                    playback: snapshot()?,
                    previous,
                })
            })
            .await
        }
        .await;
        if result.is_err() {
            let _ = cancel_preparation(app, id).await;
        }
        result
    }

    pub(super) async fn cancel_preparation(app: AppHandle, id: u64) -> Result<(), String> {
        on_main(app, move || {
            if PREPARATION
                .compare_exchange(id, 0, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                unsafe { nasti_native_cancel_preparation() };
            }
            Ok(())
        })
        .await
    }

    pub(super) async fn state(app: AppHandle) -> Result<PlaybackSnapshot, String> {
        on_main(app, snapshot).await
    }

    pub(super) async fn set_playing(
        app: AppHandle,
        playing: bool,
    ) -> Result<PlaybackSnapshot, String> {
        on_main(app, move || {
            unsafe { nasti_native_set_playing(playing) };
            snapshot()
        })
        .await
    }

    pub(super) async fn seek(app: AppHandle, position_ms: u64) -> Result<PlaybackSnapshot, String> {
        on_main(app, move || {
            unsafe { nasti_native_seek(position_ms) };
            snapshot()
        })
        .await
    }

    pub(super) async fn set_volume(
        app: AppHandle,
        volume: f32,
    ) -> Result<PlaybackSnapshot, String> {
        if !volume.is_finite() {
            return Err("音量值无效".into());
        }
        on_main(app, move || {
            unsafe { nasti_native_set_volume(volume.clamp(0.0, 1.0)) };
            snapshot()
        })
        .await
    }

    fn audio_outputs() -> Result<PlaybackOutputs, String> {
        read_native_json(unsafe { nasti_native_copy_outputs() }, "无法读取播放设备")
    }

    pub(super) async fn list_outputs(app: AppHandle) -> Result<PlaybackOutputs, String> {
        on_main(app, audio_outputs).await
    }

    pub(super) async fn set_output(
        app: AppHandle,
        identifier: Option<String>,
    ) -> Result<PlaybackOutputs, String> {
        let identifier = identifier
            .map(CString::new)
            .transpose()
            .map_err(|_| "播放设备标识无效".to_owned())?;
        on_main(app, move || {
            let device = identifier
                .as_ref()
                .map_or(std::ptr::null(), |id| id.as_ptr());
            if unsafe { nasti_native_set_output(device) } {
                audio_outputs()
            } else {
                Err("播放设备已断开".into())
            }
        })
        .await
    }

    pub(super) async fn set_video_frame(
        window: WebviewWindow,
        frame: Option<PlaybackVideoFrame>,
    ) -> Result<(), String> {
        if window.label() != "main" {
            return Err("视频画面仅支持主窗口".into());
        }
        on_main(window.app_handle().clone(), move || {
            let Some(frame) = frame else {
                unsafe {
                    nasti_native_set_video_frame(std::ptr::null_mut(), false, 0.0, 0.0, 0.0, 0.0)
                };
                return Ok(());
            };
            if ![frame.x, frame.y, frame.width, frame.height]
                .into_iter()
                .all(f64::is_finite)
            {
                return Err("视频画面坐标无效".into());
            }
            let surface = window.ns_window().map_err(|error| error.to_string())?;
            // The window pointer is valid while the WebviewWindow remains alive on this thread.
            let shown = unsafe {
                nasti_native_set_video_frame(
                    surface,
                    true,
                    frame.x,
                    frame.y,
                    frame.width,
                    frame.height,
                )
            };
            shown
                .then_some(())
                .ok_or_else(|| "原生视频画面不可用".into())
        })
        .await
    }

    pub(crate) fn shutdown() {
        WINDOW_CLOSED.store(true, Ordering::Release);
        LOAD_REVISION.fetch_add(1, Ordering::Relaxed);
        PREPARATION.store(0, Ordering::Release);
        // The native shutdown entry point dispatches to AppKit when necessary.
        unsafe { nasti_native_shutdown() };
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    use crate::models::{
        PlaybackCommitArgs, PlaybackLoadArgs, PlaybackOutputs, PlaybackPrepareArgs,
        PlaybackSnapshot, PlaybackVideoFrame, PreparedPlayback,
    };
    use tauri::{AppHandle, WebviewWindow};

    const UNAVAILABLE: &str = "此版本的原生播放桥支持 macOS";

    pub(super) async fn load(
        _: AppHandle,
        _: PlaybackLoadArgs,
    ) -> Result<PlaybackSnapshot, String> {
        Err(UNAVAILABLE.into())
    }

    pub(super) async fn state(_: AppHandle) -> Result<PlaybackSnapshot, String> {
        Err(UNAVAILABLE.into())
    }

    pub(super) async fn prepare(_: AppHandle, _: PlaybackPrepareArgs) -> Result<(), String> {
        Err(UNAVAILABLE.into())
    }

    pub(super) async fn commit(
        _: AppHandle,
        _: PlaybackCommitArgs,
    ) -> Result<PreparedPlayback, String> {
        Err(UNAVAILABLE.into())
    }

    pub(super) async fn cancel_preparation(_: AppHandle, _: u64) -> Result<(), String> {
        Ok(())
    }

    pub(super) async fn set_playing(_: AppHandle, _: bool) -> Result<PlaybackSnapshot, String> {
        Err(UNAVAILABLE.into())
    }

    pub(super) async fn seek(_: AppHandle, _: u64) -> Result<PlaybackSnapshot, String> {
        Err(UNAVAILABLE.into())
    }

    pub(super) async fn set_volume(_: AppHandle, _: f32) -> Result<PlaybackSnapshot, String> {
        Err(UNAVAILABLE.into())
    }

    pub(super) async fn list_outputs(_: AppHandle) -> Result<PlaybackOutputs, String> {
        Ok(PlaybackOutputs {
            supported: false,
            selected_device_id: None,
            default_device_id: None,
            devices: Vec::new(),
        })
    }

    pub(super) async fn set_output(
        _: AppHandle,
        _: Option<String>,
    ) -> Result<PlaybackOutputs, String> {
        Err(UNAVAILABLE.into())
    }

    pub(super) async fn set_video_frame(
        _: WebviewWindow,
        _: Option<PlaybackVideoFrame>,
    ) -> Result<(), String> {
        Err(UNAVAILABLE.into())
    }

    pub(crate) fn shutdown() {}
}
