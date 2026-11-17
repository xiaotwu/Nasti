import { createHash } from "node:crypto";
import { readFileSync, readdirSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const manifest = readFileSync(resolve(root, "docs/RELEASE_FIDELITY_SHA256SUMS"), "utf8");
const frontend = process.argv[2] ? resolve(process.argv[2]) : null;

function verifyFile(path, expected) {
  const actual = createHash("sha256").update(readFileSync(path)).digest("hex");
  if (actual !== expected) throw new Error(`Approved release asset changed: ${path}`);
}

for (const line of manifest.trim().split("\n")) {
  const match = /^([a-f0-9]{64})  (\S+)$/.exec(line);
  if (!match) throw new Error(`Invalid release manifest line: ${line}`);
  const [, expected, source] = match;
  verifyFile(resolve(root, source), expected);
  if (frontend && source.startsWith("assets/")) {
    verifyFile(resolve(frontend, source.slice("assets/".length)), expected);
  }
}

function verifyEmptyCatalog(directory) {
  const files = readdirSync(directory);
  if (files.length !== 1 || files[0] !== "catalog.json") {
    throw new Error(`Release catalog must contain only catalog.json: ${directory}`);
  }
  const catalog = JSON.parse(readFileSync(resolve(directory, "catalog.json"), "utf8"));
  if (Object.keys(catalog).length !== 3
    || !Array.isArray(catalog.tracks) || catalog.tracks.length !== 0
    || !Array.isArray(catalog.playlists) || catalog.playlists.length !== 0
    || !catalog.lyrics || Array.isArray(catalog.lyrics) || typeof catalog.lyrics !== "object"
    || Object.keys(catalog.lyrics).length !== 0) {
    throw new Error(`Release catalog contains data: ${directory}`);
  }
}

verifyEmptyCatalog(resolve(root, "assets/authorized-media"));
if (frontend) verifyEmptyCatalog(resolve(frontend, "authorized-media"));

const host = JSON.parse(readFileSync(resolve(root, "hosts/load-bearing-crew/src-tauri/tauri.conf.json"), "utf8"));
const ui = JSON.parse(readFileSync(resolve(root, "packages/prism-glass-ui/package.json"), "utf8"));
if (host.productName !== "Nasti" || host.version !== ui.version) {
  throw new Error("Release requires the Nasti name and matching host and Settings versions");
}

console.log("Approved visuals, app name, release version and empty catalog verified.");
