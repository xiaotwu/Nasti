import { element } from "./layout";

export class BootSequence {
  private readonly root = element("boot");
  private readonly artwork = this.root.querySelector<HTMLImageElement>(".boot-artwork");
  private timer = 0;
  private sequence = 0;
  private focused: HTMLElement | null = null;

  constructor() {
    this.root.querySelector(".boot-backdrop")?.addEventListener("animationend", () => {
      if (this.root.dataset.running === "true") this.skip();
    });
  }

  get active(): boolean { return !this.root.hidden; }

  start(reduced: boolean, onEnd: () => void): void {
    if (this.active) return;
    const sequence = ++this.sequence;
    this.onEnd = onEnd;
    this.focused = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    element("shell-top").inert = true;
    element("transport-dock").inert = true;
    for (const id of ["scene", "panel", "radar", "detail", "inspection-tools"]) element(id).inert = true;
    this.root.dataset.reduced = String(reduced);
    this.root.dataset.running = "false";
    this.root.hidden = false;
    this.root.querySelector("button")?.focus({ preventScroll: true });

    const beginAnimation = () => {
      if (!this.active || this.sequence !== sequence || this.root.dataset.running === "true") return;
      clearTimeout(this.timer);
      this.root.dataset.running = "true";
      // Let the painted fade finish even when the first frame is delayed.
      this.timer = window.setTimeout(() => this.skip(), reduced ? 250 : 3000);
    };
    if (reduced || !this.artwork) {
      beginAnimation();
      return;
    }
    // Keep the first frame on the theme color without holding startup indefinitely.
    this.timer = window.setTimeout(beginAnimation, 650);
    void this.artwork.decode().then(beginAnimation, beginAnimation);
  }

  private onEnd: (() => void) | null = null;

  skip(): void {
    if (!this.active) return;
    clearTimeout(this.timer);
    this.root.hidden = true;
    for (const id of ["shell-top", "transport-dock", "scene", "panel", "radar", "detail", "inspection-tools"]) element(id).inert = false;
    this.focused?.focus({ preventScroll: true });
    this.focused = null;
    this.onEnd?.();
    this.onEnd = null;
  }
}
