import { icon } from "./icons";
import { t } from "./i18n";
import {
  conflictingShortcut, defaultShortcuts, isReservedShortcut, loadShortcutBindings, readShortcut,
  shortcutAction, shortcutActions, shortcutAlternatives, type ShortcutAction,
} from "./shortcut-bindings";

const storageKey = "nasti.keyboard-shortcuts";
const labels: Record<ShortcutAction, [zh: string, en: string]> = {
  "play-toggle": ["播放或暂停", "Play or Pause"], previous: ["上一首", "Previous Track"], next: ["下一首", "Next Track"],
  "seek-backward": ["后退 5 秒", "Back 5 Seconds"], "seek-forward": ["前进 5 秒", "Forward 5 Seconds"],
  "volume-down": ["调低音量", "Volume Down"], "volume-up": ["调高音量", "Volume Up"], mute: ["切换静音", "Toggle Mute"],
  search: ["检索曲库", "Search Library"], radar: ["曲库总览", "Library Overview"],
  library: ["离线储藏库", "Vault"], settings: ["终端设置", "Settings"],
};

function label(action: ShortcutAction): string { return t(...labels[action]); }

function row(action: ShortcutAction): string {
  return `<div class="setting-row shortcut-row" data-shortcut="${action}">
    <strong>${label(action)}</strong>
    <div class="shortcut-controls">
      <button type="button" class="text-button shortcut-binding" data-shortcut-command="record"><kbd></kbd></button>
      <button type="button" class="icon-button shortcut-clear" data-shortcut-command="clear">${icon("close")}</button>
    </div>
  </div>`;
}

export function shortcutSettings(): string {
  return `<div class="shortcut-head">
    <p>${t("Nasti 窗口在前台时生效。点按键位可更改。", "Works while Nasti is focused. Select a shortcut to change it.")}</p>
    <button type="button" class="text-button" data-shortcut-command="reset">${t("恢复默认", "Reset Defaults")}</button>
  </div>
  <p class="shortcut-status" role="status"></p>
  <div aria-label="${t("播放快捷键", "Playback Shortcuts")}">
    ${shortcutActions.slice(0, 8).map(row).join("")}
  </div>
  <div class="shortcut-group" aria-label="${t("浏览快捷键", "Navigation Shortcuts")}">
    ${shortcutActions.slice(8).map(row).join("")}
  </div>`;
}

const mac = /Mac|iPhone|iPad/.test(navigator.platform);

function display(chord: string): string {
  const names: Record<string, string> = {
    Control: mac ? "⌃" : "Ctrl", Alt: mac ? "⌥" : "Alt", Shift: mac ? "⇧" : "Shift", Meta: mac ? "⌘" : "Win",
    Space: t("空格", "Space"), Plus: "+", ArrowLeft: "←", ArrowRight: "→", ArrowUp: "↑", ArrowDown: "↓",
    PageUp: t("上一页", "Page Up"), PageDown: t("下一页", "Page Down"), Backspace: t("退格", "Backspace"),
    Delete: t("删除", "Delete"), Insert: t("插入", "Insert"), Home: t("行首", "Home"), End: t("行尾", "End"),
  };
  if (chord === "Primary+,") return mac ? "⌘," : "Ctrl+,";
  return chord.split("+").map(part => names[part] ?? part).join(mac ? "" : "+");
}

function preferences(): string | null {
  try { return localStorage.getItem(storageKey); } catch { return null; }
}

export class ApplicationShortcuts {
  private readonly settings = document.querySelector<HTMLElement>("#settings-shortcuts")!;
  private bindings = loadShortcutBindings(preferences());
  private recording: ShortcutAction | null = null;

  constructor(signal: AbortSignal, private readonly changed: () => void) {
    this.settings.addEventListener("click", this.handleClick, { signal });
    this.settings.addEventListener("focusout", event => {
      if (this.recording && !(event.relatedTarget instanceof Element && event.relatedTarget.closest(`[data-shortcut="${this.recording}"]`))) this.finish();
    }, { signal });
    window.addEventListener("blur", () => { if (this.recording) this.finish(); }, { signal });
    this.render();
  }

  match(event: KeyboardEvent): ShortcutAction | undefined {
    const chord = readShortcut(event);
    return chord ? shortcutAction(this.bindings, chord) : undefined;
  }

