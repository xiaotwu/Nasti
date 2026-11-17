import fs from "node:fs";
import path from "node:path";

const requiredFiles = [
  "contracts/ipc-commands.ts",
  "contracts/ipc-models.rs",
  "contracts/scene-bridge.ts",
  "contracts/schema.sql",
  "contracts/mock-ipc.ts",
  "assets/foundry-art/nasti-illust/nasti_e2_reference.jpg",
  "assets/foundry-art/nasti-illust/e2-visual-tokens.json"
];

let missing = 0;
for (const rel of requiredFiles) {
  const full = path.resolve(process.cwd(), rel);
  if (!fs.existsSync(full)) {
    console.error(`[FAIL] Missing contract or reference asset: ${rel}`);
    missing++;
  } else {
    console.log(`[OK] Verified: ${rel} (${fs.statSync(full).size} bytes)`);
  }
}

if (missing > 0) {
  process.exit(1);
}
console.log("All contracts and E2 reference assets verified.");
