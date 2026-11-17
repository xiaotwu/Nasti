import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import {
  advanceExtraction, archiveColumns, archiveHeight, coast, EXTRACTED_LIFT, invertProjection, isArchiveResting, locateTrack,
  nearestOccurrence, restingExtraction, trackAt,
} from "../src/archive-layout.ts";

const root = fileURLToPath(new URL("../../../", import.meta.url));
for (const script of ["verify-contracts.mjs", "check-e2-tokens.mjs"]) {
  const result = spawnSync(process.execPath, [`scripts/${script}`], { cwd: root, stdio: "inherit" });
  assert.equal(result.status, 0);
}

const columns = archiveColumns([
  { columnIndex: 8, playlistId: "second", tracks: [{ videoId: "same", entryId: "second-entry" }] },
  { columnIndex: 3, playlistId: "first", tracks: [
    { videoId: "same", entryId: "first-entry" }, { videoId: "same", entryId: "repeat-entry" },
  ] },
]);
assert.equal(trackAt(columns, { lane: 0, row: -1 }).entryId, "repeat-entry");
assert.equal(trackAt(columns, { lane: -1, row: -1 }).entryId, "second-entry");
assert.deepEqual(locateTrack(columns, { lane: -1, row: 5 }, 3, 1), { lane: 0, row: 5 });
assert.equal(locateTrack(columns, { lane: 0, row: 0 }, 3, 2), null);
assert.equal(nearestOccurrence(0, 31, 32), 32);
assert.equal(trackAt([], { lane: 0, row: 0 }), null);

const projection = invertProjection({ x: 28, y: 19 }, { x: -5, y: 3 });
assert.ok(projection);
assert.deepEqual(projection(61, 35), { lane: 2, row: -1 });
assert.equal(invertProjection({ x: 1, y: 3 }, { x: 2, y: 6 }), null);
const short = coast(2, 13, 0.04);
const twice = coast(short.position, short.velocity, 0.04);
const long = coast(2, 13, 0.08);
assert.ok(Math.abs(twice.position - long.position) < 1e-9);
assert.ok(Math.abs(twice.velocity - long.velocity) < 1e-9);

const selected = { lane: 0, row: 0 };
const reselected = { lane: 1, row: 4 };
const displacement = (cell, focus, time) =>
  archiveHeight(cell, focus, time, 1, 1) - archiveHeight(cell, focus, time, 0, 1);
for (const cell of [selected, { lane: 0, row: 8 }, { lane: 3, row: -12 }]) {
  const samples = Array.from({ length: 53 }, (_, index) => displacement(cell, selected, index / 4));
  assert.ok(Math.max(...samples) - Math.min(...samples) > .10);
  assert.ok(samples.every(value => Math.abs(value) <= .102 + 1e-9));
  assert.ok(Math.abs(displacement(cell, selected, 3.25) - displacement(cell, reselected, 3.25)) < 1e-9);
}
assert.ok(Math.abs(displacement(selected, selected, 2) - (.075 + .027 * Math.sin(4 * Math.PI / 13))) < 1e-9);
console.log("[OK] Skyframe looping, drag projection, inertia and bounded idle breathing in every bank");

const yaw = -.48; // An archive inspected at an angle before the user goes back.
const step = (pose, open = false, dt = 1 / 60) => advanceExtraction(pose, open, yaw, dt, false);
const open = (count) => Array.from({ length: count }).reduce((pose) => step(pose, true), restingExtraction());
const opened = open(240);
assert.equal(opened.lift, EXTRACTED_LIFT);
assert.equal(opened.framing, 1);
assert.equal(opened.yaw, yaw);
let returning = opened;
while (returning.yaw) {
  const next = step(returning);
  assert.equal(next.lift, opened.lift, "a tilted cassette must stay above the deck while it aligns");
  assert.equal(next.framing, opened.framing, "the camera must not jump during alignment");
  assert.ok(Math.abs(next.yaw) < Math.abs(returning.yaw));
  returning = next;
}
assert.ok(step(returning).lift < returning.lift, "lowering starts after alignment");
for (let index = 0; index < 180; index++) {
  const next = step(returning);
  assert.ok(next.lift <= returning.lift && next.framing <= returning.framing);
  returning = next;
}
assert.ok(isArchiveResting(returning));
const interrupted = open(15);
assert.ok(interrupted.framing > 0 && interrupted.framing < 1);
returning = interrupted;
for (let index = 0; index < 240; index++) {
  const next = step(returning);
  assert.ok(next.framing <= returning.framing, "a close during extraction must never zoom back into detail");
  returning = next;
}
assert.deepEqual(returning, restingExtraction());
assert.deepEqual(advanceExtraction(opened, false, yaw, 1 / 60, true), restingExtraction());
const afterSecond = (hz) => Array.from({ length: hz }).reduce((pose) => step(pose, false, 1 / hz), opened);
assert.deepEqual(afterSecond(4), afterSecond(60), "dropped frames must not prolong the staged return");
for (const row of [-8, -1, 1, 8]) {
  const cell = { lane: 0, row };
  assert.ok(archiveHeight(cell, selected, 0, 0, 0) < archiveHeight(cell, selected, 0, 0, 1));
}
assert.equal(archiveHeight(selected, selected, 0, 0, 0), archiveHeight(selected, selected, 0, 0, 1));
console.log("[OK] Staged cassette return, mid-extraction reversal, reduced motion and recessed detail shelf");
