const volumeSteps: Readonly<Record<string, number>> = {
  ArrowUp: 1, ArrowRight: 1, ArrowDown: -1, ArrowLeft: -1,
};

export function bindVolumeRanges(controls: readonly HTMLInputElement[], change: (volume: number) => void, signal: AbortSignal): void {
  let drag: { control: HTMLInputElement; pointerId: number; top: number } | null = null;
  const updateFromPointer = (clientX: number, bounds: DOMRect) => {
    const inset = Math.min(5, bounds.width / 4);
    const fraction = (clientX - bounds.left - inset) / (bounds.width - 2 * inset);
    change(Math.round(Math.max(0, Math.min(1, fraction)) * 100) / 100);
  };
  const release = () => { drag = null; };
  const releasePointer = (event: PointerEvent) => {
    if (drag?.pointerId === event.pointerId) release();
  };
  const move = (event: PointerEvent) => {
    if (!drag || drag.pointerId !== event.pointerId) return;
    const bounds = drag.control.getBoundingClientRect();
    const { clientX, clientY } = event;
    if (drag.control.disabled || bounds.width === 0 || Math.abs(bounds.top - drag.top) > 5
      || clientX < 0 || clientX >= window.innerWidth || clientY < 0 || clientY >= window.innerHeight) {
      release();
      return;
    }
    // WKWebView can deliver an active drag with a zero button mask.
    updateFromPointer(clientX, bounds);
  };

  for (const control of controls) {
    const surface = control.parentElement;
    surface?.addEventListener("pointerdown", event => {
      if (!event.isPrimary || event.button !== 0 || event.ctrlKey || control.disabled) return;
      const bounds = control.getBoundingClientRect();
      if (bounds.width === 0) return;
      // Keep mouse tracking in the app and use the range itself for keyboard and accessibility.
      event.preventDefault();
      if (surface.hasPointerCapture(event.pointerId)) surface.releasePointerCapture(event.pointerId);
      if (document.activeElement !== control) control.focus({ preventScroll: true });
      drag = { control, pointerId: event.pointerId, top: bounds.top };
      updateFromPointer(event.clientX, bounds);
    }, { signal });
    control.addEventListener("blur", () => {
      if (drag?.control === control) release();
    }, { signal });
    const updateNativeValue = () => change(control.valueAsNumber / 100);
    control.addEventListener("input", updateNativeValue, { signal });
    control.addEventListener("change", updateNativeValue, { signal });
    control.addEventListener("keydown", event => {
      if (event.defaultPrevented || event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return;
      const step = volumeSteps[event.key];
      if (step === undefined && event.key !== "Home" && event.key !== "End") return;
      event.preventDefault();
      const value = event.key === "Home" ? 0 : event.key === "End" ? 100
        : Math.max(0, Math.min(100, control.valueAsNumber + step));
      change(value / 100);
    }, { signal });
  }

  window.addEventListener("pointerdown", releasePointer, { capture: true, signal });
  window.addEventListener("pointermove", move, { capture: true, passive: true, signal });
  window.addEventListener("pointerup", event => {
    move(event);
    releasePointer(event);
  }, { capture: true, signal });
  window.addEventListener("pointercancel", releasePointer, { capture: true, signal });
  window.addEventListener("keydown", event => {
    if (event.key === "Escape") release();
  }, { capture: true, signal });
  window.addEventListener("blur", release, { signal });
  window.addEventListener("resize", release, { signal });
  document.documentElement.addEventListener("pointerleave", releasePointer, { signal });
  document.addEventListener("visibilitychange", release, { signal });
}
