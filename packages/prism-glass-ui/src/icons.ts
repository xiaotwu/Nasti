const strokes = {
  search: '<circle cx="10.5" cy="10.5" r="5.8"/><path d="m15 15 4.2 4.2"/>',
  radar: '<circle cx="12" cy="12" r="8"/><circle cx="12" cy="12" r="4.4"/><path d="m12 12 6-6"/><circle cx="8.5" cy="15.3" r=".6" fill="currentColor" stroke="none"/>',
  library: '<path d="M4 6.5h16v3.3H4zM5.3 9.8v8.1h13.4V9.8M10 13h4"/>',
  globe: '<circle cx="12" cy="12" r="8.1"/><ellipse cx="12" cy="12" rx="3.7" ry="8.1"/><path d="M4 12h16"/>',
  settings: '<path d="m10 3.5-.5 2.3-1.8 1-2.3-.7-2 3.5 1.8 1.6v2.1l-1.8 1.6 2 3.5 2.3-.7 1.8 1 .5 2.3h4l.5-2.3 1.8-1 2.3.7 2-3.5-1.8-1.6v-2.1l1.8-1.6-2-3.5-2.3.7-1.8-1-.5-2.3z"/><circle cx="12" cy="12" r="2.7"/>',
  light: '<circle cx="12" cy="12" r="4.2"/><path d="M12 3v2m0 14v2M3 12h2m14 0h2M5.6 5.6 7 7m10 10 1.4 1.4m0-12.8L17 7M7 17l-1.4 1.4"/>',
  dark: '<path d="M19.3 15.1a7.8 7.8 0 0 1-10.4-10.4 7.9 7.9 0 1 0 10.4 10.4Z"/>',
  close: '<path d="m6 6 12 12M18 6 6 18"/>',
  back: '<path d="M19 12H5m0 0 6-6m-6 6 6 6"/>',
  up: '<path d="m6 15 6-6 6 6"/>',
  down: '<path d="m6 9 6 6 6-6"/>',
  locate: '<circle cx="12" cy="12" r="3.4"/><path d="M12 4v3m0 10v3M4 12h3m10 0h3"/>',
  more: '<path d="M5 12h.1m6.9 0h.1m6.9 0h.1" stroke-width="2.8"/>',
  play: '<path d="M8.5 5.5 19 12 8.5 18.5z" fill="currentColor" stroke="none"/>',
  pause: '<path d="M9 6v12m6-12v12" stroke-width="2.7"/>',
  previous: '<path d="M5.5 6v12"/><path d="m18 6-9.2 6 9.2 6z" fill="currentColor" stroke="none"/>',
  next: '<path d="M18.5 6v12"/><path d="m6 6 9.2 6L6 18z" fill="currentColor" stroke="none"/>',
  skipPrevious: '<path d="M21 6.2a.8.8 0 0 0-1.2-.7L12 10.1V6.2a.8.8 0 0 0-1.2-.7L2 11.3a.8.8 0 0 0 0 1.4l8.8 5.8a.8.8 0 0 0 1.2-.7v-3.9l7.8 4.6a.8.8 0 0 0 1.2-.7z" fill="currentColor" stroke="none"/>',
  skipNext: '<path d="M3 6.2a.8.8 0 0 1 1.2-.7l7.8 4.6V6.2a.8.8 0 0 1 1.2-.7l8.8 5.8a.8.8 0 0 1 0 1.4l-8.8 5.8a.8.8 0 0 1-1.2-.7v-3.9l-7.8 4.6a.8.8 0 0 1-1.2-.7z" fill="currentColor" stroke="none"/>',
  shuffle: '<path d="M3 6h2c5.7 0 7.7 12 13 12h3m-3-3 3 3-3 3M3 18h2c5.7 0 7.7-12 13-12h3m-3-3 3 3-3 3"/>',
  repeat: '<path d="M4 10V8a2 2 0 0 1 2-2h13m-3-3 3 3-3 3m4 5v2a2 2 0 0 1-2 2H5m3-3-3 3 3 3"/>',
  repeatOne: '<path d="M4 10V8a2 2 0 0 1 2-2h13m-3-3 3 3-3 3m4 5v2a2 2 0 0 1-2 2H5m3-3-3 3 3 3m3-10 1.5-1v6"/>',
  output: '<path d="M5.2 17.7a9 9 0 1 1 13.6 0M7.6 15a6 6 0 1 1 8.8 0m-6.5-2.2a2.9 2.9 0 1 1 4.2 0"/><path d="M12 13.4 6 21h12z" fill="currentColor" stroke="none"/>',
  laptop: '<rect x="4.5" y="5" width="15" height="11" rx="1.5"/><path d="M2 18.5h20"/>',
  headphones: '<path d="M4.5 13v-1a7.5 7.5 0 0 1 15 0v1M4.5 12.5h2.2c.7 0 1.3.6 1.3 1.3v4.4H6a1.5 1.5 0 0 1-1.5-1.5zm15 0h-2.2c-.7 0-1.3.6-1.3 1.3v4.4h2a1.5 1.5 0 0 0 1.5-1.5z"/>',
  display: '<rect x="3.3" y="4.7" width="17.4" height="12.4" rx="1.6"/><path d="M8.5 20h7"/>',
  chevron: '<path d="m9.5 6 6 6-6 6"/>',
  volume: '<path d="M4 9h3.3l4.5-3.5v13l-4.5-3.4H4zM15.6 9c1.5 1.7 1.5 4.3 0 6m2.4-8.1a7.4 7.4 0 0 1 0 10.2"/>',
  mute: '<path d="M4 9h3.3l4.5-3.5v13l-4.5-3.4H4zM15.5 9.5l5 5m0-5-5 5"/>',
  list: '<path d="M8.5 6.5h11m-11 5.5h11m-11 5.5h11M4.5 6.5h.2m-.2 5.5h.2m-.2 5.5h.2"/>',
  music: '<path d="M9 16.5V6l10-2v10.5M9 9l10-2"/><ellipse cx="6.5" cy="16.5" rx="2.5" ry="1.9"/><ellipse cx="16.5" cy="14.5" rx="2.5" ry="1.9"/>',
  editList: '<path d="M4.5 6h14m-14 5h8m-8 5h5m3 3 .7-3.6 5.1-5.1 2.9 2.9-5.1 5.1z"/>',
  lyrics: '<path d="M5 5.5h14v10H9l-4 3zm3.5 4h7m-7 3h4.5"/>',
  video: '<rect x="3.8" y="5.6" width="16.4" height="12.8" rx="2"/><path d="m10 9 5 3-5 3z" fill="currentColor" stroke="none"/>',
  cube: '<path d="M12 3.5 19.5 8v8L12 20.5 4.5 16V8zM4.7 8.2 12 12.5l7.3-4.3M12 12.5v8"/>',
  explode: '<path d="m12 4 8 4.4-8 4.4-8-4.4zm-8 8.3 8 4.3 8-4.3m-16 4 8 4.3 8-4.3"/>',
  glass: '<path d="M3.7 8.1 12 4l8.3 4.1v8.2L12 20l-8.3-3.7zm1.4 8.1L17.7 7m-8 11 9.8-7.2"/>',
  download: '<path d="M12 3.8v10.6m-4-4 4 4 4-4M5 15v4.2h14V15"/>',
  importAll: '<path d="M6 5h5m-5 4h5m-5 4h5m5-7v8m-3-3 3 3 3-3M5 16v3h14v-3"/>',
  storage: '<rect x="4" y="7" width="16" height="10" rx="2"/><path d="M7 11h6m-6 3h3m6-1h1"/>',
  undo: '<path d="M9 6 4 11l5 5M4 11h10a5 5 0 0 1 5 5v2"/>',
  check: '<path d="m5.5 12.5 4.2 4.2 8.8-9"/>',
  add: '<path d="M12 5v14M5 12h14"/>',
  remove: '<path d="M6 12h12"/>',
  import: '<path d="M7 12H3.5V5h11v3.5M11 13.5l3.5 3.5 5.5-6M20 16v3.5H7.5v-5"/>',
  skip: '<path d="m4 6 6.5 6L4 18m8-12 6.5 6-6.5 6"/>',
  restart: '<path d="M4.4 10a7.6 7.6 0 1 1 1.7 7.3M4.2 4.8v5.6h5.6"/>',
  power: '<path d="M12 3.8v8.1M7.1 6.4a7.6 7.6 0 1 0 9.8 0"/>',
  link: '<path d="M10 6h8v8m0-8-9 9m4 3H6V9"/>',
  account: '<circle cx="12" cy="8.5" r="3.2"/><path d="M5.2 19a6.8 6.8 0 0 1 13.6 0"/><circle cx="12" cy="12" r="9"/>',
  warning: '<path d="M10.6 4.6 3 18a1.3 1.3 0 0 0 1.1 2h15.8a1.3 1.3 0 0 0 1.1-2L13.4 4.6a1.6 1.6 0 0 0-2.8 0Z"/><path d="M12 9v4m0 3v.1"/>',
  error: '<circle cx="12" cy="12" r="8"/><path d="m9 9 6 6m0-6-6 6"/>',
  info: '<circle cx="12" cy="12" r="8"/><path d="M12 11.4v4.8m0-8.4v.1"/>',
} as const;

export type IconName = keyof typeof strokes;

export function icon(name: IconName): string {
  return '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.45" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' + strokes[name] + "</svg>";
}

export function button(name: IconName, action: string, label: string, extra = ""): string {
  return `<button class="icon-button" type="button" data-action="${action}" aria-label="${label}" title="${label}" ${extra}>${icon(name)}</button>`;
}

export function setIcon(button: HTMLButtonElement, name: IconName, label: string): void {
  if (button.dataset.icon !== name) {
    button.dataset.icon = name;
    button.innerHTML = icon(name);
  }
  if (button.getAttribute("aria-label") !== label) {
    button.setAttribute("aria-label", label);
    button.title = label;
  }
}
