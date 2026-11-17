import type { NastiIpcContract, PlaylistColumn, PlaylistRemoval, TrackItem } from "../../../contracts/ipc-commands";
import { dateLocale, localizePlaylistTitle, localizeTrackTitle, t } from "./i18n";
import { element } from "./layout";
import { actionButton, empty, makeOverviewPlaylist } from "./views";

function control(action: string, label: string): HTMLButtonElement {
  const button = document.createElement("button");
  button.className = "text-button";
  button.type = "button";
  button.dataset.action = action;
  button.textContent = label;
  return button;
}

export class PlaylistEditor {
  private column: PlaylistColumn | null = null;
  private editing = false;
  private menuId: string | null = null;
  private confirming: string | null = null;
  private selected = new Set<string>();
  private busy = false;
  private pendingUndo = new Map<string, { removal: PlaylistRemoval; timer: number }>();
  private undoSequence = 0;
  private readonly picker = element("add-menu");

  constructor(
    private readonly ipc: NastiIpcContract,
    private readonly refresh: () => Promise<void>,
    private readonly redraw: () => void,
    private readonly message: (text: string) => void,
    signal: AbortSignal,
  ) {
    document.addEventListener("pointerdown", event => {
      if (event.target instanceof Element && !event.target.closest("#add-menu,[data-action=add-inspected]")) this.closePicker(false);
    }, { signal });
    signal.addEventListener("abort", () => {
      for (const item of this.pendingUndo.values()) clearTimeout(item.timer);
      this.pendingUndo.clear();
    }, { once: true });
  }

  decorate(column: PlaylistColumn | undefined, rows: HTMLElement[]): void {
    if (this.column?.playlistId !== column?.playlistId) {
      this.editing = false;
      this.confirming = this.menuId = null;
      this.selected.clear();
    }
    this.column = column ?? null;
    this.renderToolbar();
    if (!column?.isEditable) return;
    for (const row of rows) {
      if (row.dataset.entryId) this.decorateRow(row, column);
    }
  }

  async handle(action: string, button: HTMLButtonElement): Promise<void> {
    if (action === "undo-removal") return this.undo(button);
    const column = this.column;
    if (!column || this.busy) return;
    const row = button.closest<HTMLElement>(".track-row");
    const entryId = row?.dataset.entryId;
    switch (action) {
      case "manage-playlist":
        this.editing = !this.editing;
        this.menuId = this.confirming = null;
        this.selected.clear();
        return this.reflow('[data-action="manage-playlist"]');
      case "entry-menu":
        this.menuId = this.menuId === entryId ? null : entryId ?? null;
        this.confirming = null;
        return this.reflow(this.menuId ? '[data-action="confirm-entry"]' : '[data-action="entry-menu"][aria-expanded="false"]', entryId);
      case "confirm-entry":
        this.menuId = null;
        this.confirming = entryId ?? null;
        return this.reflow('[data-action="cancel-remove-entry"]', entryId);
      case "cancel-remove-entry":
        this.confirming = null;
        return this.reflow('[data-action="entry-menu"]', entryId);
      case "submit-remove-entry":
        if (entryId && this.confirming === entryId) await this.remove(column, [entryId]);
        return;
      case "submit-batch":
        if (this.selected.size) await this.remove(column, [...this.selected]);
        return;
    }
  }

  togglePicker(columns: PlaylistColumn[], trigger: HTMLButtonElement): void {
    if (!this.picker.hidden) return this.closePicker(true);
    const rows = columns.filter(column => column.isEditable).map(column => {
      const button = makeOverviewPlaylist(column, false);
      button.dataset.action = "choose-add";
      button.removeAttribute("aria-current");
      return button;
    });
    this.picker.replaceChildren(...(rows.length ? rows : [empty(t("暂无可编辑歌单", "No editable playlists"))]));
    this.picker.hidden = false;
    trigger.setAttribute("aria-expanded", "true");
    this.picker.querySelector("button")?.focus();
  }

  closePicker(focus: boolean): void {
    if (this.picker.hidden) return;
    this.picker.hidden = true;
    const trigger = document.querySelector<HTMLButtonElement>('[data-action="add-inspected"]');
    trigger?.setAttribute("aria-expanded", "false");
    if (focus) trigger?.focus({ preventScroll: true });
  }

  dismiss(): boolean {
    if (!this.picker.hidden) {
      this.closePicker(true);
      return true;
    }
    const id = this.confirming ?? this.menuId;
    if (!id || element("radar").hidden) return false;
    this.menuId = this.confirming = null;
    this.reflow('[data-action="entry-menu"]', id);
    return true;
  }

  async add(track: TrackItem | undefined, column: PlaylistColumn | undefined): Promise<void> {
    if (!track || !column?.isEditable || this.busy) return;
    this.busy = true;
    try {
      await this.ipc.playlist_add_track({ playlistId: column.playlistId, videoId: track.videoId });
      this.closePicker(true);
      await this.refresh();
      const playlistTitle = localizePlaylistTitle(column);
      this.message(t(`已加入「${playlistTitle}」`, `Added to “${playlistTitle}”`));
    } finally {
      this.busy = false;
    }
  }

