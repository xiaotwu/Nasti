import assert from "node:assert/strict";
import {
  conflictingShortcut, defaultShortcuts, isReservedShortcut, loadShortcutBindings, readShortcut, shortcutAction,
} from "../src/shortcut-bindings.ts";

const key = (key, overrides = {}) => readShortcut({
  key, code: "", isComposing: false, metaKey: false, altKey: false, shiftKey: false, ctrlKey: false,
  getModifierState: () => false, ...overrides,
});

assert.equal(key("m"), "M");
assert.equal(key("M", { shiftKey: true }), "Shift+M");
assert.equal(key(" "), "Space");
assert.equal(key("+", { shiftKey: true }), "Shift+Plus");
assert.equal(key("å", { altKey: true, code: "KeyA" }), "Alt+A");
assert.equal(key("m", { isComposing: true }), null);
assert.equal(key("m", { getModifierState: () => true }), null);
assert.equal(key("Escape"), null);
assert.ok(isReservedShortcut("Tab"));
assert.ok(isReservedShortcut("ArrowLeft"));
assert.ok(isReservedShortcut("Meta+Q"));
assert.ok(isReservedShortcut("Alt+Meta+H"));
assert.ok(isReservedShortcut("Shift+Meta+Z"));
assert.ok(isReservedShortcut("Control+Meta+F"));
assert.ok(!isReservedShortcut("Alt+ArrowRight"));

assert.deepEqual(loadShortcutBindings("invalid JSON"), defaultShortcuts);
assert.deepEqual(loadShortcutBindings("[]"), defaultShortcuts);
assert.equal(shortcutAction(defaultShortcuts, "Meta+,"), "settings");
assert.equal(shortcutAction(defaultShortcuts, "Control+,"), "settings");
assert.equal(conflictingShortcut(defaultShortcuts, "next", "Control+,"), "settings");
assert.equal(conflictingShortcut(defaultShortcuts, "previous", "M"), "mute");
assert.equal(conflictingShortcut(defaultShortcuts, "mute", "M"), undefined);

const customized = { ...defaultShortcuts, mute: null, next: "M", settings: "Meta+J" };
const readback = loadShortcutBindings(JSON.stringify(customized));
assert.deepEqual(readback, customized);
assert.equal(shortcutAction(readback, "M"), "next");
assert.equal(shortcutAction(readback, "Meta+,"), undefined);
assert.equal(shortcutAction(readback, "Control+,"), undefined);
assert.equal(shortcutAction(readback, "Meta+J"), "settings");
assert.equal(loadShortcutBindings('{"next":"V","mute":"Meta+Q"}').next, null);
assert.equal(loadShortcutBindings('{"next":"V","mute":"Meta+Q"}').mute, "M");

console.log("[OK] Shortcuts preserve defaults, reserve system/text keys, reject collisions, and persist reassigned bindings.");
