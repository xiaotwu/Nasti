import assert from "node:assert/strict";
import { nextRepeatMode, occurrenceId, repeatMode, TrackNavigation } from "../src/track-navigation.ts";

const [first, duplicate, third, fourth] = [
  { entryId: "a", videoId: "shared" }, { entryId: "b", videoId: "shared" },
  { entryId: "c", videoId: "other" }, { entryId: "d", videoId: "fourth" },
];
const playlist = { playlistId: "one", tracks: [first, duplicate, third, fourth] };
const navigation = new TrackNavigation(() => 0);
const sequential = { shuffle: false, wrap: false };
const shuffle = { shuffle: true, wrap: false };

assert.equal(navigation.step(playlist, 0, 1, sequential), 1, "play the repeated source occurrence separately");
assert.equal(navigation.step(playlist, 3, 1, sequential), null, "stop at the end by default");
assert.equal(navigation.step(playlist, 3, 1, { ...sequential, wrap: true }), 0);
assert.equal(navigation.step(playlist, 0, -1, { ...sequential, wrap: true }), 3);
assert.equal(navigation.step(playlist, 4, 1, shuffle), null);

assert.equal(navigation.step(playlist, 1, 1, shuffle), 2);
assert.equal(navigation.step(playlist, 2, 1, shuffle), 3);
assert.equal(navigation.step(playlist, 3, 1, shuffle), 0);
assert.equal(navigation.step(playlist, 0, 1, shuffle), null);
assert.equal(navigation.step(playlist, 2, -1, shuffle), 1, "previous follows the same shuffle order");
assert.equal(navigation.step(playlist, 0, 1, { ...shuffle, wrap: true }), 1);

const reordered = { ...playlist, tracks: [fourth, duplicate, first, third] };
assert.equal(navigation.step(reordered, 1, 1, shuffle), 3, "a source reorder retains the same shuffled occurrences");
const changed = { ...playlist, tracks: [duplicate, first] };
assert.equal(navigation.step(changed, 0, 1, shuffle), 1, "playlist membership changes cannot navigate to removed entries");
navigation.reset();
assert.equal(navigation.step(playlist, 2, -1, shuffle), null);
assert.notEqual(occurrenceId({ videoId: "legacy" }, 0), occurrenceId({ videoId: "legacy" }, 1));

assert.equal(repeatMode(null), "off");
assert.equal(repeatMode("garbage"), "off");
assert.equal(repeatMode("all"), "all");
assert.equal(repeatMode("one"), "one");
assert.deepEqual(["off", "all", "one"].map(nextRepeatMode), ["all", "one", "off"]);

console.log("[OK] Track navigation preserves playlist occurrences, shuffle order and legacy repeat defaults.");
