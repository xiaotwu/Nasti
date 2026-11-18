import type {
  YoutubePlaylist, YoutubePlaylistItem, YoutubePlaylistSummary, YoutubePrivacy,
  YoutubeReply, YoutubeRequest, YoutubeStatus,
} from "../../../contracts/ipc-commands";
import { icon, setIcon, type IconName } from "./icons";
import { dateLocale, localizeError, t } from "./i18n";
import { element, setText } from "./layout";
import { empty, trackCountLabel } from "./views";

type ListSource = "imported" | "mine";
type EditMode = "create" | "update";
type Disclosure = "account" | "playlist";
type Announcement = "pending" | "success" | "error";
type DataReply = Exclude<YoutubeReply, { kind: "done" }>;
type ReplyValue<K extends DataReply["kind"]> = Extract<DataReply, { kind: K }>["value"];
type Confirmation = { request: YoutubeRequest; caption: string; trigger: HTMLButtonElement };
const policyKey = "nasti.youtube-policy.2026-11-17";

function hasAcceptedPolicy(): boolean {
  try { return localStorage.getItem(policyKey) === "accepted"; }
  catch { return false; }
}

const privacyLabels: Record<YoutubePrivacy, readonly [string, string]> = {
  private: ["私密", "Private"], public: ["公开", "Public"], unlisted: ["不公开", "Unlisted"],
};
const privacyLabel = (privacy: YoutubePrivacy) => t(...privacyLabels[privacy]);
const trackPosition = (index: number) => (index + 1).toLocaleString(dateLocale(), { useGrouping: false });

export class YoutubePanel {
  private readonly root = element("youtube-panel");
  private readonly controls = element<HTMLFieldSetElement>("youtube-controls");
  private readonly importForm = element<HTMLFormElement>("youtube-import");
  private readonly input = element<HTMLInputElement>("youtube-reference");
  private readonly connect = this.control("connect");
  private readonly acceptPolicy = this.control("accept-policy");
  private readonly cancelConnect = element<HTMLButtonElement>("youtube-cancel-connect");
  private readonly retry = this.control("retry");
  private account: YoutubeStatus | null = null;
  private source: ListSource = "imported";
  private playlists: YoutubePlaylistSummary[] = [];
  private selected: YoutubePlaylist | null = null;
  private editing: EditMode | null = null;
  private confirmation: Confirmation | null = null;
  private menu: Disclosure | null = null;
  private adding = false;
  private nextFocus: HTMLElement | null = null;
  private connecting = false;
  private policyAccepted = hasAcceptedPolicy();

  constructor(
    private readonly invoke: (request: YoutubeRequest) => Promise<YoutubeReply>,
    signal: AbortSignal,
  ) {
    this.root.addEventListener("click", event => {
      if (!(event.target instanceof Element)) return;
      const link = event.target.closest<HTMLAnchorElement>("a[data-youtube-policy-page]");
      if (link) {
        event.preventDefault();
        const page = link.dataset.youtubePolicyPage;
        if (page === "privacy" || page === "nastiTerms" || page === "youtubeTerms") {
          void this.complete({ action: "openPolicy", page }).catch(error => {
            this.announce(localizeError(error, "无法打开文档", "Could not open the document"), "error");
          });
        }
        return;
      }
      const button = event.target.closest<HTMLButtonElement>("button[data-youtube-action]");
      if (button && !button.disabled && !this.controls.disabled) this.handle(button);
      else if (!event.target.closest(".youtube-disclosure")) this.setMenu(null);
    }, { signal });
    this.cancelConnect.addEventListener("click", () => this.cancelConnection(), { signal });
    signal.addEventListener("abort", () => this.cancelConnection(), { once: true });
    this.importForm.addEventListener("submit", event => {
      event.preventDefault();
      void this.import(this.input.value);
    }, { signal });
    element("youtube-add").addEventListener("submit", event => {
      event.preventDefault();
      const input = element<HTMLInputElement>("youtube-song");
      if (this.selected && input.value.trim()) {
        void this.changePlaylist(
          { action: "add", playlistId: this.selected.id, reference: input.value.trim() },
          t("已添加歌曲", "Track added"),
        ).then(changed => {
          if (changed) {
            input.value = "";
            this.focus(input);
          }
        });
      }
    }, { signal });
    element("youtube-edit").addEventListener("submit", event => {
      event.preventDefault();
      void this.saveEdit();
    }, { signal });
  }