  private renderToolbar(): void {
    const actions = element("overview-actions");
    actions.replaceChildren();
    const column = this.column;
    if (column?.isEditable && (column.tracks.length > 0 || this.editing)) {
      actions.append(actionButton(this.editing ? "check" : "editList", "manage-playlist",
        this.editing ? t("完成歌单编辑", "Finish editing playlist") : t("批量编辑歌单", "Edit playlist tracks")));
    }
    this.renderBatch();
  }

  private renderBatch(): void {
    const bar = element("overview-batch");
    bar.hidden = !this.editing;
    if (!this.editing) return;
    const count = document.createElement("span");
    const size = this.selected.size.toLocaleString(dateLocale());
    count.textContent = t(`已选 ${size} 首`, this.selected.size === 1 ? "1 track selected" : `${size} tracks selected`);
    count.setAttribute("aria-live", "polite");
    const button = control("submit-batch", t("移除所选", "Remove selected"));
    button.disabled = this.selected.size === 0 || this.busy;
    bar.replaceChildren(count, button);
  }

  private decorateRow(row: HTMLElement, column: PlaylistColumn): void {
    const id = row.dataset.entryId!;
    const trackIndex = Number(row.dataset.trackIndex);
    const track = column.tracks[trackIndex];
    if (!track?.entryId) return;
    const position = (trackIndex + 1).toLocaleString(dateLocale(), { useGrouping: false });
    const trackTitle = localizeTrackTitle(track);
    if (this.editing) {
      const box = document.createElement("input");
      box.type = "checkbox";
      box.checked = this.selected.has(id);
      box.setAttribute("aria-label", t(`选择第 ${position} 首 ${trackTitle}`, `Select track ${position}, ${trackTitle}`));
      box.addEventListener("change", () => {
        if (box.checked) this.selected.add(id);
        else this.selected.delete(id);
        this.renderBatch();
      });
      row.prepend(box);
      return;
    }
    if (this.confirming === id) {
      row.classList.add("remove-confirmation");
      const context = document.createElement("span");
      const repeated = column.tracks.filter(item => item.videoId === track.videoId).length > 1;
      const occurrence = repeated ? ` · ${t(`第 ${position} 项`, `Track ${position}`)}` : "";
      context.textContent = `${localizePlaylistTitle(column)} · ${trackTitle}${occurrence}`;
      row.replaceChildren(context, control("cancel-remove-entry", t("取消", "Cancel")), control("submit-remove-entry", t("移除", "Remove")));
      return;
    }
    const more = actionButton("more", "entry-menu", t(`曲目操作 ${trackTitle}`, `Track options for ${trackTitle}`));
    more.setAttribute("aria-expanded", String(this.menuId === id));
    row.append(more);
    if (this.menuId === id) {
      const menu = document.createElement("div");
      menu.className = "row-menu glass";
      menu.append(control("confirm-entry", t("从歌单移除", "Remove from playlist")));
      row.append(menu);
    }
  }

  private async remove(column: PlaylistColumn, entryIds: string[]): Promise<void> {
    this.busy = true;
    try {
      const removal = await this.ipc.playlist_remove_entries({ playlistId: column.playlistId, entryIds });
      this.menuId = this.confirming = null;
      this.selected.clear();
      this.showUndo(removal, localizePlaylistTitle(column));
      await this.refresh();
      element("overview-actions").querySelector("button")?.focus({ preventScroll: true });
    } finally {
      this.busy = false;
      this.renderBatch();
    }
  }

  private showUndo(removal: PlaylistRemoval, title: string): void {
    const key = String(++this.undoSequence);
    const box = document.createElement("div");
    box.className = "undo-notice glass";
    box.dataset.undo = key;
    const label = document.createElement("span");
    label.setAttribute("role", "status");
    const count = removal.entries.length.toLocaleString(dateLocale());
    label.textContent = t(`已从「${title}」移除 ${count} 首`,
      `Removed ${count} ${removal.entries.length === 1 ? "track" : "tracks"} from “${title}”`);
    const button = actionButton("undo", "undo-removal", t("撤销移除", "Undo removal"));
    button.dataset.undoKey = key;
    box.append(label, button);
    element("undo-notices").append(box);
    const timer = window.setTimeout(() => {
      if (button.disabled) return;
      this.pendingUndo.delete(key);
      box.remove();
    }, 10000);
    this.pendingUndo.set(key, { removal, timer });
  }

  private async undo(button: HTMLButtonElement): Promise<void> {
    const key = button.dataset.undoKey ?? "";
    const item = this.pendingUndo.get(key);
    if (!item) return;
    button.disabled = true;
    try {
      await this.ipc.playlist_restore_entries({ removal: item.removal });
      this.pendingUndo.delete(key);
      clearTimeout(item.timer);
      button.closest(".undo-notice")?.remove();
      await this.refresh();
      this.message(t("已撤销移除", "Removal undone"));
    } finally {
      button.disabled = false;
    }
  }

  private reflow(selector: string, entryId?: string) {
    const list = element("overview-tracks");
    const offset = list.scrollTop;
    this.redraw();
    list.scrollTop = offset;
    const row = entryId ? [...list.children].find(node => (node as HTMLElement).dataset.entryId === entryId) : element("overview-actions");
    row?.querySelector<HTMLElement>(selector)?.focus({ preventScroll: true });
  }
}
