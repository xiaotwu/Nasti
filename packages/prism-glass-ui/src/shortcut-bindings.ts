export const shortcutActions = [
  "play-toggle", "previous", "next", "seek-backward", "seek-forward", "volume-down", "volume-up", "mute",
  "search", "radar", "library", "settings",
] as const;

export type ShortcutAction = typeof shortcutActions[number];
export type ShortcutBindings = Record<ShortcutAction, string | null>;

export const defaultShortcuts: Readonly<ShortcutBindings> = {
  "play-toggle": "Space", previous: null, next: null, "seek-backward": null, "seek-forward": null,
  "volume-down": null, "volume-up": null, mute: "M", search: "/", radar: "V", library: null, settings: "Primary+,",
};

type ShortcutEvent = Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "altKey" | "shiftKey" | "metaKey" | "isComposing" | "getModifierState">;

const keyPattern = /^(?:[A-Z0-9.,/;\[\]'`=\\-]|Plus|Space|Arrow(?:Left|Right|Up|Down)|Home|End|PageUp|PageDown|Insert|Delete|Backspace|F(?:[1-9]|1[0-9]))$/;
const chordPattern = /^(?:Control\+)?(?:Alt\+)?(?:Shift\+)?(?:Meta\+)?([^+]+)$/;
const printable = /^(?:[A-Z0-9.,/;\[\]'`=\\-]|Plus|Space)$/;
const nativeCommandKey = /^(?:A|C|F|H|M|Q|V|W|X|Y|Z|Space|`|ArrowLeft|ArrowRight|Backspace)$/;

export function readShortcut(event: ShortcutEvent): string | null {
  if (event.isComposing || event.getModifierState("AltGraph")) return null;
  let key = event.key === " " ? "Space" : event.key === "+" ? "Plus" : event.key;
  if (/^[a-zA-Z]$/.test(key)) key = key.toUpperCase();
  if (!keyPattern.test(key) && event.altKey && /^Key[A-Z]$/.test(event.code)) key = event.code.slice(3);
  if (!keyPattern.test(key)) return null;
  return (event.ctrlKey ? "Control+" : "") + (event.altKey ? "Alt+" : "")
    + (event.shiftKey ? "Shift+" : "") + (event.metaKey ? "Meta+" : "") + key;
}

export function isReservedShortcut(chord: string): boolean {
  if (chord === "Primary+,") return false;
  const key = chord.match(chordPattern)?.[1];
  if (!key || !keyPattern.test(key)) return true;
  const command = chord.includes("Meta+") || chord.includes("Control+");
  const modified = command || chord.includes("Alt+");
  if (!modified && !printable.test(key) && !key.startsWith("F")) return true;
  if (command && nativeCommandKey.test(key) && !chord.includes("Alt+")) return true;
  return ["Control+Meta+F", "Alt+Meta+H", "Alt+Meta+M", "Alt+F4"].includes(chord);
}

export function shortcutAlternatives(chord: string): string[] {
  return chord === "Primary+," ? ["Meta+,", "Control+,"] : [chord];
}

export function conflictingShortcut(bindings: ShortcutBindings, action: ShortcutAction, chord: string): ShortcutAction | undefined {
  const keys = shortcutAlternatives(chord);
  return shortcutActions.find(other => {
    const value = bindings[other];
    return other !== action && value !== null && shortcutAlternatives(value).some(key => keys.includes(key));
  });
}

export function shortcutAction(bindings: ShortcutBindings, chord: string): ShortcutAction | undefined {
  return shortcutActions.find(action => {
    const binding = bindings[action];
    return binding !== null && shortcutAlternatives(binding).includes(chord);
  });
}

export function loadShortcutBindings(json: string | null): ShortcutBindings {
  let saved: unknown;
  try { saved = JSON.parse(json ?? "null"); } catch { return { ...defaultShortcuts }; }
  if (typeof saved !== "object" || saved === null || Array.isArray(saved)) return { ...defaultShortcuts };
  const overrides = saved as Partial<Record<ShortcutAction, unknown>>;
  const bindings = { ...defaultShortcuts };
  for (const action of shortcutActions) {
    const value = overrides[action];
    if (value === null || (typeof value === "string" && !isReservedShortcut(value) && (action === "settings" || value !== "Primary+,"))) {
      bindings[action] = value;
    }
  }
  for (const action of shortcutActions) {
    const value = bindings[action];
    if (value !== null && conflictingShortcut(bindings, action, value)) {
      const fallback = defaultShortcuts[action];
      bindings[action] = fallback !== null && !conflictingShortcut(bindings, action, fallback) ? fallback : null;
    }
  }
  return bindings;
}