  async show(reference?: string): Promise<void> {
    this.setMenu(null);
    if (reference) {
      this.input.value = reference;
      this.selected = null;
      this.editing = null;
      this.confirmation = null;
    }
    if (this.controls.disabled) return;
    this.renderView();
    if (!this.policyAccepted) {
      this.focus(this.acceptPolicy);
      return;
    }
    await this.run(async () => {
      const status = await this.request({ action: "status" }, "status");
      if (this.account?.connected && !status.connected) {
        this.selected = null;
        this.editing = this.confirmation = null;
        this.source = "imported";
        this.playlists = [];
        this.renderList();
        this.renderView();
      }
      this.account = status;
      this.retry.hidden = true;
      await this.loadList(this.account.connected ? this.source : "imported");
    }, t("正在读取歌单", "Loading playlists…")).then(loaded => {
      this.retry.hidden = loaded;
      this.renderAccount();
      if (this.selected) this.focus(element("youtube-title"));
      else if (!this.connect.hidden && this.connect.disabled) this.focus(element("youtube-signin-help"));
      else this.focus(this.connect.hidden ? this.input : this.connect);
    });
  }

  dismiss(): boolean {
    if (this.connecting) {
      this.cancelConnection();
      return true;
    }
    if (this.confirmation) {
      const { trigger } = this.confirmation;
      this.confirmation = null;
      this.renderView();
      this.focus(trigger);
      return true;
    }
    if (this.editing) {
      const action = this.editing === "create" ? "create" : "edit";
      this.editing = null;
      this.renderView();
      this.focus(this.control(action));
      return true;
    }
    if (this.menu) {
      const trigger = this.control(this.menu + "-menu");
      this.setMenu(null);
      this.focus(trigger);
      return true;
    }
    if (this.adding) {
      this.adding = false;
      this.renderView();
      this.focus(this.control("add-song"));
      return true;
    }
    return false;
  }

  private setMenu(menu: Disclosure | null): void {
    if (menu === this.menu) return;
    this.menu = menu;
    for (const name of ["account", "playlist"] as const) {
      const open = name === menu;
      this.control(name + "-menu").setAttribute("aria-expanded", String(open));
      element("youtube-" + name + "-menu").hidden = !open;
    }
  }

  private control(action: string): HTMLButtonElement {
    return this.root.querySelector<HTMLButtonElement>(`[data-youtube-action="${action}"]`)!;
  }

  private async request<K extends DataReply["kind"]>(
    request: YoutubeRequest, kind: K,
  ): Promise<ReplyValue<K>> {
    const reply = await this.invoke(request);
    if (reply.kind !== kind || !("value" in reply)) throw new Error(t("YouTube Music 返回了意外的结果", "YouTube Music returned an unexpected response"));
    return reply.value as ReplyValue<K>;
  }

  private async complete(request: YoutubeRequest): Promise<void> {
    if ((await this.invoke(request)).kind !== "done") throw new Error(t("YouTube Music 返回了意外的结果", "YouTube Music returned an unexpected response"));
  }

  private async run(
    work: () => Promise<void>, pending: string, success = "",
    failure: readonly [string, string] = ["YouTube Music 操作失败", "The YouTube Music operation failed"],
  ): Promise<boolean> {
    if (this.controls.disabled) return false;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    this.controls.disabled = true;
    this.controls.setAttribute("aria-busy", "true");
    this.announce(pending, "pending");
    try {
      await work();
      this.announce(success, "success");
      return true;
    } catch (error) {
      this.announce(localizeError(error, ...failure), "error");
      return false;
    } finally {
      this.controls.disabled = false;
      this.controls.removeAttribute("aria-busy");
      this.focus(this.nextFocus ?? previous);
      this.nextFocus = null;
    }
  }

  private focus(target: HTMLElement | null): void {
    if (target?.isConnected && !target.closest("[hidden]") && this.root.closest("dialog")?.open) {
      target.focus({ preventScroll: true });
    }
  }

