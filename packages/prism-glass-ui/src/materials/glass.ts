export const glassFilters = `<svg class="glass-filters" aria-hidden="true" width="0" height="0"><defs>
  <filter id="prism-edge" x="-5%" y="-25%" width="110%" height="150%" color-interpolation-filters="sRGB">
    <feTurbulence type="fractalNoise" baseFrequency="0.018 0.075" numOctaves="1" seed="18" result="prism-waves"/>
    <feDisplacementMap in="SourceGraphic" in2="prism-waves" scale="4" xChannelSelector="R" yChannelSelector="G"/>
  </filter>
</defs></svg>`;

export function installGlassMotion(root: HTMLElement): AbortController {
  const controller = new AbortController();
  const { signal } = controller;
  const reduced = matchMedia("(prefers-reduced-motion: reduce)");

  root.addEventListener("pointermove", event => {
    if (!(event.target instanceof Element) || reduced.matches) return;
    const glass = event.target.closest<HTMLElement>(".glass");
    if (!glass) return;
    const rect = glass.getBoundingClientRect();
    glass.style.setProperty("--glass-x", (event.clientX - rect.left) / rect.width * 100 + "%");
    glass.style.setProperty("--glass-y", (event.clientY - rect.top) / rect.height * 100 + "%");
    glass.style.setProperty("--glass-hit", "1");
  }, { passive: true, signal });

  root.addEventListener("pointerout", event => {
    if (!(event.target instanceof Element)) return;
    const glass = event.target.closest<HTMLElement>(".glass");
    if (glass && !(event.relatedTarget instanceof Node && glass.contains(event.relatedTarget))) {
      glass.style.setProperty("--glass-hit", "0");
    }
  }, { passive: true, signal });

  root.addEventListener("pointerup", event => {
    if (!(event.target instanceof Element) || reduced.matches) return;
    const button = event.target.closest<HTMLElement>(".capsule .icon-button");
    if (!button) return;
    springButton(button);
  }, { passive: true, signal });
  return controller;
}

function springButton(button: HTMLElement): void {
  const frames: Keyframe[] = [];
  let displacement = -0.13;
  let velocity = 0;
  const step = 1 / 120;
  for (let index = 0; index <= 48; index++) {
    frames.push({ transform: `scale(${1 + displacement})` });
    velocity += (-320 * displacement - 28 * velocity) * step;
    displacement += velocity * step;
  }
  button.getAnimations().forEach(animation => animation.cancel());
  button.animate(frames, { duration: 400, easing: "linear" });
}
