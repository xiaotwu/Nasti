import { localizeError, t } from "./i18n";
import { element, setText } from "./layout";
import type { AppUpdater } from "./playback";

export class SettingsNavigation {
  private readonly scroll = element("settings-scroll");
  private readonly links = [...element("settings-panel").querySelectorAll<HTMLButtonElement>("[data-section]")];
  private readonly sections = this.links.map(button => element("settings-" + button.dataset.section));
  private active = 0;
  private target: number | null = null;
  private animation = 0;

  constructor(signal: AbortSignal) {
    this.scroll.addEventListener("scroll", () => this.update(), { signal, passive: true });
    for (const event of ["wheel", "touchstart", "pointerdown", "keydown"]) {
      this.scroll.addEventListener(event, () => this.cancel(), { signal, passive: true });
    }
    const resize = new ResizeObserver(() => this.measure());
    resize.observe(this.scroll);
    for (const section of this.sections) resize.observe(section);
    signal.addEventListener("abort", () => { this.cancel(); resize.disconnect(); }, { once: true });
    this.highlight(0);
  }

  measure(): void {
    if (!this.scroll.clientHeight) return;
    // Leave enough scroll range to align the final heading with the top of the pane.
    const last = this.sections[this.sections.length - 1];
    this.scroll.style.paddingBottom = Math.max(0, this.scroll.clientHeight - last.offsetHeight) + "px";
  }

  jump(key: string | undefined, reduced: boolean): void {
    const index = this.links.findIndex(button => button.dataset.section === key);
    if (index < 0) return;
    this.cancel();
    const start = this.scroll.scrollTop;
    const end = this.top(index);
    const animate = !reduced && Math.abs(index - this.active) === 1 && start !== end;
    this.highlight(index);
    if (!animate) {
      this.scroll.scrollTop = end;
      return;
    }
    this.target = index;
    const began = performance.now();
    const step = (now: number) => {
      const amount = Math.min(1, (now - began) / 180);
      this.scroll.scrollTop = start + (end - start) * (1 - Math.pow(1 - amount, 3));
      if (amount < 1) this.animation = requestAnimationFrame(step);
      else this.cancel();
    };
    this.animation = requestAnimationFrame(step);
  }

  private top(index: number): number {
    return this.sections[index].offsetTop;
  }

  private cancel(): void {
    cancelAnimationFrame(this.animation);
    this.target = null;
  }

  private update(): void {
    if (this.target !== null || !this.scroll.clientHeight) return;
    let index = 0;
    for (let next = 1; next < this.sections.length && this.top(next) <= this.scroll.scrollTop + 12; next++) index = next;
    this.highlight(index);
  }

  private highlight(index: number): void {
    this.active = index;
    for (const [i, button] of this.links.entries()) {
      button.setAttribute("aria-current", i === index ? "true" : "false");
    }
  }
}


export class SettingsUpdates {
  private readonly status = element("settings-update-status");
  private readonly controls = element("settings-update-controls");
  private readonly checkButton = element<HTMLButtonElement>("settings-update-check");
  private readonly installButton = element<HTMLButtonElement>("settings-update-install");
  private availableVersion: string | null = null;
  private busy = false;

  constructor(private readonly updater: AppUpdater, private readonly signal: AbortSignal) {
    element("settings-update").hidden = false;
    this.checkButton.addEventListener("click", () => void this.check(), { signal });
    this.installButton.addEventListener("click", () => void this.install(), { signal });
  }

  private setBusy(busy: boolean): void {
    this.busy = busy;
    for (const button of [this.checkButton, this.installButton]) {
      button.setAttribute("aria-disabled", String(busy));
    }
    this.controls.setAttribute("aria-busy", String(busy));
  }

  private async check(): Promise<void> {
    if (this.busy) return;
    this.setBusy(true);
    setText(this.status, t("正在检查更新…", "Checking for updates…"));
    try {
      const update = await this.updater.check();
      if (this.signal.aborted) return;
      this.availableVersion = update.availableVersion ?? null;
      this.installButton.hidden = !this.availableVersion;
      setText(this.status, this.availableVersion
        ? t(`版本 ${this.availableVersion} 可用。`, `Version ${this.availableVersion} is available.`)
        : t(`已是最新版本（${update.currentVersion}）。`, `You’re on the latest version (${update.currentVersion}).`));
    } catch (error) {
      if (this.signal.aborted) return;
      this.availableVersion = null;
      this.installButton.hidden = true;
      setText(this.status, localizeError(error, "无法检查更新，请重试。", "Could not check for updates. Try again."));
    } finally {
      if (!this.signal.aborted) this.setBusy(false);
    }
  }

  private async install(): Promise<void> {
    const version = this.availableVersion;
    if (this.busy || !version) return;
    this.setBusy(true);
    setText(this.status, t(`正在下载并安装 ${version}，完成后应用会重启…`, `Downloading and installing ${version}. Nasti will restart when it’s ready…`));
    try {
      await this.updater.install({ version });
      if (!this.signal.aborted) setText(this.status, t("更新已安装，正在重启…", "Update installed. Restarting…"));
    } catch (error) {
      if (this.signal.aborted) return;
      setText(this.status, localizeError(error, "无法安装更新，请重试。", "Could not install the update. Try again."));
      this.setBusy(false);
    }
  }
}