  private announce(message: string, state: Announcement): void {
    const status = element("youtube-status");
    status.dataset.state = state;
    setText(status, message);
  }

  private handle(button: HTMLButtonElement): void {
    const action = button.dataset.youtubeAction;
    if (action === "account-menu" || action === "playlist-menu") {
      const menu = action === "account-menu" ? "account" : "playlist";
      this.setMenu(this.menu === menu ? null : menu);
      if (this.menu) this.focus(element("youtube-" + menu + "-menu").querySelector<HTMLButtonElement>("button:not([hidden])"));
      return;
    }
    const trigger = this.menu && button.closest(".youtube-menu") ? this.control(this.menu + "-menu") : button;
    this.setMenu(null);
    switch (action) {
      case "accept-policy":
        this.policyAccepted = true;
        try { localStorage.setItem(policyKey, "accepted"); } catch { /* Keep consent for this window. */ }
        void this.show();
        return;
      case "retry": void this.show(); return;
      case "connect": void this.connectAccount(); return;
      case "disconnect": this.askConfirmation(
        { action }, t("撤销授权并移除这个账号的本地歌单记录？", "Revoke access and remove this account’s local playlist records?"), trigger,
      ); return;
      case "imported": case "mine":
        if (action === "imported" || this.account?.connected) void this.run(() => this.loadList(action), t("正在读取歌单", "Loading playlists…"));
        return;
      case "create": this.edit("create"); return;
      case "edit": this.edit("update"); return;
      case "cancel-edit": case "cancel-confirm": this.dismiss(); return;
      case "confirm": void this.confirm(); return;
      case "back":
        this.selected = null;
        this.renderView();
        this.focus(this.control(this.source));
        return;
      case "load":
        if (button.dataset.playlistId) void this.loadPlaylist(button.dataset.playlistId);
        return;
    }
    const playlist = this.selected;
    if (!playlist) return;
    const playlistId = playlist.id;
    const entryId = button.dataset.entryId;
    switch (action) {
      case "refresh": void this.import(playlistId); return;
      case "add-song":
        this.adding = !this.adding;
        this.renderView();
        if (this.adding) this.focus(element("youtube-song"));
        return;
      case "open":
        this.focus(trigger);
        void this.run(() => this.complete({ action, playlistId, videoId: button.dataset.videoId }), t("正在打开 YouTube Music", "Opening YouTube Music…"));
        return;
      case "forget":
        this.askConfirmation({ action, playlistId }, t(`移除「${playlist.title}」在 Nasti 保存的歌单记录？`, `Remove the local Nasti copy of “${playlist.title}”?`), trigger);
        return;
      case "delete":
        this.askConfirmation({ action, playlistId }, t(`删除「${playlist.title}」在线歌单？`, `Delete “${playlist.title}” from YouTube Music?`), trigger);
        return;
      case "remove": {
        const track = playlist.items.find(item => item.entryId === entryId);
        if (track) this.askConfirmation(
          { action, playlistId, entryId: track.entryId },
          t(`从在线歌单移除第 ${trackPosition(track.position)} 首「${track.title}」？`, `Remove track ${trackPosition(track.position)}, “${track.title}”, from the online playlist?`), button,
        );
        return;
      }
      case "move":
        if (entryId && button.dataset.position !== undefined) {
          void this.changePlaylist({ action, playlistId, entryId, position: Number(button.dataset.position) }, t("已更新曲序", "Track order updated"), button);
        }
    }
  }

  cancelConnection(): void {
    if (!this.connecting || this.cancelConnect.disabled) return;
    this.cancelConnect.disabled = true;
    void this.complete({ action: "cancelConnect" }).catch(error => {
      this.cancelConnect.disabled = false;
      this.announce(localizeError(error, "取消登录失败，请重试", "Could not cancel sign-in. Try again."), "error");
    });
  }