  title(action: ShortcutAction, base: string): string {
    const binding = this.bindings[action];
    if (binding === null) return base;
    const name = display(binding);
    return base + t(`（${name}）`, ` (${name})`);
  }

  record(event: KeyboardEvent): boolean {
    if (!this.recording) return false;
    if (event.key === "Tab") { this.finish(); return false; }
    event.preventDefault();
    event.stopPropagation();
    if (event.repeat) return true;
    if (event.key === "Escape") { this.finish(); return true; }
    if (["Control", "Alt", "Meta", "Shift", "AltGraph"].includes(event.key)) return true;
    const chord = readShortcut(event);
    if (!chord || isReservedShortcut(chord)) {
      this.status(t("此按键用于输入、导航或系统操作。请换一个组合。", "This key is reserved for typing, navigation, or system commands. Try another combination."));
      return true;
    }
    const conflict = conflictingShortcut(this.bindings, this.recording, chord);
    if (conflict) {
      const name = label(conflict);
      this.status(t(`此快捷键已用于「${name}」。`, `This shortcut is already assigned to ${name}.`));
      return true;
    }
    this.bindings[this.recording] = chord;
    this.commit(t("快捷键已保存。", "Shortcut saved."));
    return true;
  }

  private readonly handleClick = (event: MouseEvent): void => {
    const button = event.target instanceof Element ? event.target.closest<HTMLButtonElement>("[data-shortcut-command]") : null;
    if (!button) return;
    const action = button.closest<HTMLElement>("[data-shortcut]")?.dataset.shortcut as ShortcutAction | undefined;
    switch (button.dataset.shortcutCommand) {
      case "record":
        if (!action) return;
        if (this.recording === action) return this.finish();
        this.recording = action;
        button.focus({ preventScroll: true });
        this.render();
        this.status(t("按下新快捷键，或按 Esc 取消。", "Press a new shortcut, or Esc to cancel."));
        break;
      case "clear":
        if (!action) return;
        this.bindings[action] = null;
        this.commit(t("快捷键已清除。", "Shortcut cleared."));
        button.closest("[data-shortcut]")?.querySelector<HTMLButtonElement>(".shortcut-binding")?.focus();
        break;
      case "reset":
        this.bindings = { ...defaultShortcuts };
        this.commit(t("已恢复默认快捷键。", "Default shortcuts restored."));
        break;
    }
  };

  private commit(message: string): void {
    try { localStorage.setItem(storageKey, JSON.stringify(this.bindings)); }
    catch { message = t("此更改将在本次使用期间生效。", "This change applies for the current session."); }
    this.finish(message);
    this.changed();
  }

  private finish(message = ""): void {
    this.recording = null;
    this.render();
    this.status(message);
  }

  private status(message: string): void {
    this.settings.querySelector(".shortcut-status")!.textContent = message;
  }

  private render(): void {
    for (const action of shortcutActions) {
      const row = this.settings.querySelector(`[data-shortcut="${action}"]`)!;
      const change = row.querySelector<HTMLButtonElement>(".shortcut-binding")!;
      const clear = row.querySelector<HTMLButtonElement>(".shortcut-clear")!;
      const binding = this.bindings[action];
      const displayed = binding === null ? t("未设置", "Unassigned") : display(binding);
      const name = label(action);
      const recording = this.recording === action;
      const prompt = recording ? t("等待按键", "Listening…") : displayed;
      change.querySelector("kbd")!.textContent = prompt;
      change.setAttribute("aria-pressed", String(recording));
      change.setAttribute("aria-label", t(`更改${name}的快捷键，${prompt}`, `Change shortcut for ${name}, ${prompt}`));
      const clearLabel = t(`清除${name}的快捷键`, `Clear shortcut for ${name}`);
      clear.disabled = binding === null;
      clear.title = clearLabel;
      clear.setAttribute("aria-label", clearLabel);
      for (const control of document.querySelectorAll<HTMLButtonElement>(`button[data-action="${action}"]`)) {
        if (binding) control.setAttribute("aria-keyshortcuts", shortcutAlternatives(binding).join(" "));
        else control.removeAttribute("aria-keyshortcuts");
        if (action !== "play-toggle" && action !== "mute") {
          const original = control.dataset.shortcutDescription ?? control.getAttribute("aria-label") ?? name;
          control.dataset.shortcutDescription = original;
          const description = this.title(action, original);
          control.title = description;
          control.setAttribute("aria-label", description);
        }
      }
    }
  }
}
