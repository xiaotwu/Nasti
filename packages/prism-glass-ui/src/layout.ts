import { version as appVersion } from "../package.json";
import { button, icon, type IconName } from "./icons";
import { glassFilters } from "./materials/glass";
import { t } from "./i18n";
import { shortcutSettings } from "./shortcuts";

export const artUrl = "/foundry-art/nasti-illust/nasti_e2_reference.jpg";
const bootArtwork = "/foundry-art/desktop/boot-prism-reveal.png";

const settingSelect = (label: string, id: string, options: readonly (readonly [string, string])[]) => `
  <label class="setting-row"><strong>${label}</strong><select id="${id}">
    ${options.map(([value, title]) => `<option value="${value}">${title}</option>`).join("")}
  </select></label>`;

const youtubeButton = (glyph: IconName, action: string, label: string, extra = "") =>
  `<button type="button" class="icon-button" data-youtube-action="${action}" aria-label="${label}" title="${label}" ${extra}>${icon(glyph)}</button>`;

export function mountLayout(root: HTMLElement): void {
  const endOptions = [["loop", t("循环", "Wrap around")], ["stop", t("首尾停止", "Stop at ends")]] as const;
  const youtubePrivacyOptions = `<option value="private">${t("私密", "Private")}</option>
    <option value="unlisted">${t("不公开", "Unlisted")}</option><option value="public">${t("公开", "Public")}</option>`;
  const settingsSections = [
    ["appearance", t("外观", "Appearance"), `
      ${settingSelect(t("语言", "Language"), "language-setting", [["zh-CN", t("简体中文", "Simplified Chinese")], ["en", t("英语", "English")]])}
      ${settingSelect(t("界面主题", "Theme"), "theme-setting", [["celestial-cyan", t("浅色", "Light")], ["banshee-obsidian", t("深色", "Dark")], ["system", t("跟随系统", "Follow system")]])}
      <label class="setting-row"><strong>${t("远景剪影", "Background silhouette")}</strong><input type="checkbox" id="silhouette-setting"></label>`],
    ["playback", t("播放", "Playback"), settingSelect(t("当前曲目播完后", "When a track ends"), "end-setting", [["continue", t("继续下一首", "Play next track")], ["stop", t("停在当前曲", "Stop playback")]])],
    ["interaction", t("交互", "Interaction"), `
      ${settingSelect(t("歌曲左右切换", "Track navigation"), "track-wrap-setting", endOptions)}
      ${settingSelect(t("歌单上下切换", "Playlist navigation"), "list-wrap-setting", endOptions)}
      ${settingSelect(t("动态效果", "Motion"), "motion-setting", [["standard", t("标准", "Standard")], ["reduced", t("减少动态", "Reduce motion")]])}
      <div class="setting-row"><strong>${t("开机动画", "Startup animation")}</strong>${button("restart", "replay-boot", t("重播开机动画", "Replay startup animation"))}</div>`],
    ["shortcuts", t("快捷键", "Shortcuts"), shortcutSettings()],
    ["sound", t("声音", "Sound"), `
      <div class="setting-row"><label for="settings-volume"><strong>${t("主音量", "Main volume")}</strong></label>
        <div class="setting-volume"><span class="volume-range-hit"><input class="range" id="settings-volume" type="range" min="0" max="100" step="1"></span><output id="settings-volume-value" for="settings-volume"></output>${button("volume", "mute", t("静音", "Mute"))}</div>
      </div>`],
    ["performance", t("性能", "Performance"), `
      ${settingSelect(t("三维画质", "3D quality"), "quality-setting", [["performance", t("性能", "Performance")], ["balanced", t("均衡", "Balanced")], ["high", t("高", "High")], ["ultra", t("极高", "Ultra")]])}
      <label class="setting-row"><strong>${t("节能模式", "Power saving")}</strong><input type="checkbox" id="power-setting"></label>`],
    ["library", t("曲库", "Library"), `
      <div class="setting-row"><strong>${t("导入歌单", "Import playlist")}</strong>${button("import", "import-playlist", t("打开导入抽屉", "Open playlist import"))}</div>
      <div class="setting-row" data-youtube-entry hidden><strong>YouTube Music</strong>${button("music", "youtube-playlists", t("管理 YouTube Music 歌单", "Manage YouTube Music playlists"))}</div>
      <div class="setting-row"><div><strong>Vault</strong><p id="settings-storage"></p></div>${button("storage", "vault-capacity", t("查看容量", "View storage"))}</div>`],
    ["about", t("关于", "About"), `
      <div class="setting-row"><strong>Nasti</strong><span>${appVersion} · GPL-3.0-or-later</span></div>
      <div class="setting-row settings-update" id="settings-update" hidden>
        <div>
          <strong>${t("软件更新", "Software update")}</strong>
          <p id="settings-update-status" role="status" aria-atomic="true"></p>
        </div>
        <div class="settings-update-controls" id="settings-update-controls">
          <button type="button" class="text-button" id="settings-update-check">${t("检查更新", "Check for updates")}</button>
          <button type="button" class="text-button" id="settings-update-install" hidden>${t("更新并重启", "Update &amp; restart")}</button>
        </div>
      </div>
      <div class="setting-row about-links"><strong>${t("许可与素材", "Licenses and assets")}</strong><div>
        <a href="https://www.gnu.org/licenses/gpl-3.0.html" target="_blank" rel="noopener noreferrer">${t("应用许可", "App license")}</a>
        <a href="https://github.com/LBEILC/RhineLabUI" target="_blank" rel="noopener noreferrer">RhineLabUI</a>
        <a class="rhine-source" href="https://ak.hypergryph.com/cvintro" target="_blank" rel="noopener noreferrer"
          aria-label="${t("明日方舟官网 · 莱茵生命素材来源", "Arknights website · Rhine Lab assets")}" title="${t("明日方舟官网 · 莱茵生命素材来源", "Arknights website · Rhine Lab assets")}"></a>
      </div></div>`],
  ] as const;

  root.innerHTML = `
    ${glassFilters}
    <main class="terminal" id="terminal" data-mode="archive-array" data-theme="banshee-obsidian">
      <div class="scene" id="scene"></div>
      <div class="scene-vignette" aria-hidden="true"></div>
      <header class="shell-top" id="shell-top">
        <span class="wordmark" role="img" aria-label="${t("Nasti · 莱茵生命", "Nasti · Rhine Lab")}">
          <img src="/foundry-art/desktop/header-lockup-transparent.webp" alt=""
            width="576" height="360" draggable="false">
        </span>
        <nav class="glass capsule search-capsule" aria-label="${t("检索与浏览", "Search and browse")}">
          ${button("back", "close-detail", t("返回档案阵列（Esc）", "Back to archive (Esc)"), "hidden")}
          ${button("search", "search", t("检索曲库", "Search library"))}
          ${button("radar", "radar", t("曲库总览", "Library overview"), 'aria-pressed="false"')}
        </nav>
        <nav class="glass capsule terminal-capsule" aria-label="${t("终端", "Terminal")}">
          ${button("library", "library", t("离线储藏库", "Vault"))}
          ${button("import", "import-playlist", t("导入歌单", "Import playlist"))}
          ${button("settings", "settings", t("终端设置", "Settings"))}
        </nav>
        <nav class="glass capsule detail-capsule" id="detail-tabs" aria-label="${t("详情视图", "Detail views")}" hidden>
          ${button("list", "tracklist", t("列表", "Track list"), 'aria-pressed="false" aria-controls="tracklist"')}
          ${button("lyrics", "lyrics", t("歌词", "Lyrics"), 'aria-pressed="true" aria-controls="lyrics"')}
          ${button("video", "mv-stage", t("视频", "Video"), 'aria-pressed="false" aria-controls="mv-stage"')}
        </nav>
      </header>

      <section class="radar" id="radar" aria-label="${t("曲库总览", "Library overview")}" hidden>
        <aside class="overview-sidebar">
          <h2>${t("歌单", "Playlists")}</h2>
          <div id="radar-columns"></div>
        </aside>
        <div class="overview-content">
          <header class="overview-title"><h1 id="radar-title"></h1><div id="overview-actions"></div></header>
          <div class="overview-tracks" id="overview-tracks"></div>
          <div id="overview-batch" hidden></div>
        </div>
      </section>

      <aside class="silhouette" id="companion" aria-hidden="true" hidden></aside>
      <section class="detail" id="detail" aria-label="${t("歌曲详情", "Track details")}" hidden>
        <div class="detail-inner" id="detail-inner">
          <div class="tracklist" id="tracklist" hidden tabindex="0" aria-label="${t("歌单曲目", "Playlist tracks")}">
            <div id="detail-tracks"></div>
          </div>
          <div class="lyric-stage" id="lyrics" aria-label="${t("同步歌词", "Synced lyrics")}" tabindex="0"></div>
          <div class="mv-stage" id="mv-stage" aria-label="${t("音乐影像", "Music videos")}" hidden>
            <div class="mv-viewport" id="mv-viewport">
              <img id="mv-poster" alt="" draggable="false">
              <div class="mv-overlay" id="mv-overlay">
                <p id="mv-hint" role="status"></p>
                ${button("play", "start-mv", t("播放 MV", "Play MV"))}
              </div>
            </div>
          </div>
        </div>
      </section>
      <nav class="inspection-tools" id="inspection-tools" aria-label="${t("档案盒检查", "Cassette inspection")}" hidden>
        <div class="viewer-heading">
          ${button("back", "leave-inspection", t("返回详情", "Back to track details"))}
          <h1>${t("档案拆解", "Cassette disassembly")}</h1><span id="viewer-track"></span>
        </div>
        <div class="glass capsule viewer-controls">
          ${button("cube", "assemble", t("合拢档案", "Assemble cassette"), 'aria-pressed="true"')}
          ${button("explode", "explode", t("拆解档案", "Disassemble cassette"), 'aria-pressed="false"')}
          ${button("restart", "reset-view", t("重置视角", "Reset view"))}
        </div>
      </nav>

      <footer class="transport-dock" id="transport-dock">
        <div class="scene-caption" id="scene-caption" hidden>
          <div><small>${t("当前 / 总数", "Selected / Total")}</small><p><strong id="scene-count"></strong><span id="scene-total"></span></p></div>
          <div><h1 id="scene-column"></h1><p id="scene-owner"></p></div>
        </div>
        <div class="glass capsule transport" aria-label="${t("播放控制", "Playback controls")}">
          <div class="transport-actions">
            ${button("shuffle", "shuffle-tracks", t("随机播放", "Shuffle"), 'aria-pressed="false"')}
            ${button("skipPrevious", "previous", t("上一首", "Previous track"))}
            ${button("play", "play-toggle", t("播放（空格）", "Play (Space)"))}
            ${button("skipNext", "next", t("下一首", "Next track"))}
            ${button("repeat", "repeat-tracks", t("循环已关闭", "Repeat off"), 'aria-pressed="false"')}
          </div>
          <span class="transport-idle-mark" id="transport-idle-mark" role="img" aria-label="Nasti"></span>
          <div class="transport-center">
            <div class="transport-now" id="transport-now" hidden>
              <div class="now-art">
                ${icon("music")}
                <img class="now-cover" id="now-cover" alt="" hidden>
              </div>
              <div class="now-copy" id="now-copy">
                <strong id="now-title"></strong>
                <span id="now-artist"></span>
              </div>
              <span class="spectrum" aria-hidden="true" id="spectrum"></span>
              <div class="seek-position">
                <output id="elapsed" class="visually-hidden" aria-hidden="true">0:00</output>
                <input id="seek" class="range seek-range" type="range" min="0" max="1000" step="1" value="0" aria-label="${t("播放进度", "Playback position")}" disabled>
                <output id="duration" class="visually-hidden" aria-hidden="true">0:00</output>
              </div>
            </div>
            <div class="transport-more" id="transport-more" hidden>
              ${button("more", "transport-more", t("更多曲目操作", "More track actions"), 'aria-expanded="false" aria-controls="transport-menu"')}
              <div class="glass transport-menu" id="transport-menu" role="group" aria-label="${t("曲目操作", "Track actions")}" hidden>
                <button class="transport-menu-item" type="button" data-action="add-inspected" aria-expanded="false" aria-controls="add-menu">
                  ${t("加入歌单", "Add to Playlist")}${icon("chevron")}
                </button>
                <div id="add-menu" class="glass add-menu" hidden></div>
                <button class="transport-menu-item" type="button" data-action="disassemble">
                  ${t("查看档案", "Inspect Cassette")}
                </button>
                <button class="transport-menu-item" id="transport-vault" type="button" data-action="toggle-vault">
                  ${t("存入 Vault", "Save to Vault")}
                </button>
              </div>
            </div>
          </div>
          <div class="transport-end" id="transport-end">
            <div class="output-anchor" id="output-anchor">
              ${button("output", "output-panel", t("播放设备", "Playback Devices"), 'aria-expanded="false" aria-controls="output-panel"')}
              <div id="output-panel" class="glass output-panel" role="group" aria-label="${t("播放设备", "Playback Devices")}" hidden>
                <p class="output-status" id="output-status" role="status" hidden></p>
                <div id="output-devices" class="output-devices"></div>
                <div id="output-volume-widget" class="output-volume-widget" hidden>
                  <span class="volume-range-hit"><input id="output-volume" class="range" type="range" min="0" max="100" step="1" value="100" aria-label="${t("Nasti 音量", "Nasti volume")}"></span>
                </div>
              </div>
            </div>
            <div class="volume-anchor" id="volume-anchor">
              ${button("volume", "volume-panel", t("音量", "Volume"), 'aria-expanded="false" aria-controls="volume-panel"')}
              <div id="volume-panel" class="glass volume-panel" hidden>
                <output id="volume-value" for="volume" class="visually-hidden" aria-hidden="true">100%</output>
                <span class="volume-range-hit"><input id="volume" class="range" type="range" min="0" max="100" step="1" value="100" aria-label="${t("音量", "Volume")}"></span>
              </div>
            </div>
          </div>
        </div>
        <nav class="archive-navigation" id="archive-navigation" aria-label="${t("档案导航", "Archive navigation")}" hidden>
          <button type="button" class="archive-arrow" data-action="previous-playlist" aria-label="${t("上一个歌单", "Previous playlist")}" title="${t("上一个歌单", "Previous playlist")}"><span aria-hidden="true">↑</span></button>
          <div class="archive-ticks">
            <div class="archive-tick-markers" id="archive-tick-markers" aria-hidden="true"></div>
            <input class="archive-scale" id="archive-scale" type="range" min="0" max="0" value="0" aria-label="${t("歌单中的曲目", "Tracks in playlist")}">
          </div>
          <button type="button" class="archive-arrow" data-action="next-playlist" aria-label="${t("下一个歌单", "Next playlist")}" title="${t("下一个歌单", "Next playlist")}"><span aria-hidden="true">↓</span></button>
        </nav>
      </footer>

      <div id="feedback" class="feedback glass" role="status" aria-live="polite" hidden></div>
      <div id="undo-notices" class="undo-notices"></div>
      <div id="loading-status" class="loading-status" role="status">${t("正在打开曲库", "Opening library")}</div>

      <dialog id="panel" class="dialog glass" aria-labelledby="panel-title">
        <header class="panel-header">
          <h2 id="panel-title"></h2>
          ${button("close", "close-panel", t("关闭", "Close"))}
        </header>
        <div class="dialog-content" id="panel-content">
          <section id="search-panel" hidden>
            <div id="search-sources" class="search-filters search-sources" role="group" aria-label="${t("曲库来源", "Library source")}" hidden>
              ${button("library", "search-local", t("本地曲库", "Local library"), 'aria-pressed="true"')}
              ${button("globe", "search-archive", t("Internet Archive 授权曲库", "Licensed Internet Archive catalog"), 'aria-pressed="false"')}
              <span id="search-source-label">${t("本地曲库", "Local library")}</span>
            </div>
            <form id="search-form" class="import-form search-form">
              <label class="search-input">${icon("search")}<input id="search-input" type="search" placeholder="${t("搜索歌名、艺人或歌单", "Search tracks, artists, or playlists")}" autocomplete="off" aria-label="${t("搜索曲库", "Search library")}"></label>
              <button id="search-submit" type="submit" class="icon-button" aria-label="${t("检索在线曲库", "Search online catalog")}" title="${t("检索在线曲库", "Search online catalog")}" hidden>${icon("search")}</button>
            </form>
            <div id="search-filters" class="search-filters" role="group" aria-label="${t("搜索类别", "Search filters")}">
              ${button("radar", "filter-all", t("所有结果", "All results"), 'aria-pressed="true"')}
              ${button("music", "filter-songs", t("歌曲", "Tracks"), 'aria-pressed="false"')}
              ${button("video", "filter-videos", t("音乐影像", "Music videos"), 'aria-pressed="false"')}
              ${button("library", "filter-playlists", t("歌单", "Playlists"), 'aria-pressed="false"')}
            </div>
            <div id="search-results" class="search-results"></div>
          </section>
          <section id="library-panel" hidden>
            <div class="vault-toolbar">
              ${button("back", "vault-drawer", t("返回储藏库抽屉", "Back to Vault"), "hidden")}
              ${button("storage", "vault-capacity", t("展开储藏库容量", "Show Vault storage"))}
            </div>
            <div class="vault-body">
              <div class="vault-meter">
                <p id="vault-stats" role="status"></p>
                <progress id="vault-progress" aria-label="${t("Vault 已使用容量", "Vault storage used")}" max="1" value="0"></progress>
                <p id="vault-remaining"></p>
                <form id="vault-limit-form" class="vault-limit" hidden>
                  <label for="vault-limit">${t("容量上限（MiB）", "Storage limit (MiB)")}</label>
                  <input id="vault-limit" type="number" min="0" max="2048" step="1" required>
                  <button type="submit" class="icon-button" aria-label="${t("保存容量上限", "Save storage limit")}" title="${t("保存容量上限", "Save storage limit")}">${icon("check")}</button>
                </form>
              </div>
              <div id="library-list" class="vault-list"></div>
            </div>
          </section>
          <section id="import-panel" hidden>
            <form id="import-form" class="import-form">
              <label class="search-input"><input id="import-input" autocomplete="off" placeholder="${t("歌单链接或编号", "Playlist link or ID")}" aria-label="${t("授权歌单链接或编号", "Licensed playlist link or ID")}" required></label>
              <button type="submit" class="icon-button" aria-label="${t("查找歌单", "Find playlist")}" title="${t("查找歌单", "Find playlist")}">${icon("search")}</button>
              ${button("music", "youtube-playlists", t("导入或管理 YouTube Music 歌单", "Import or manage YouTube Music playlists"), "data-youtube-entry hidden")}
            </form>
            <p id="import-status" role="status"></p>
            <div id="import-preview" hidden>
              <header>
                <h3>${t("导入预览", "Import preview")}</h3><p id="import-summary"></p>
              </header>
              <div id="import-list"></div>
              <footer>
                ${button("importAll", "import-all", t("导入全部曲目", "Import all tracks"))}
              </footer>
            </div>
          </section>
          <section id="youtube-panel" hidden>
            <p id="youtube-status" role="status" aria-live="polite"></p>
            <button type="button" class="text-button" id="youtube-cancel-connect" hidden>${t("取消登录", "Cancel sign-in")}</button>
            <fieldset id="youtube-controls">
              <div id="youtube-navigation" class="youtube-toolbar">
                ${button("back", "import-playlist", t("返回导入歌单", "Back to playlist import"))}
                <button type="button" class="text-button" data-youtube-action="connect" aria-describedby="youtube-signin-help" hidden>${t("使用 Google 登录", "Sign in with Google")}</button>
                ${youtubeButton("restart", "retry", t("重新读取账号和歌单", "Reload account and playlists"), "hidden")}
                <div id="youtube-account-tools" class="youtube-disclosure" hidden>
                  ${youtubeButton("account", "account-menu", t("Google 账号", "Google account"), 'aria-expanded="false" aria-controls="youtube-account-menu"')}
                  <div id="youtube-account-menu" class="youtube-menu" hidden>
                    <button type="button" data-youtube-action="disconnect">${t("撤销授权", "Revoke access")}</button>
                  </div>
                </div>
              </div>
              <div id="youtube-policy" class="youtube-policy">
                <p id="youtube-policy-summary">${t("Nasti 通过 YouTube API 访问你授权的歌单，并在本机保存你导入的歌单记录。", "Nasti uses YouTube API Services to access the playlists you authorize and saves imported playlist snapshots on this Mac.")}</p>
                <nav aria-label="${t("YouTube Music 隐私与条款", "YouTube Music privacy and terms")}">
                  <a href="https://xiaotwu.github.io/Nasti/privacy.html" data-youtube-policy-page="privacy">${t("隐私政策", "Privacy policy")}</a>
                  <a href="https://xiaotwu.github.io/Nasti/terms.html" data-youtube-policy-page="nastiTerms">${t("Nasti 使用条款", "Nasti terms of use")}</a>
                  <a href="https://www.youtube.com/t/terms" data-youtube-policy-page="youtubeTerms">${t("YouTube 服务条款", "YouTube Terms of Service")}</a>
                </nav>
                <button type="button" class="text-button" data-youtube-action="accept-policy" aria-describedby="youtube-policy-summary">${t("同意隐私政策及条款并继续", "Agree to privacy policy and terms")}</button>
              </div>
              <p id="youtube-signin-help" class="youtube-signin-help" tabindex="-1" hidden></p>
              <form id="youtube-import" class="import-form">
                <label class="search-input">${icon("link")}<input id="youtube-reference" autocomplete="off" placeholder="${t("歌单链接或编号", "Playlist link or ID")}" aria-label="${t("YouTube Music 歌单链接或编号", "YouTube Music playlist link or ID")}" required></label>
                <button type="submit" class="icon-button" aria-label="${t("导入歌单", "Import playlist")}" title="${t("导入歌单", "Import playlist")}">${icon("import")}</button>
              </form>
              <section id="youtube-list-view" aria-label="${t("YouTube Music 歌单", "YouTube Music playlists")}">
                <div class="youtube-toolbar youtube-list-tools" role="group" aria-label="${t("歌单来源", "Playlist source")}">
                  <button type="button" class="text-button" data-youtube-action="imported" aria-pressed="true">${t("已导入", "Imported")}</button>
                  <button type="button" class="text-button" data-youtube-action="mine" aria-pressed="false" hidden>${t("我的歌单", "My playlists")}</button>
                  ${youtubeButton("add", "create", t("新建歌单", "Create playlist"), "hidden")}
                </div>
                <div id="youtube-lists"></div>
              </section>
              <section id="youtube-detail" aria-labelledby="youtube-title" hidden>
                <div class="youtube-playlist-header">
                  ${youtubeButton("back", "back", t("返回歌单列表", "Back to playlists"))}
                  <div class="youtube-heading">
                    <h3 id="youtube-title" tabindex="-1"></h3>
                    <p id="youtube-snapshot"></p>
                  </div>
                  <div class="youtube-toolbar youtube-playlist-tools" role="group" aria-label="${t("歌单操作", "Playlist actions")}">
                    ${youtubeButton("restart", "refresh", t("刷新歌单", "Refresh playlist"))}
                    ${youtubeButton("add", "add-song", t("添加歌曲", "Add track"), 'aria-expanded="false" aria-controls="youtube-add" hidden')}
                    ${youtubeButton("editList", "edit", t("编辑歌单", "Edit playlist"), "hidden")}
                    <div class="youtube-disclosure">
                      ${youtubeButton("more", "playlist-menu", t("更多歌单操作", "More playlist actions"), 'aria-expanded="false" aria-controls="youtube-playlist-menu"')}
                      <div id="youtube-playlist-menu" class="youtube-menu" hidden>
                        <button type="button" data-youtube-action="open">${icon("link")}${t("在 YouTube Music 打开", "Open in YouTube Music")}</button>
                        <button type="button" data-youtube-action="forget" hidden>${icon("remove")}${t("移除本地记录", "Remove local snapshot")}</button>
                        <button type="button" data-youtube-action="delete" hidden>${icon("close")}${t("删除在线歌单", "Delete online playlist")}</button>
                      </div>
                    </div>
                  </div>
                </div>
                <p id="youtube-description"></p>
                <form id="youtube-add" class="import-form" hidden>
                  <label class="search-input">${icon("music")}<input id="youtube-song" autocomplete="off" placeholder="${t("歌曲链接或视频编号", "Track link or video ID")}" aria-label="${t("歌曲链接或视频编号", "Track link or video ID")}" required></label>
                  <button type="submit" class="icon-button" aria-label="${t("确认添加歌曲", "Confirm adding track")}" title="${t("确认添加歌曲", "Confirm adding track")}">${icon("check")}</button>
                </form>
                <ol id="youtube-tracks" aria-label="${t("歌单曲序", "Playlist order")}"></ol>
              </section>
              <form id="youtube-edit" aria-labelledby="youtube-edit-title" hidden>
                <h3 id="youtube-edit-title"></h3>
                <label>${t("名称", "Name")}<input id="youtube-name" autocomplete="off" maxlength="150" required></label>
                <label>${t("简介", "Description")}<textarea id="youtube-note" maxlength="5000" rows="3"></textarea></label>
                <label>${t("可见性", "Visibility")}<select id="youtube-privacy">${youtubePrivacyOptions}</select></label>
                <div class="youtube-toolbar">
                  <button type="button" class="text-button" data-youtube-action="cancel-edit">${t("取消", "Cancel")}</button>
                  <button type="submit" class="text-button">${t("保存", "Save")}</button>
                </div>
              </form>
              <div id="youtube-confirm" aria-labelledby="youtube-confirm-label" role="group" hidden>
                <p id="youtube-confirm-label"></p>
                <div class="youtube-toolbar">
                  <button type="button" class="text-button" data-youtube-action="cancel-confirm">${t("取消", "Cancel")}</button>
                  <button type="button" class="text-button" data-youtube-action="confirm">${t("确认", "Confirm")}</button>
                </div>
              </div>
            </fieldset>
          </section>
        </div>
        <section id="settings-panel" aria-labelledby="panel-title" hidden>
          <nav class="settings-categories" aria-label="${t("设置分类", "Settings categories")}">
            ${settingsSections.map(([key, label]) => `<button type="button" data-action="settings-section" data-section="${key}">${label}</button>`).join("")}
          </nav>
          <div class="settings-scroll" id="settings-scroll" tabindex="0" aria-label="${t("设置项目", "Settings controls")}">
            ${settingsSections.map(([key, label, content]) => `<section id="settings-${key}"><h2>${label}</h2>${content}</section>`).join("")}
          </div>
        </section>
      </dialog>

      <section class="boot" id="boot" aria-label="${t("终端开机动画", "Startup animation")}" hidden>
        <div class="boot-backdrop"></div>
        <div class="boot-composition" aria-hidden="true">
          <div class="boot-aperture">
            <div class="boot-moving">
              <img class="boot-artwork" src="${bootArtwork}" alt="" fetchpriority="high" draggable="false">
            </div>
          </div>
          <svg class="boot-frame" viewBox="0 0 1182 665" fill="none">
            <path class="boot-frame-edge" d="M720 5.5 1047 332.5 720 659.5 393 332.5Z"/>
            <path class="boot-frame-slashes" d="M932 158 990 100M450 547 506 491"/>
          </svg>
          <div class="boot-hand">
            <img class="boot-artwork" src="${bootArtwork}" alt="" draggable="false">
          </div>
          <div class="boot-wordmark">
            <img class="boot-artwork" src="${bootArtwork}" alt="" draggable="false">
          </div>
          <span class="boot-trace"></span>
        </div>
        ${button("skip", "skip-boot", t("跳过开机动画（Esc）", "Skip startup animation (Esc)"))}
      </section>
    </main>`;
}

export function element<T extends HTMLElement = HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error("Missing terminal element " + id);
  return node as T;
}

export function durationLabel(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  return Math.floor(seconds / 60) + ":" + String(seconds % 60).padStart(2, "0");
}

export function safeAssetUrl(path: string | null | undefined): string {
  if (path && (path.startsWith("/authorized-media/") || path.startsWith("/foundry-art/"))) return path;
  return artUrl;
}

export function setText(node: HTMLElement, text: string): void {
  if (node.textContent !== text) node.textContent = text;
}
