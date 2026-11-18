import type {
  PlaylistColumn, YoutubePlaylist, YoutubePrivacy, YoutubeReply, YoutubeRequest, YoutubeStatus,
} from "../../../contracts/ipc-commands";
import { localizeError, t } from "./i18n";
import { element, setText } from "./layout";
import { actionButton, empty, trackCountLabel } from "./views";
import type { IconName } from "./icons";

type ReplyValues = { [Reply in Exclude<YoutubeReply, { kind: "done" }> as Reply["kind"]]: Reply["value"] };

export type ToastLevel = "info" | "success" | "warning" | "error";

export function playlistReference(text: string): string | null {
  const value = text.trim();
  if (/^VLPL[\w-]{8,125}$/.test(value)) return value.slice(2);
  if (/^(?:PL|OLAK5uy_|UU|LL|RD|FL|WL)[\w-]{8,127}$/.test(value)) return value;
  try {
    const url = new URL(/^[\w.]+\.com\//i.test(value) ? `https://${value}` : value);
    if (!/^https?:$/.test(url.protocol) || url.username || url.password || url.port
      || !["youtube.com", "www.youtube.com", "music.youtube.com", "m.youtube.com", "youtu.be"].includes(url.hostname)) return null;
    const browse = url.hostname === "music.youtube.com" && /^\/browse\/VL([\w-]{2,128})$/.exec(url.pathname);
    if (browse) return browse[1];
    const ids = url.searchParams.getAll("list");
    return ids.length === 1 && /^[\w-]{2,128}$/.test(ids[0]) ? ids[0] : null;
  } catch {
    return null;
  }
}

export class YoutubePanel {
  private account: YoutubeStatus | null = null;
  private column: PlaylistColumn | null = null;
  private editing: "create" | "update" | null = null;
  private pending: YoutubeRequest | null = null;
  private busy = false;
  private touchedAccount = false;
  private loggedInSync = false;
  private readonly login = element<HTMLDialogElement>("login-card");
  private readonly avatar = element<HTMLButtonElement>("user-avatar-pill");
  private readonly popover = element("account-popover");

  constructor(
    private readonly request: ((request: YoutubeRequest) => Promise<YoutubeReply>) | undefined,
    private readonly changed: (playlistId?: string) => Promise<void>,
    private readonly notify: (text: string, level: ToastLevel, openLogin?: boolean) => void,
    private readonly signal: AbortSignal,
  ) {
    element("terminal").addEventListener("click", event => {
      if (!(event.target instanceof Element)) return;
      const button = event.target.closest<HTMLButtonElement>("button[data-youtube-action]");
      if (button) {
        void this.execute(() => this.click(button));
      } else if (!this.popover.contains(event.target)) {
        this.setPopover(false);
      }
    }, { signal });
    element("youtube-client-file").addEventListener("change", event => {
      const input = event.target as HTMLInputElement;
      const file = input.files?.[0];
      input.value = "";
      if (!file || this.busy) return;
      void this.perform(async () => {
        if (file.size > 65_536) throw new Error(t("OAuth 客户端文件过大", "OAuth client file is too large"));
        this.account = await this.send({ action: "configure", json: await file.text() }, "status");
        this.renderAccount();
        this.notify(t("已导入 OAuth 客户端", "OAuth client imported"), "success");
      });
    }, { signal });
    element("youtube-edit").addEventListener("submit", event => {
      event.preventDefault();
      const playlist = this.column;
      const values = {
        title: element<HTMLInputElement>("youtube-name").value.trim(),
        description: element<HTMLTextAreaElement>("youtube-note").value,
        privacy: element<HTMLSelectElement>("youtube-privacy").value as YoutubePrivacy,
      };
      if (this.editing === "create") void this.mutate({ action: "create", ...values });
      else if (this.editing === "update" && playlist) void this.mutate({ action: "update", playlistId: playlist.playlistId, ...values });
    }, { signal });
    element("youtube-add").addEventListener("submit", event => {
      event.preventDefault();
      const input = element<HTMLInputElement>("youtube-song");
      if (this.column && input.value.trim()) {
        void this.mutate({ action: "add", playlistId: this.column.playlistId, reference: input.value.trim() });
      }
    }, { signal });
  }

  async start(bootComplete: Promise<void>, automatically = true): Promise<void> {
    const status = this.request ? this.send({ action: "status" }, "status") : Promise.resolve({ canConnect: false, connected: false });
    await this.execute(async () => {
      const account = await status;
      if (this.signal.aborted || this.touchedAccount) return;
      this.account = account;
      this.renderAccount();
      if (account.connected && automatically) {
        // Render the persisted columns immediately while the service refreshes the account.
        await this.changed();
        void this.syncOwned();
      }
      await bootComplete;
      if (!this.signal.aborted && automatically && !this.touchedAccount && !this.account?.connected) this.openLogin();
    });
  }

  get connected(): boolean { return Boolean(this.account?.connected); }

  requireAccount(): boolean {
    if (this.connected) return true;
    this.notify(t("请先登录 Google 账号以导入或管理歌单", "Sign in with Google to import or manage playlists"), "warning", true);
    return false;
  }

  openLogin(): void {
    this.touchedAccount = true;
    this.setPopover(false);
    if (!this.login.open) {
      this.login.show();
      this.login.querySelector<HTMLElement>("button:not(:disabled)")?.focus();
    }
  }

  dismiss(): boolean {
    if (this.login.open) { this.login.close(); this.avatar.focus(); return true; }
    if (!this.popover.hidden) { this.setPopover(false); this.avatar.focus(); return true; }
    if (!element("radar").hidden && (this.editing || this.pending || !element("youtube-add").hidden)) {
      this.closeForms();
      return true;
    }
    return false;
  }

  async preview(id: string, area: HTMLElement, current: () => boolean): Promise<void> {
    if (!current()) return;
    if (!this.requireAccount()) {
      area.replaceChildren();
      return;
    }
    area.replaceChildren(empty(t("正在读取歌单", "Loading playlist…")));
    const playlist = await this.send({ action: "load", playlistId: id }, "playlist");
    if (!current()) return;
    const heading = document.createElement("h3");
    heading.textContent = playlist.title;
    const summary = document.createElement("p");
    summary.textContent = [trackCountLabel(playlist.items.length), playlist.description].filter(Boolean).join(" · ");
    const list = document.createElement("ol");
    list.className = "playlist-preview";
    for (const track of playlist.items) {
      const entry = document.createElement("li");
      entry.textContent = [track.title, track.artist].filter(Boolean).join(" · ");
      list.append(entry);
    }
    const save = this.control("import", "subscribe", playlist.canEdit ? t("同步到曲库", "Sync to library") : t("订阅歌单", "Subscribe to playlist"));
    save.dataset.playlistId = id;
    save.disabled = !playlist.canEdit && playlist.privacy !== "public";
    area.replaceChildren(heading, summary, save, list);
  }

  decorate(column: PlaylistColumn | null, rows: HTMLElement[]): void {
    if (this.column?.playlistId !== column?.playlistId) this.closeForms();
    this.column = column;
    const actions = element("overview-actions");
    actions.replaceChildren();
    setText(element("overview-description"), column?.subtitle ?? "");
    if (!column) return;
    actions.append(this.control("restart", "refresh", t("刷新歌单", "Refresh playlist")));
    if (!column.isEditable) {
      actions.append(this.control("remove", "forget", t("取消订阅", "Unsubscribe")));
      return;
    }
    actions.append(
      this.control("add", "add-song", t("添加歌曲", "Add track")),
      this.control("editList", "edit", t("编辑歌单", "Edit playlist")),
      this.control("remove", "delete", t("删除在线歌单", "Delete playlist on YouTube")),
    );
    const positions = column.tracks.map((track, index) => (track.trackNumber ?? index + 1) - 1);
    rows.forEach((row, index) => {
      const tools = document.createElement("div");
      tools.className = "youtube-track-tools";
      for (const [glyph, offset, label] of [["up", -1, t("上移", "Move up")], ["down", 1, t("下移", "Move down")]] as const) {
        const move = this.control(glyph, "move", label);
        const target = positions[index + offset];
        move.disabled = target === undefined || target !== positions[index] + offset;
        move.dataset.position = String(target);
        tools.append(move);
      }
      tools.append(this.control("remove", "remove", t("从歌单移除", "Remove from playlist")));
      row.append(tools);
    });
  }

  private async click(button: HTMLButtonElement): Promise<void> {
    const action = button.dataset.youtubeAction;
    if (action === "account") {
      this.touchedAccount = true;
      if (this.request && !this.connected) {
        this.account = await this.send({ action: "status" }, "status");
        this.renderAccount();
        if (this.connected && !this.loggedInSync) void this.syncOwned();
      }
      if (this.connected) this.setPopover(this.popover.hidden);
      else this.openLogin();
      return;
    }
    if (action === "privacy" || action === "terms") {
      await this.request?.({ action: "openPolicy", page: action === "privacy" ? "privacy" : "nastiTerms" });
      return;
    }
    if (action === "close-login") { this.login.close(); this.avatar.focus(); return; }
    if (action === "cancel-edit" || action === "cancel-confirm") { this.closeForms(); return; }
    if (this.busy) return;
    if (action === "connect") {
      await this.perform(async () => {
        this.notify(t("请在系统浏览器完成 Google 授权", "Complete Google authorization in your browser"), "info");
        this.account = await this.send({ action: "connect" }, "status");
        this.renderAccount();
        this.login.close();
        this.avatar.focus();
        this.notify(t("已连接 Google 账号", "Google account connected"), "success");
      });
      if (this.connected) await this.syncOwned();
      return;
    }
    if (action === "confirm") {
      if (this.pending) await this.mutate(this.pending);
      return;
    }
    if (action !== "forget" && !this.requireAccount()) return;
    if (action === "sync") return this.syncOwned();
    if (action === "subscribe" && button.dataset.playlistId) {
      await this.mutate({ action: "import", reference: button.dataset.playlistId });
      if (button.isConnected) button.disabled = this.lastMutationSucceeded;
      return;
    }
    if (action === "disconnect") {
      if (!window.confirm(t("断开 Google 账号并撤销授权？", "Disconnect your Google account and revoke authorization?"))) return;
      return this.mutate({ action: "disconnect" });
    }
    if (action === "create") { this.openEdit(null); return; }
    const column = this.column;
    if (!column) return;
    const playlistId = column.playlistId;
    if (action === "edit") {
      const playlist = await this.send({ action: "cached", playlistId }, "playlist");
      if (this.column?.playlistId === playlistId) this.openEdit(playlist);
    } else if (action === "add-song") {
      const add = element("youtube-add");
      const open = add.hidden;
      this.closeForms();
      add.hidden = !open;
      if (open) element("youtube-song").focus();
    } else if (action === "refresh") {
      await this.mutate({ action: "load", playlistId });
    } else if (action === "forget" || action === "delete") {
      this.confirm({ action, playlistId }, action === "forget"
        ? t(`取消订阅「${column.title}」？`, `Unsubscribe from “${column.title}”?`)
        : t(`从 YouTube 删除「${column.title}」？`, `Delete “${column.title}” from YouTube?`));
    } else {
      const entryId = button.closest<HTMLElement>("[data-entry-id]")?.dataset.entryId;
      if (!entryId) return;
      if (action === "remove") this.confirm({ action, playlistId, entryId }, t("从在线歌单移除这首曲目？", "Remove this track from the online playlist?"));
      if (action === "move") await this.mutate({ action, playlistId, entryId, position: Number(button.dataset.position) });
    }
  }

  private async syncOwned(): Promise<void> {
    if (this.busy) return;
    this.loggedInSync = true;
    await this.perform(async () => {
      try {
        await this.send({ action: "sync" }, "lists");
      } finally {
        await this.changed();
      }
      this.notify(t("YouTube 歌单已同步", "YouTube playlists synced"), "success");
    });
  }

  private lastMutationSucceeded = false;

  private async mutate(request: YoutubeRequest): Promise<void> {
    this.lastMutationSucceeded = false;
    if (request.action !== "forget" && !this.requireAccount()) return;
    await this.perform(async () => {
      if (!this.request) throw new Error("YouTube IPC unavailable");
      let reply: YoutubeReply;
      try {
        reply = await this.request(request);
      } catch (error) {
        if (request.action === "disconnect") {
          this.account = await this.send({ action: "status" }, "status");
          this.renderAccount();
          if (!this.connected) await this.changed();
        }
        throw error;
      }
      if (this.signal.aborted) return;
      if (reply.kind === "status") {
        this.account = reply.value;
        this.loggedInSync = false;
        this.renderAccount();
        this.setPopover(false);
      }
      this.closeForms();
      await this.changed(reply.kind === "playlist" ? reply.value.id : undefined);
      this.lastMutationSucceeded = true;
      this.notify(t("操作已完成", "Changes applied"), "success");
    });
  }

  private confirm(request: YoutubeRequest, label: string): void {
    this.closeForms();
    this.pending = request;
    setText(element("youtube-confirm-label"), label);
    element("youtube-confirm").hidden = false;
    element("youtube-confirm").querySelector("button")?.focus();
  }

  private openEdit(playlist: YoutubePlaylist | null): void {
    this.closeForms();
    this.editing = playlist ? "update" : "create";
    setText(element("youtube-edit-title"), playlist ? t("编辑在线歌单", "Edit playlist") : t("新建 YouTube 歌单", "Create YouTube playlist"));
    const name = element<HTMLInputElement>("youtube-name");
    name.value = playlist?.title ?? "";
    element<HTMLTextAreaElement>("youtube-note").value = playlist?.description ?? "";
    element<HTMLSelectElement>("youtube-privacy").value = playlist?.privacy ?? "private";
    element("youtube-edit").hidden = false;
    name.focus();
  }

  private closeForms(): void {
    this.editing = this.pending = null;
    for (const id of ["youtube-edit", "youtube-add", "youtube-confirm"]) element(id).hidden = true;
    element<HTMLInputElement>("youtube-song").value = "";
  }

  private setPopover(open: boolean): void {
    this.popover.hidden = !open;
    this.avatar.setAttribute("aria-expanded", String(open));
    if (open) this.popover.querySelector("button")?.focus();
  }

  private renderAccount(): void {
    this.avatar.dataset.connected = String(this.connected);
    setText(element("account-channel"), this.account?.channelId ?? "");
    element("login-setup").hidden = Boolean(this.account?.canConnect);
    const button = this.login.querySelector<HTMLButtonElement>('[data-youtube-action="connect"]')!;
    button.disabled = !this.account?.canConnect || this.busy;
  }

  private control(glyph: IconName, action: string, label: string): HTMLButtonElement {
    const button = actionButton(glyph, action, label);
    delete button.dataset.action;
    button.dataset.youtubeAction = action;
    return button;
  }

  private async execute(fn: () => Promise<void>): Promise<void> {
    try {
      await fn();
    } catch (error) {
      if (!this.signal.aborted) this.notify(localizeError(error), "error");
    }
  }

  private async perform(fn: () => Promise<void>): Promise<void> {
    if (this.busy || this.signal.aborted) return;
    this.busy = true;
    this.renderAccount();
    try {
      await this.execute(fn);
    } finally {
      this.busy = false;
      if (!this.signal.aborted) this.renderAccount();
    }
  }

  private async send<K extends keyof ReplyValues>(
    request: YoutubeRequest, kind: K,
  ): Promise<ReplyValues[K]> {
    if (!this.request) throw new Error(t("请在 Nasti 桌面客户端连接 YouTube", "Connect to YouTube from the Nasti desktop app"));
    const reply = await this.request(request);
    if (reply.kind !== kind || !("value" in reply)) throw new Error(t("YouTube 响应无效", "Invalid YouTube response"));
    return reply.value as ReplyValues[K];
  }
}
