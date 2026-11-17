use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};

static ENGLISH: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Deserialize)]
pub(crate) enum Language {
    #[serde(rename = "zh-CN")]
    Chinese,
    #[serde(rename = "en")]
    English,
}

impl Language {
    pub(crate) fn current() -> Self {
        if ENGLISH.load(Ordering::Relaxed) {
            Self::English
        } else {
            Self::Chinese
        }
    }

    pub(crate) fn choose<'a>(self, chinese: &'a str, english: &'a str) -> &'a str {
        match self {
            Self::Chinese => chinese,
            Self::English => english,
        }
    }
}

#[tauri::command]
pub(crate) async fn ui_set_language(
    window: tauri::WebviewWindow,
    language: Language,
) -> Result<(), String> {
    ENGLISH.store(matches!(language, Language::English), Ordering::Relaxed);
    window
        .set_title(language.choose("娜斯提 · Nasti", "Nasti"))
        .map_err(|error| error.to_string())?;
    #[cfg(target_os = "macos")]
    window
        .app_handle()
        .set_menu(macos_menu(window.app_handle(), language).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(target_os = "macos")]
use tauri::Manager;

#[cfg(target_os = "macos")]
fn macos_menu(
    app: &tauri::AppHandle,
    language: Language,
) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{
        AboutMetadata, Menu, PredefinedMenuItem as Item, Submenu, HELP_SUBMENU_ID,
        WINDOW_SUBMENU_ID,
    };

    let label = |zh, en| Some(language.choose(zh, en));
    let about = AboutMetadata {
        name: Some("Nasti".into()),
        version: Some(app.package_info().version.to_string()),
        copyright: app.config().bundle.copyright.clone(),
        authors: app
            .config()
            .bundle
            .publisher
            .clone()
            .map(|publisher| vec![publisher]),
        ..Default::default()
    };
    let app_menu = Submenu::with_items(
        app,
        "Nasti",
        true,
        &[
            &Item::about(app, label("关于 Nasti", "About Nasti"), Some(about))?,
            &Item::separator(app)?,
            &Item::services(app, label("服务", "Services"))?,
            &Item::separator(app)?,
            &Item::hide(app, label("隐藏 Nasti", "Hide Nasti"))?,
            &Item::hide_others(app, label("隐藏其他应用", "Hide Others"))?,
            &Item::separator(app)?,
            &Item::quit(app, label("退出 Nasti", "Quit Nasti"))?,
        ],
    )?;
    let file = Submenu::with_items(
        app,
        language.choose("文件", "File"),
        true,
        &[&Item::close_window(app, label("关闭窗口", "Close Window"))?],
    )?;
    let edit = Submenu::with_items(
        app,
        language.choose("编辑", "Edit"),
        true,
        &[
            &Item::undo(app, label("撤销", "Undo"))?,
            &Item::redo(app, label("重做", "Redo"))?,
            &Item::separator(app)?,
            &Item::cut(app, label("剪切", "Cut"))?,
            &Item::copy(app, label("复制", "Copy"))?,
            &Item::paste(app, label("粘贴", "Paste"))?,
            &Item::select_all(app, label("全选", "Select All"))?,
        ],
    )?;
    let view = Submenu::with_items(
        app,
        language.choose("显示", "View"),
        true,
        &[&Item::fullscreen(
            app,
            label("切换全屏", "Toggle Full Screen"),
        )?],
    )?;
    let window = Submenu::with_id_and_items(
        app,
        WINDOW_SUBMENU_ID,
        language.choose("窗口", "Window"),
        true,
        &[
            &Item::minimize(app, label("最小化", "Minimize"))?,
            &Item::maximize(app, label("缩放", "Zoom"))?,
            &Item::separator(app)?,
            &Item::close_window(app, label("关闭窗口", "Close Window"))?,
        ],
    )?;
    let help = Submenu::with_id_and_items(
        app,
        HELP_SUBMENU_ID,
        language.choose("帮助", "Help"),
        true,
        &[],
    )?;
    Menu::with_items(app, &[&app_menu, &file, &edit, &view, &window, &help])
}
