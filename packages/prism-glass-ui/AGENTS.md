# AGENTS.md — Agent-4: `prism-glass-ui` (折射棱镜 Liquid Glass 与精英二动态图层)

## Territory & Boundaries
- **Allowed Write Path**: `packages/prism-glass-ui/**`
- **Read-Only Contract**: `../../contracts/ipc-commands.ts`, `../../contracts/scene-bridge.ts`, `../../docs/NASTI_E2_ART_VISUAL_SPEC.md`, `../../assets/foundry-art/nasti-illust/e2-visual-tokens.json`

## Core Responsibilities
1. **Apple Liquid Glass Material Pipeline (`src/materials/`)**:
   - Implement real-time SVG `<feDisplacementMap>` edge refraction, `backdrop-filter: blur(28px) saturate(195%)`, subtle chromatic aberration, mouse-tracked radial specular rim highlights, and spring-physics morphing containers (`stiffness: 320, damping: 28`).
2. **Strictly Icon-Only 4 Consolidated Capsules (`src/capsules/`)**:
   - **Zero Static Text Mandate**: Every button, filter, and mode toggle MUST use SF Symbols + Rhine Lab geometric icons with zero static text labels:
     - `SearchRadarPill` (Top-Center)
     - `TerminalSyncPill` (Top-Right)
     - `MasterPlayPill` (Bottom-Center, including the 32-bar **Golden Soundwave Barcode** replicated from the bottom-right of Nasti's E2 art)
     - `DetailModePill` (Right-Stage Top)
3. **Right-Hand 62% Detail Stage (`src/detail-stage/`)**:
   - Smoothly cross-fade (`180ms`) when a 3D cassette is extracted on the left:
     - **Tracklist & Two-Way YTM Sync**: Add/remove songs from YTM playlists and trigger closed-loop Vault downloads.
     - **Word-by-Word Synced Lyrics**: Syllable-level fluid glow highlighting, duet split, and translation lines.
     - **Inline 4K/60fps MV Stage**: Seamless Audio <-> MV stream switching inside a Liquid Glass rounded viewport while streaming WebAudio FFT data (`AudioSpectrumFrame`) to the 3D scene.
4. **Nasti Official Elite 2 (`立绘_娜斯提_2.jpg`) 7-Layer 2.5D Parallax Renderer (`src/nasti-atelier/`)**:
   - Render `assets/foundry-art/nasti-illust/nasti_e2_reference.jpg` using the exact 7-layer specification in `e2-visual-tokens.json` (45° Rotated Diamond Prism Frame, cyan-emerald wand bloom on high frequencies, mechanical exoskeleton micro-vibration on sub-bass, hair/coat mesh wave, and golden soundwave barcode equalizer).

## Verification Command
```bash
pnpm --filter @nasti/prism-glass-ui test
```