  private async connectAccount(): Promise<void> {
    if (this.controls.disabled || !this.account?.canConnect) return;
    this.connecting = true;
    this.cancelConnect.disabled = false;
    this.cancelConnect.hidden = false;
    try {
      const pending = this.run(async () => {
        this.account = await this.request({ action: "connect" }, "status");
        this.renderAccount();
        if (!this.account.connected) throw new Error(t("尚未连接 YouTube，请重试", "Could not connect to YouTube. Try again."));
        const pendingImport = Boolean(this.input.value.trim());
        await this.loadList(pendingImport ? "imported" : "mine");
        this.nextFocus = this.selected ? element("youtube-title") : pendingImport ? this.input : this.control("mine");
      }, t("请在系统浏览器完成 Google 授权，完成后返回 Nasti。", "Authorize with Google in your system browser, then return to Nasti."), t("已连接 YouTube", "Connected to YouTube"));
      this.focus(this.cancelConnect);
      await pending;
    } finally {
      const restoreFocus = document.activeElement === this.cancelConnect;
      this.connecting = false;
      this.cancelConnect.hidden = true;
      if (restoreFocus) this.focus(this.connect);
    }
  }

  private async loadList(source: ListSource): Promise<void> {
    this.playlists = await this.request({ action: source }, "lists");
    this.source = source;
    this.renderList();
    this.renderView();
  }

  private async loadPlaylist(playlistId: string): Promise<void> {
    await this.run(async () => {
      this.showPlaylist(await this.request({ action: "load", playlistId }, "playlist"));
      this.nextFocus = element("youtube-title");
    }, t("正在读取歌单", "Loading playlist…"));
  }

  private async import(reference: string): Promise<void> {
    if (!reference.trim() || !this.account?.connected) return;
    await this.run(async () => {
      const playlist = await this.request({ action: "import", reference: reference.trim() }, "playlist");
      this.replaceSummary(playlist);
      this.showPlaylist(playlist);
      this.nextFocus = element("youtube-title");
    }, t("正在同步歌单", "Syncing playlist…"), t("已保存到 Nasti", "Saved to Nasti"));
  }

  private async changePlaylist(request: YoutubeRequest, caption: string, move?: HTMLButtonElement): Promise<boolean> {
    return this.run(async () => {
      const playlist = await this.request(request, "playlist");
      this.replaceSummary(playlist);
      this.showPlaylist(playlist);
      if (move?.dataset.entryId) {
        const moved = this.findTrackButton(move.dataset.entryId, move.dataset.direction);
        this.nextFocus = moved ?? element("youtube-title");
      }
    }, t("正在更新 YouTube 歌单", "Updating YouTube playlist…"), caption);
  }

  private findTrackButton(entryId: string, direction?: string): HTMLButtonElement | undefined {
    const buttons = element("youtube-tracks").querySelectorAll<HTMLButtonElement>("[data-youtube-action=move]");
    return [...buttons].find(button => button.dataset.entryId === entryId && button.dataset.direction === direction && !button.disabled)
      ?? [...buttons].find(button => button.dataset.entryId === entryId && !button.disabled);
  }

  private edit(mode: EditMode): void {
    const current = mode === "update" ? this.selected : null;
    this.editing = mode;
    setText(element("youtube-edit-title"), mode === "update" ? t("编辑在线歌单", "Edit playlist") : t("新建在线歌单", "New playlist"));
    const name = element<HTMLInputElement>("youtube-name");
    name.value = current?.title ?? "";
    element<HTMLTextAreaElement>("youtube-note").value = current?.description ?? "";
    element<HTMLSelectElement>("youtube-privacy").value = current?.privacy ?? "private";
    this.renderView();
    this.focus(name);
  }

  private async saveEdit(): Promise<void> {
    const title = element<HTMLInputElement>("youtube-name").value.trim();
    if (!title) {
      this.announce(t("请输入歌单名称", "Enter a playlist name"), "error");
      return;
    }
    const values = {
      title,
      description: element<HTMLTextAreaElement>("youtube-note").value,
      privacy: element<HTMLSelectElement>("youtube-privacy").value as YoutubePrivacy,
    };
    const request: YoutubeRequest = this.editing === "create"
      ? { action: "create", ...values }
      : { action: "update", playlistId: this.selected?.id ?? "", ...values };
    await this.run(async () => {
      const playlist = await this.request(request, "playlist");
      this.editing = null;
      this.replaceSummary(playlist);
      this.showPlaylist(playlist);
      this.nextFocus = element("youtube-title");
    }, t("正在保存在线歌单", "Saving playlist…"), t("已保存在线歌单", "Playlist saved"));
  }

