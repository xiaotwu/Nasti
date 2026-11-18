//! Load-Bearing Crew (应急承重小组) — Unified macOS & Windows 11 Tauri v2 Host

mod app_update;
#[cfg(target_os = "macos")]
mod app_update_install;
mod catalog_commands;
mod playback;
mod runtime_storage;
mod ui_locale;
mod youtube_commands;
mod youtube_oauth;

pub use uraniborg_core::models;

pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(target_os = "macos")]
    let builder = builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(app_update::AppUpdater::default());
    builder
        .setup(catalog_commands::setup)
        .on_window_event(|window, event| {
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
                playback::shutdown();
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_update::app_update_check,
            app_update::app_update_install,
            ui_locale::ui_set_language,
            catalog_commands::catalog_fetch_columns,
            catalog_commands::catalog_search,
            catalog_commands::vault_download_track,
            catalog_commands::vault_remove_track,
            catalog_commands::vault_get_storage_stats,
            catalog_commands::vault_list_tracks,
            catalog_commands::vault_set_storage_limit,
            catalog_commands::lyrics_get_synced,
            playback::playback_load,
            playback::playback_get_state,
            playback::playback_set_playing,
            playback::playback_seek,
            playback::playback_set_volume,
            playback::playback_list_outputs,
            playback::playback_set_output,
            playback::playback_set_video_frame,
            playback::playback_prepare,
            playback::playback_commit,
            playback::playback_cancel_preparation,
            youtube_commands::youtube_playlist_action,
        ])
        .run(tauri::generate_context!())
        .expect("Nasti desktop host failed");
}
