# Nasti Beta

Nasti is a macOS music player built with Tauri, Rust and Three.js. This first beta starts with an empty library. On Apple Silicon, launch the app and use the search drawer to access the supported licensed Internet Archive catalog. YouTube Music playlist management requires your own Google OAuth setup and opens playback in YouTube Music. The beta uses a separate application identifier and storage from the local development app.

Nasti 是基于 Tauri、Rust 和 Three.js 的 macOS 音乐播放器。首个 beta 面向 Apple Silicon，首次打开时曲库为空。可在检索抽屉访问支持的 Internet Archive 授权目录。YouTube Music 歌单管理需要自行配置 OAuth，曲目由 YouTube Music 打开。beta 使用独立的应用标识和数据目录。

[Releases / 下载](https://github.com/xiaotwu/Nasti/releases) · [Roadmap / 看板](https://github.com/users/xiaotwu/projects/5/views/2) · [Issues / 问题](https://github.com/xiaotwu/Nasti/issues)

Build using Node, pnpm 11, Rust and Xcode. Run `pnpm install --frozen-lockfile` and then `pnpm --dir hosts/load-bearing-crew exec tauri build --bundles app`.

Source is GPL-3.0-or-later. Archive model by LBEILC/RhineLabUI, used under MIT. See [LICENSE](LICENSE) and the [model notice](assets/foundry-art/desktop/ARCHIVE-LICENSE.txt).
