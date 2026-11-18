const theme = document.querySelector("[data-theme-toggle]");
const chinese = document.documentElement.lang === "zh-CN";
const word = (en, zh) => chinese ? zh : en;
const glyph = path => `<svg viewBox="0 0 24 24" width="19" height="19" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${path}</svg>`;
const sun = glyph('<circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M2 12h2m16 0h2M4.9 4.9l1.4 1.4m11.4 11.4 1.4 1.4m0-14.2-1.4 1.4M6.3 17.7l-1.4 1.4"/>');
const moon = glyph('<path d="M20.8 14.5A9 9 0 0 1 9.5 3.2 9 9 0 1 0 20.8 14.5Z"/>');
function displayThemeButton() {
  const dark = document.documentElement.dataset.theme === "dark";
  theme.innerHTML = dark ? sun : moon;
  const label = dark ? word("Switch to light theme", "切换到浅色模式") : word("Switch to dark theme", "切换到深色模式");
  theme.setAttribute("aria-label", label);
  theme.title = label;
}
if (theme) {
  theme.hidden = false;
  displayThemeButton();
  theme.addEventListener("click", () => {
    const next = document.documentElement.dataset.theme === "dark" ? "light" : "dark";
    document.documentElement.dataset.theme = next;
    try { localStorage.setItem("nasti-site-theme", next); } catch { /* Keep the current page theme. */ }
    displayThemeButton();
  });
}

const trigger = document.querySelector("[data-search-trigger]");
if (trigger && typeof HTMLDialogElement !== "undefined") {
  const pages = [
    [word("Overview", "首页"), "./", "home nasti music 娜斯提 音乐"],
    [word("Archive and Vault", "档案与离线储藏库"), "./#archive", "playlists offline library internet archive 歌单 本地 下载"],
    [word("Install Nasti", "安装 Nasti"), "./guide.html#install-nasti", "get started mac installation dmg download 下载 苹果"],
    [word("Build your library", "建立曲库"), "./guide.html#build-your-library", "search import tracks 检索 导入 歌曲"],
    ["YouTube Music", "./guide.html#youtube-music", "google authorization playlists 授权 登录"],
    [word("Updates and support", "更新与支持"), "./guide.html#updates-and-support", "github releases download help 发行 帮助"],
    [word("Privacy policy", "隐私政策"), "./privacy.html", "google keychain tokens data retention deletion 数据 保留 删除"],
    [word("Terms of use", "使用条款"), "./terms.html", "license legal gpl service youtube 许可 法律"],
  ];
  const dialog = document.createElement("dialog");
  dialog.className = "search-dialog";
  dialog.setAttribute("aria-labelledby", "site-search-label");
  dialog.innerHTML = `<label id="site-search-label" for="site-search">${word("Search Nasti", "搜索娜斯提")}</label><div class="search-field"><input type="search" id="site-search" autocomplete="off"><form method="dialog"><button aria-label="${word("Close search", "关闭搜索")}">Esc</button></form></div><ul></ul><p role="status"></p>`;
  document.body.append(dialog);
  const input = dialog.querySelector("input");
  const list = dialog.querySelector("ul");
  const status = dialog.querySelector('p[role="status"]');
  function showResults() {
    const words = input.value.trim().toLowerCase().split(/\s+/);
    const matches = pages.filter(([title, , keywords]) => words.every(word => (title + " " + keywords).toLowerCase().includes(word)));
    list.replaceChildren(...matches.map(([title, url]) => {
      const li = document.createElement("li");
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.textContent = title;
      anchor.addEventListener("click", () => dialog.close());
      li.append(anchor);
      return li;
    }));
    status.textContent = matches.length ? "" : word("No matching pages.", "没有匹配的页面。");
  }
  function openSearch() {
    if (dialog.open) return;
    input.value = "";
    showResults();
    dialog.showModal();
    input.focus();
  }
  input.addEventListener("input", showResults);
  input.addEventListener("keydown", event => {
    if (event.key === "ArrowDown") {
      const first = list.querySelector("a");
      if (first) { event.preventDefault(); first.focus(); }
    }
  });
  trigger.addEventListener("click", openSearch);
  trigger.hidden = false;
  document.addEventListener("keydown", event => {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
      event.preventDefault();
      openSearch();
    }
  });
}
