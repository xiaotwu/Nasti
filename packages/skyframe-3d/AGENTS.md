# AGENTS.md — Agent-3: `skyframe-3d` (高空桁架 3D 阵列与解密引擎)

## Territory & Boundaries
- **Allowed Write Path**: `packages/skyframe-3d/**`, `assets/foundry-art/**`
- **Read-Only Contract**: `../../contracts/scene-bridge.ts`, `../../contracts/ipc-commands.ts`, `../../docs/NASTI_E2_ART_VISUAL_SPEC.md`, `../../assets/foundry-art/nasti-illust/e2-visual-tokens.json`

## Core Responsibilities
1. **Nasti E2 Boot Sequence (`src/boot/`)**:
   - Replicate `LBEILC/RhineLabUI` 25fps timeline (`6.76s - 27.00s`), 38ms typing audio, and dual-ring logo drawing.
   - Replace identity confirmation with `ID CONFIRMED : NASTI LUNOREY // CHIEF ENGINEER` and trigger the **45° Rotated Diamond Prism reveal** of Nasti's Official Elite 2 artwork (`nasti_e2_reference.jpg`).
2. **9×32 Infinite 3D Cassette Array — Scheme A: Column = Playlist, Cassette = Song (`src/archive-loop/`, `src/cassette-renderer/`)**:
   - Preserve exact `RhineLabUI` geometry (`width=5.0, height=3.7, depth=0.31, rowSpacing=0.62, colSpacing=5.2`), camera (`azimuth=59°, elevation=19°, dist=35`), 2×2 inverse projection matrix free planar drag with inertia, hover lift (`0.28`), selection lift (`0.4`), positive cosine wave, and `8s/13s` breathing.
   - **High-Readability Song Metadata Baking**: Use `OffscreenCanvas (1024x1024)` to bake `[Track #] · [Song Title] — [Artist]`, `[Duration]`, and status inlay color (`#e8cf8c` Gold = Song, `#ff5a6e` Crimson = MV available, `#6c8c3c` Olive = Vault Downloaded) onto the cassette top bevel/lightguide, and bind the square album cover onto the internal substrate behind the `roughness=0.42` frosted cover.
3. **100% Replicated Extraction Decryption & 360° Disassembly Viewer (`src/decryption-stage/`, `src/model-viewer/`)**:
   - Lift selected song cassette to `4.05`, rotate camera to `azimuth=18°, elevation=13.8°, dist=72, near=5, far=300` (anchored in the left `38%` of viewport).
   - Render the **Cyan-Emerald (`#48f0c8`) diagonal decryption laser line** inspired by Nasti's E2 raised acoustic wand, sweeping top-to-bottom (`24%` feather band) from `roughness=0.42` to `0.025`.
   - Drive internal dual-ring rotation and 6-part disassembly micro-vibrations via `AudioSpectrumFrame`.
4. **0% GPU Background Hibernation (`src/quality-pipeline/`)**:
   - Implement 4 quality presets, 9 granular settings, Super Performance Mode, and completely halt `requestAnimationFrame` when `setPowerSavingPaused(true)` is active.

## Verification Command
```bash
pnpm --filter @nasti/skyframe-3d test
```