  private askConfirmation(request: YoutubeRequest, caption: string, trigger: HTMLButtonElement): void {
    this.confirmation = { request, caption, trigger };
    this.renderView();
    this.focus(this.control("cancel-confirm"));
  }

  private async confirm(): Promise<void> {
    const pending = this.confirmation;
    if (!pending) return;
    const request = pending.request;
    const finished = await this.run(async () => {
      switch (request.action) {
        case "disconnect":
          this.account = await this.request(request, "status");
          this.selected = null;
          this.playlists = [];
          this.source = "imported";
          this.renderAccount();
          break;
        case "remove":
          this.showPlaylist(await this.request(request, "playlist"));
          break;
        case "delete": case "forget":
          await this.complete(request);
          this.playlists = this.playlists.filter(playlist => playlist.id !== request.playlistId
            || request.action === "forget" && this.source === "mine");
          this.selected = null;
          break;
      }
      this.confirmation = null;
      this.renderList();
      this.renderView();
      this.nextFocus = this.selected ? element("youtube-title") : this.control(this.source);
    }, t("正在提交操作", "Applying changes…"), t("操作已完成", "Changes applied"));
    if (finished && request.action === "disconnect") {
      await this.run(() => this.loadList("imported"), t("正在读取歌单", "Loading playlists…"), t("已撤销授权", "Access revoked"));
    }
  }

  private showPlaylist(playlist: YoutubePlaylist): void {
    if (this.selected?.id !== playlist.id) {
      this.adding = false;
      element<HTMLInputElement>("youtube-song").value = "";
    }
    this.selected = playlist;
    const title = element("youtube-title");
    title.title = playlist.title;
    setText(title, playlist.title);
    setText(element("youtube-description"), playlist.description);
    const snapshot = element("youtube-snapshot");
    if (playlist.importedAt == null) snapshot.title = "";
    else {
      const updated = new Intl.DateTimeFormat(dateLocale(), { dateStyle: "medium", timeStyle: "short" }).format(playlist.importedAt);
      snapshot.title = t(`更新于 ${updated}`, `Updated ${updated}`);
    }
    setText(snapshot, `${privacyLabel(playlist.privacy)} · ${trackCountLabel(playlist.items.length)}`);
    const refresh = this.control("refresh");
    const saved = playlist.importedAt != null;
    setIcon(refresh, saved ? "restart" : "import", saved ? t("刷新歌单", "Refresh playlist") : t("导入歌单", "Import playlist"));
    this.control("refresh").disabled = !this.account?.connected;
    this.control("forget").hidden = playlist.importedAt == null;
    const editable = playlist.canEdit && Boolean(this.account?.connected);
    this.control("edit").hidden = this.control("delete").hidden = !editable;
    this.control("add-song").hidden = !editable;
    const tracks = element("youtube-tracks");
    tracks.replaceChildren(...playlist.items.map(item => this.makeTrack(item, playlist.items.length, editable)));
    if (!playlist.items.length) {
      const placeholder = document.createElement("li");
      placeholder.append(empty(t("歌单暂无歌曲", "This playlist has no tracks")));
      tracks.append(placeholder);
    }
    this.renderView();
  }

  private makeTrack(item: YoutubePlaylistItem, count: number, editable: boolean): HTMLLIElement {
    const row = document.createElement("li");
    row.className = "youtube-track";
    const index = document.createElement("small");
    const positionLabel = trackPosition(item.position);
    index.textContent = positionLabel;
    const tools = document.createElement("div");
    tools.className = "youtube-track-tools";
    const play = this.iconButton("link", "open", t(`在 YouTube Music 播放 ${item.title}`, `Play ${item.title} on YouTube Music`));
    play.dataset.videoId = item.videoId;
    play.disabled = !item.videoId;
    tools.append(play);
    if (editable) {
      for (const [direction, offset, zh, en] of [["up", -1, "上移", "up"], ["down", 1, "下移", "down"]] as const) {
        const position = item.position + offset;
        const move = this.iconButton(direction, "move", t(`${zh}第 ${positionLabel} 首 ${item.title}`, `Move track ${positionLabel}, ${item.title}, ${en}`));
        move.dataset.entryId = item.entryId;
        move.dataset.direction = direction;
        move.dataset.position = String(position);
        move.disabled = !item.videoId || position < 0 || position >= count;
        tools.append(move);
      }
      const remove = this.iconButton("remove", "remove", t(`移除第 ${positionLabel} 首 ${item.title}`, `Remove track ${positionLabel}, ${item.title}`));
      remove.dataset.entryId = item.entryId;
      tools.append(remove);
    }
    row.append(index, this.copy(item.title, item.artist), tools);
    return row;
  }

