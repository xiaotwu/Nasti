import type { NastiIpcContract, PlaylistImportPreview, TrackItem, VaultItem } from "../../../contracts/ipc-commands";
import { dateLocale, localizeError, localizePlaylistTitle, localizeTrack, localizeTrackTitle, t } from "./i18n";
import { durationLabel, element, safeAssetUrl, setText } from "./layout";
import { actionButton, empty, trackArtists, trackCountLabel } from "./views";

const MIB = 1048576;
function mb(bytes: number): string {
  const precision = bytes < MIB ? 2 : 1;
  const formatted = (bytes / MIB).toLocaleString(dateLocale(), {
    minimumFractionDigits: precision, maximumFractionDigits: precision,
  });
  return `${formatted} MiB`;
}

export class LibraryPanels {
  private preview: PlaylistImportPreview | null = null;
  private previewRevision = 0;
  private reading = false;
  private importing = false;
  private vaultRows = new Map<string, VaultItem>();
  private vaultRevision = 0;

  constructor(
    private readonly ipc: NastiIpcContract,
    private readonly refresh: () => Promise<void>,
    private readonly play: (track: TrackItem) => Promise<void>,
    private readonly message: (message: string) => void,
    signal: AbortSignal,
    private readonly openYoutube?: (reference: string) => Promise<void>,
  ) {
    element("import-form").addEventListener("submit", event => {
      event.preventDefault();
      void this.find(element<HTMLInputElement>("import-input").value);
    }, { signal });
    element("import-input").addEventListener("input", () => {
      this.previewRevision++;
      this.reading = false;
      this.preview = null;
      setText(element("import-status"), "");
      element("import-preview").hidden = true;
    }, { signal });
    element("vault-limit-form").addEventListener("submit", event => {
      event.preventDefault();
      const limit = element<HTMLInputElement>("vault-limit").valueAsNumber;
      void this.ipc.vault_set_storage_limit({ limitBytes: limit * MIB })
        .then(() => this.showVault(true))
        .catch(error => this.message(localizeError(error, "无法修改容量上限", "Could not change the storage limit")));
    }, { signal });
  }

  async find(reference: string): Promise<void> {
    const input = element<HTMLInputElement>("import-input");
    const value = reference.trim();
    if (!value || this.reading || this.importing) return;
    input.value = value;
    if (this.openYoutube && isYoutubeLink(value)) {
      await this.openYoutube(value);
      return;
    }
    const request = ++this.previewRevision;
    this.reading = true;
    this.preview = null;
    element("import-preview").hidden = true;
    setText(element("import-status"), t("正在查找歌单", "Looking up playlist…"));
    try {
      const preview = await this.ipc.catalog_preview_playlist({ urlOrId: value });
      if (request !== this.previewRevision) return;
      this.preview = preview;
      this.renderPreview();
      setText(element("import-status"), "");
    } catch (error) {
      if (request === this.previewRevision) {
        setText(element("import-status"), localizeError(error, "无法读取歌单", "Could not load the playlist"));
      }
    } finally {
      if (request === this.previewRevision) this.reading = false;
    }
  }

  async import(sourceIndex?: number): Promise<void> {
    if (!this.preview || this.importing) return;
    const sourceId = this.preview.sourceId;
    const input = element<HTMLInputElement>("import-input");
    this.importing = input.readOnly = true;
    for (const button of element("import-preview").querySelectorAll<HTMLButtonElement>("button")) button.disabled = true;
    setText(element("import-status"), t("正在导入", "Importing…"));
    try {
      this.preview = await this.ipc.catalog_apply_playlist_import({ sourceId, sourceIndex });
      await this.refresh();
      setText(element("import-status"), sourceIndex === undefined
        ? t("歌单已按原曲序补齐", "Playlist synced in its original order") : t("已导入曲目", "Track imported"));
    } catch (error) {
      setText(element("import-status"), localizeError(error, "无法导入歌单", "Could not import the playlist"));
    } finally {
      this.importing = input.readOnly = false;
      this.renderPreview();
      const next = element("import-preview").querySelector<HTMLButtonElement>("button:not([disabled])");
      if (next && element("panel").dataset.panel === "import") next.focus({ preventScroll: true });
    }
  }

  async showVault(capacity: boolean): Promise<void> {
    const request = ++this.vaultRevision;
    const list = element("library-list");
    element("library-panel").querySelector<HTMLElement>('[data-action="vault-capacity"]')!.hidden = capacity;
    element("library-panel").querySelector<HTMLElement>('[data-action="vault-drawer"]')!.hidden = !capacity;
    element("vault-limit-form").hidden = !capacity;
    element("vault-remaining").hidden = !capacity;
    list.replaceChildren(empty(t("正在读取离线副本", "Loading offline copies…")));
    try {
      const [stats, items] = await Promise.all([this.ipc.vault_get_storage_stats(), this.ipc.vault_list_tracks()]);
      if (request !== this.vaultRevision) return;
      this.vaultRows = new Map(items.map(item => [item.track.videoId, item]));
      setText(element("vault-stats"), mb(stats.usedBytes) + " / " + mb(stats.limitBytes));
      const remaining = mb(Math.max(0, stats.limitBytes - stats.usedBytes));
      setText(element("vault-remaining"), t(`剩余 ${remaining}`, `${remaining} available`));
      const progress = element<HTMLProgressElement>("vault-progress");
      progress.max = Math.max(1, stats.limitBytes);
      progress.value = stats.usedBytes;
      element<HTMLInputElement>("vault-limit").value = String(Math.ceil(stats.limitBytes / MIB));
      list.replaceChildren(...(items.length ? items.map(item => this.vaultRow(item)) : [empty(t("储藏库暂无离线副本", "Vault has no offline copies"))]));
    } catch (error) {
      if (request !== this.vaultRevision) return;
      list.replaceChildren(
        empty(localizeError(error, "无法读取储藏库", "Could not load Vault")),
        actionButton("restart", "retry-vault", t("重试读取储藏库", "Retry loading Vault")),
      );
    }
  }

