# Nasti

Nasti is a macOS music player built with Tauri, Rust and Three.js. On Apple Silicon, use the search drawer to access the supported licensed Internet Archive catalog. When Google sign-in is available, Nasti uses your authorization to manage YouTube Music playlists. YouTube Music opens playback in the browser. Upgrades from the prerelease keep the same app identifier and library.

Nasti 是基于 Tauri、Rust 和 Three.js 的 macOS 音乐播放器，支持 Apple Silicon。可在检索抽屉访问 Internet Archive 授权目录。Google 登录可用时，Nasti 使用你的账号授权来管理 YouTube Music 歌单，曲目在浏览器中播放。从预发行版升级时沿用应用标识和曲库。

## Library and account / 曲库与账号

- **Licensed catalog / 授权曲库** — use the top search control to select the licensed catalog, search, preview and import tracks. You can also paste a supported Internet Archive release link into Import Playlist. 从顶栏检索入口选择授权曲库，搜索、预览并导入曲目。也可在「导入歌单」粘贴受支持的 Internet Archive 专辑链接。
- **YouTube Music** — open Import Playlist → YouTube Music → Sign in with Google. Authorization opens in the system browser and returns to Nasti; no client JSON selection is required. If this build has no publisher-provided app identity, sign-in is unavailable. 从顶栏「导入歌单」进入 YouTube Music，选择「使用 Google 登录」，在系统浏览器授权后返回 Nasti。无需选择客户端 JSON；未配置应用身份的构建会显示登录尚未启用。播放会打开 YouTube Music，导入及管理在 Nasti 内完成。

[Releases / 下载](https://github.com/xiaotwu/Nasti/releases) · [Roadmap / 看板](https://github.com/users/xiaotwu/projects/5/views/2) · [Issues / 问题](https://github.com/xiaotwu/Nasti/issues)

Build using Node, pnpm 11, Rust and Xcode. Run `pnpm install --frozen-lockfile` and then `pnpm --dir hosts/load-bearing-crew exec tauri build --bundles app`.

Desktop update and cache behavior is described in [native storage and releases](docs/NATIVE_STORAGE.md). 关于应用内更新和本机缓存目录，见[桌面版更新与存储](docs/NATIVE_STORAGE.md)。

Publisher setup and Google browser restrictions are described in [Google sign-in](docs/GOOGLE_SIGN_IN.md). 发布者客户端配置与 Google 浏览器限制见 [Google 登录说明](docs/GOOGLE_SIGN_IN.md)。

Source is GPL-3.0-or-later. Archive model by LBEILC/RhineLabUI, used under MIT. See [LICENSE](LICENSE) and the [model notice](assets/foundry-art/desktop/ARCHIVE-LICENSE.txt).