  private iconButton(glyph: IconName, action: string, label: string): HTMLButtonElement {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "icon-button";
    button.dataset.youtubeAction = action;
    button.title = label;
    button.setAttribute("aria-label", label);
    button.innerHTML = icon(glyph);
    return button;
  }

  private copy(title: string, secondary: string): HTMLElement {
    const copy = document.createElement("span");
    copy.className = "track-copy";
    const name = document.createElement("strong");
    name.textContent = name.title = title;
    const byline = document.createElement("small");
    byline.textContent = byline.title = secondary;
    copy.append(name, byline);
    return copy;
  }

  private replaceSummary(playlist: YoutubePlaylist): void {
    const eligible = this.source === "mine" ? playlist.canEdit : playlist.importedAt != null;
    if (eligible) {
      const index = this.playlists.findIndex(item => item.id === playlist.id);
      if (index < 0) this.playlists.unshift(playlist);
      else this.playlists[index] = playlist;
    }
    this.renderList();
  }

  private renderList(): void {
    for (const source of ["imported", "mine"] as const) {
      this.control(source).setAttribute("aria-pressed", String(this.source === source));
    }
    const list = element("youtube-lists");
    list.replaceChildren(...this.playlists.map(playlist => {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "youtube-list-row";
      button.dataset.youtubeAction = "load";
      button.dataset.playlistId = playlist.id;
      button.append(this.copy(playlist.title, privacyLabel(playlist.privacy)));
      return button;
    }));
    if (!this.playlists.length) list.append(empty(this.source === "mine"
      ? t("账号暂无可管理歌单", "This account has no playlists to manage")
      : t("暂无已导入歌单", "No playlists imported")));
  }

  private renderAccount(): void {
    const status = this.account;
    this.renderSignin();
    this.connect.hidden = !status || status.connected;
    this.connect.disabled = !status?.canConnect;
    element("youtube-account-tools").hidden = !status?.connected;
    this.control("mine").hidden = this.control("create").hidden = !status?.connected;
    this.importForm.querySelector("button")!.disabled = !status?.connected;
    if (this.selected) this.showPlaylist(this.selected);
  }

  private renderSignin(): void {
    const status = this.account;
    const help = element("youtube-signin-help");
    help.hidden = !status || status.connected || Boolean(this.selected || this.confirmation || this.editing);
    setText(help, status?.canConnect
      ? t("将在系统浏览器中授权，完成后返回 Nasti。", "Your system browser will open for authorization. Return to Nasti when finished.")
      : t("此版本尚未启用 Google 登录。", "Google sign-in is not available in this build."));
  }

  private renderView(): void {
    const overlay = Boolean(this.confirmation || this.editing);
    element("youtube-navigation").hidden = Boolean(this.selected) || overlay;
    const policy = element("youtube-policy");
    policy.dataset.accepted = String(this.policyAccepted);
    this.acceptPolicy.hidden = this.policyAccepted;
    this.renderSignin();
    const add = this.control("add-song");
    const adding = this.adding && !add.hidden && Boolean(this.selected) && !overlay;
    element("youtube-add").hidden = !adding;
    add.setAttribute("aria-expanded", String(adding));
    this.importForm.hidden = !this.policyAccepted || Boolean(this.selected) || overlay;
    element("youtube-list-view").hidden = !this.policyAccepted || Boolean(this.selected) || overlay;
    element("youtube-detail").hidden = !this.selected || overlay;
    element("youtube-edit").hidden = !this.editing || Boolean(this.confirmation);
    element("youtube-confirm").hidden = !this.confirmation;
    setText(element("youtube-confirm-label"), this.confirmation?.caption ?? "");
  }
}
