import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, statfsSync } from "node:fs";
import { resolve } from "node:path";

const separator = process.argv.indexOf("--");
if (separator < 3 || separator >= process.argv.length - 1) {
  throw new Error("Usage: node scripts/with-build-budget.mjs SCRATCH [OTHER_OUTPUT...] -- COMMAND [ARG...]");
}
const [scratch, ...otherOutputs] = process.argv.slice(2, separator).map(path => resolve(path));
const [command, ...args] = process.argv.slice(separator + 1);
mkdirSync(scratch, { recursive: true });

const gib = 1024 ** 3;
const scratchLimit = 8 * gib;
const freeFloor = 50 * gib;
const reserve = 384 * 1024 ** 2;
function usage() {
  const present = [scratch, ...otherOutputs].filter(path => existsSync(path));
  const result = spawnSync("/usr/bin/du", ["-sk", ...present], { encoding: "utf8" });
  const transient = result.stderr?.trim().split("\n")
    .every(line => line.endsWith("No such file or directory"));
  // A compiler may unlink a temporary file while du is walking its parent.
  if (result.status !== 0 && !(result.status === 1 && transient)) {
    throw new Error(result.stderr || "Unable to measure scratch space");
  }
  const allocated = result.stdout.trim().split("\n").map(line => Number.parseInt(line, 10));
  if (!allocated.every(Number.isFinite)) throw new Error("Missing scratch-space measurement");
  return allocated.reduce((sum, kib) => sum + kib * 1024, 0);
}
function freeBytes(path) {
  const { bavail, bsize } = statfsSync(path);
  return bavail * bsize;
}
function check() {
  const used = usage();
  const free = Math.min(freeBytes(scratch), freeBytes(process.cwd()));
  if (used >= scratchLimit - reserve) throw new Error(`Scratch budget reached (${(used / gib).toFixed(2)} GiB allocated)`);
  if (free < freeFloor + reserve) throw new Error(`Free space floor reached (${(free / gib).toFixed(2)} GiB available)`);
  return { used, free };
}

const started = check();
console.log(`Build budget: 8 GiB scratch, 50 GiB free floor. Available ${(started.free / gib).toFixed(1)} GiB.`);
const child = spawn(command, args, { stdio: "inherit", detached: true });
let halted = false;
function stop() {
  halted = true;
  if (!child.pid) return;
  try {
    process.kill(-child.pid, "SIGTERM");
  } catch (error) {
    if (error.code !== "ESRCH") throw error;
  }
}
const timer = setInterval(() => {
  try { check(); }
  catch (error) {
    clearInterval(timer);
    console.error(error.message);
    stop();
  }
}, 2000);
process.on("SIGINT", stop);
process.on("SIGTERM", stop);
child.on("error", error => console.error(error.message));
child.on("close", code => {
  clearInterval(timer);
  process.exitCode = halted ? 1 : code ?? 1;
});
