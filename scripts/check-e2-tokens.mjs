import fs from "node:fs";

const raw = fs.readFileSync("assets/foundry-art/nasti-illust/e2-visual-tokens.json", "utf8");
const data = JSON.parse(raw);

if (data.e2ParallaxLayers.length !== 7) {
  throw new Error("Expected 7 parallax layers for Nasti Elite 2 artwork.");
}
if (!data.themes.celestialCyan || !data.themes.bansheeObsidian) {
  throw new Error("Missing E2 color themes.");
}
console.log("[OK] Nasti Elite 2 visual tokens validated: 7 parallax layers & dual E2 themes ready.");
