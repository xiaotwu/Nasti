import type { NastiIpcContract, PlaybackOutput, PlaybackOutputs } from "../../../contracts/ipc-commands";
import { icon, type IconName } from "./icons";
import { localizeError, t } from "./i18n";
import { element, setText } from "./layout";

const deviceIcons: Record<PlaybackOutput["kind"], IconName> = {
  builtin: "laptop", headphones: "headphones", display: "display", speaker: "volume", other: "output",
};

export class OutputPicker {
  private readonly panel = element("output-panel");
  private readonly devices = element("output-devices");
  private readonly status = element("output-status");
  private readonly slider = element("output-volume-widget");
  private readonly trigger = element("output-anchor").querySelector<HTMLButtonElement>("button")!;
  private snapshot = "";
  private generation = 0;
  private refreshTimer = 0;
  private changing = false;

  constructor(private readonly ipc: NastiIpcContract, signal: AbortSignal) {
    this.devices.addEventListener("click", event => {
      const choice = event.target instanceof Element && event.target.closest<HTMLButtonElement>("[data-output]");
      if (choice) void this.select(choice.dataset.output || null);
    }, { signal });
    signal.addEventListener("abort", () => this.close(), { once: true });
  }

  get open(): boolean { return !this.panel.hidden; }

  async toggle(): Promise<void> {
    if (this.open) return this.close(true);
    this.panel.hidden = false;
    this.trigger.setAttribute("aria-expanded", "true");
    this.showStatus(t("正在读取播放设备", "Finding playback devices…"));
    await this.refresh(++this.generation, true);
  }

  close(restoreFocus = false): void {
    if (!this.open) return;
    this.generation++;
    window.clearTimeout(this.refreshTimer);
    this.panel.hidden = true;
    this.trigger.setAttribute("aria-expanded", "false");
    if (restoreFocus) this.trigger.focus({ preventScroll: true });
  }

  private async refresh(generation: number, focus = false): Promise<void> {
    try {
      const outputs = await this.ipc.playback_list_outputs();
      if (generation !== this.generation || this.changing) return;
      this.render(outputs);
      if (focus && document.activeElement === this.trigger) {
        this.devices.querySelector<HTMLButtonElement>('[aria-pressed="true"]')?.focus({ preventScroll: true });
      }
    } catch (error) {
      if (generation === this.generation && !this.changing) {
        this.showStatus(localizeError(error, "无法读取播放设备", "Could not read playback devices."));
      }
    } finally {
      if (generation === this.generation) {
        this.refreshTimer = window.setTimeout(() => void this.refresh(generation), 1800);
      }
    }
  }

  private async select(deviceId: string | null): Promise<void> {
    if (this.changing) return;
    this.changing = true;
    this.devices.setAttribute("aria-busy", "true");
    try {
      const outputs = await this.ipc.playback_set_output({ deviceId });
      if (this.open) this.render(outputs);
    } catch (error) {
      if (this.open) this.showStatus(localizeError(error, "无法切换播放设备", "Could not switch playback devices."));
    } finally {
      this.changing = false;
      this.devices.removeAttribute("aria-busy");
    }
  }

  private render(outputs: PlaybackOutputs): void {
    if (!outputs.supported) this.showStatus(t("播放设备由系统管理", "Playback devices are managed by your system."));
    else if (outputs.devices.length === 0) this.showStatus(t("暂无可用的播放设备", "No playback devices available."));
    else this.status.hidden = true;
    const snapshot = JSON.stringify(outputs);
    if (this.snapshot === snapshot) return;
    this.snapshot = snapshot;
    const active = document.activeElement;
    const focusedId = active instanceof HTMLButtonElement ? active.dataset.output : undefined;
    const activeSlider = this.slider.contains(active);
    this.panel.append(this.slider);
    this.slider.hidden = true;
    this.devices.replaceChildren();
    if (!outputs.supported || outputs.devices.length === 0) return;
    const systemDevice = outputs.devices.find(device => device.id === outputs.defaultDeviceId);
    const systemLabel = t("跟随系统", "System Default");
    const choices: { id: string; name: string; kind: PlaybackOutput["kind"] | "output" }[] = [
      { id: "", name: systemDevice ? systemLabel + " · " + systemDevice.name : systemLabel, kind: systemDevice?.kind ?? "output" },
      ...outputs.devices,
    ];
    for (const choice of choices) {
      const selected = (outputs.selectedDeviceId ?? "") === choice.id;
      const row = document.createElement("div");
      row.className = "output-row";
      row.dataset.selected = String(selected);
      const control = document.createElement("button");
      control.type = "button";
      control.className = "output-device";
      control.dataset.output = choice.id;
      control.setAttribute("aria-pressed", String(selected));
      control.title = choice.name;
      control.insertAdjacentHTML("beforeend", icon(choice.kind === "output" ? "output" : deviceIcons[choice.kind]));
      const name = document.createElement("span");
      name.className = "output-name";
      setText(name, choice.name);
      const indicator = document.createElement("span");
      indicator.className = "output-check";
      indicator.setAttribute("aria-hidden", "true");
      if (selected) indicator.innerHTML = icon("check");
      control.append(name, indicator);
      row.append(control);
      if (selected) {
        row.append(this.slider);
        this.slider.hidden = false;
      }
      this.devices.append(row);
      if (choice.id === focusedId) control.focus({ preventScroll: true });
    }
    if (activeSlider) this.slider.querySelector("input")?.focus({ preventScroll: true });
    else if (focusedId !== undefined && !this.devices.contains(document.activeElement)) this.trigger.focus({ preventScroll: true });
  }

  private showStatus(text: string): void {
    setText(this.status, text);
    this.status.hidden = false;
  }
}
