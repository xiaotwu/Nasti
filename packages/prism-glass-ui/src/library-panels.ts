import type { TerminalIpc } from "./playback";
import type { ToastLevel } from "./youtube-panel";
import type { TrackItem, VaultItem } from "../../../contracts/ipc-commands";
import { dateLocale, localizeError, localizeTrack, localizeTrackTitle, t } from "./i18n";
import { durationLabel, element, safeAssetUrl, setText } from "./layout";
import { actionButton, empty, trackArtists } from "./views";

const MIB = 1048576;
function mb(bytes: number): string {
  const precision = bytes < MIB ? 2 : 1;
  const formatted = (bytes / MIB).toLocaleString(dateLocale(), {
    minimumFractionDigits: precision, maximumFractionDigits: precision,
  });
  return `${formatted} MiB`;
}

export class LibraryPanels {
  private vaultRows = new Map<string, VaultItem>();
  private vaultRevision = 0;

  constructor(
    private readonly ipc: TerminalIpc,
    private readonly refresh: () => Promise<void>,
    private readonly play: (track: TrackItem) => Promise<void>,
    private readonly message: (message: string, level?: ToastLevel) => void,
    signal: AbortSignal,
  ) {
    element("vault-limit-form").addEventListener("submit", event => {
      event.preventDefault();
      const limit = element<HTMLInputElement>("vault-limit").valueAsNumber;
      void this.ipc.vault_set_storage_limit({ limitBytes: limit * MIB })
        .then(() => this.showVault())
        .catch(error => this.message(localizeError(error, "无法修改容量上限", "Could not change the storage limit"), "error"));
    }, { signal });
  }

  async showVault(): Promise<void> {
    const request = ++this.vaultRevision;
    const list = element("library-list");
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
      this.message(localizeError(error, "无法读取储藏库", "Could not load Vault"), "error");
      list.replaceChildren(
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
      await this.showVault();
      this.message(t("已移除离线副本", "Offline copy removed"), "success");
    } finally {
      button.disabled = false;
    }
  }

  private trackCopy(track: TrackItem): HTMLElement {
    const copy = document.createElement("span");
    copy.className = "track-copy";
    const title = document.createElement("strong");
    title.textContent = track.title;
    const subtitle = document.createElement("small");
    subtitle.textContent = trackArtists(track);
    copy.append(title, subtitle);
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
    size.textContent = [track.durationSec > 0 ? durationLabel(track.durationSec * 1000) : "", mb(bytes), media].filter(Boolean).join(" · ");
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