  async handleVault(action: string, button: HTMLButtonElement): Promise<void> {
    const row = button.closest<HTMLElement>("[data-vault-id]");
    const saved = this.vaultRows.get(row?.dataset.vaultId ?? "");
    if (!saved || !row) return;
    if (action === "play-vault") return this.play(saved.track);
    const menu = row.querySelector<HTMLElement>(".row-menu")!;
    if (action === "vault-menu") {
      for (const item of element("library-list").querySelectorAll<HTMLElement>(".row-menu:not([hidden])")) {
        if (item !== menu) {
          item.hidden = true;
          item.parentElement?.querySelector('[data-action="vault-menu"]')?.setAttribute("aria-expanded", "false");
        }
      }
      menu.hidden = !menu.hidden;
      button.setAttribute("aria-expanded", String(!menu.hidden));
      if (!menu.hidden) menu.querySelector("button")?.focus();
      return;
    }
    button.disabled = true;
    try {
      await this.ipc.vault_remove_track({ videoId: saved.track.videoId });
      await this.refresh();
      await this.showVault(element("panel").dataset.panel === "capacity");
      this.message(t("已移除离线副本", "Offline copy removed"));
    } finally {
      button.disabled = false;
    }
  }

  private renderPreview(): void {
    const preview = this.preview;
    if (!preview) return;
    element("import-preview").hidden = false;
    const title = localizePlaylistTitle({ playlistId: preview.sourceId, title: preview.title });
    setText(element("import-summary"), `${title} · ${trackCountLabel(preview.tracks.length)}`);
    const list = element("import-list");
    if (!preview.tracks.length) list.replaceChildren(empty(t("来源歌单暂无曲目", "The source playlist has no tracks")));
    else list.replaceChildren(...preview.tracks.map(({ track, sourceIndex, imported }) => {
      const row = document.createElement("div");
      row.className = "import-track";
      const number = document.createElement("small");
      number.textContent = (sourceIndex + 1).toLocaleString(dateLocale(), { useGrouping: false, minimumIntegerDigits: 2 });
      const copy = this.trackCopy(track);
      const title = localizeTrackTitle(track);
      const label = imported ? t(`已导入 ${title}`, `${title} imported`) : t(`导入 ${title}`, `Import ${title}`);
      const button = actionButton(imported ? "check" : "import", "import-one", label);
      button.dataset.sourceIndex = String(sourceIndex);
      button.disabled = imported;
      row.append(number, copy, button);
      return row;
    }));
    const all = element("import-preview").querySelector<HTMLButtonElement>('[data-action="import-all"]')!;
    all.disabled = preview.tracks.length === 0;
  }

  private trackCopy(track: TrackItem): HTMLElement {
    const copy = document.createElement("span");
    copy.className = "track-copy";
    const strong = document.createElement("strong");
    strong.textContent = strong.title = localizeTrackTitle(track);
    const byline = document.createElement("small");
    byline.textContent = trackArtists(localizeTrack(track));
    byline.title = byline.textContent;
    copy.append(strong, byline);
    return copy;
  }

  private vaultRow({ track, bytes }: VaultItem): HTMLElement {
    const row = document.createElement("div");
    row.className = "vault-row";
    row.dataset.vaultId = track.videoId;
    const cover = document.createElement("img");
    cover.className = "track-cover";
    cover.alt = "";
    cover.loading = "lazy";
    cover.src = safeAssetUrl(track.thumbnailSquareUrl);
    const info = this.trackCopy(track);
    const size = document.createElement("small");
    size.className = "vault-item-meta";
    const media = track.isVaultVideoDownloaded ? t("音频与视频", "Audio and video") : t("音频", "Audio");
    size.textContent = [durationLabel(track.durationSec * 1000), mb(bytes), media].join(" · ");
    info.append(size);
    const title = localizeTrackTitle(track);
    const more = actionButton("more", "vault-menu", t(`副本操作 ${title}`, `Offline copy options for ${title}`));
    more.setAttribute("aria-expanded", "false");
    const menu = document.createElement("div");
    menu.className = "row-menu glass";
    menu.hidden = true;
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "text-button";
    remove.dataset.action = "remove-vault-copy";
    remove.textContent = t("移除离线副本", "Remove offline copy");
    menu.append(remove);
    row.append(cover, info, actionButton("play", "play-vault", t(`播放离线副本 ${title}`, `Play offline copy of ${title}`)), more, menu);
    return row;
  }
}

function isYoutubeLink(value: string): boolean {
  try {
    const url = new URL(value);
    return url.protocol === "https:"
      && ["music.youtube.com", "www.youtube.com", "youtube.com", "m.youtube.com"].includes(url.hostname)
      && url.searchParams.has("list");
  } catch {
    return false;
  }
}
