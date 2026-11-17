# AGENTS.md — Agent-5: `load-bearing-crew` (应急承重小组 — 跨平台宿主集成)

## Territory & Boundaries
- **Allowed Write Path**: `hosts/**`
- **Read-Only Contract**: `../../contracts/ipc-commands.ts`, `../../contracts/ipc-models.rs`

## Core Responsibilities
1. **Desktop Host (`hosts/load-bearing-crew/` — macOS & Windows 11)**:
   - Wire all `uraniborg-core` and `convallis-pipe` services into Tauri 2.0 `#[tauri::command]` handlers matching `NastiIpcContract`.
   - Configure macOS native `NSVisualEffectView` (`UnderWindowBackground` / `HudWindow`) with `titleBarStyle: "Overlay"`.
   - Manage the lazy in-app YouTube Music login WebView (`https://music.youtube.com`), intercepting session cookies upon sign-in and storing them securely in macOS Keychain / Windows Credential Manager via `keyring`.
   - Spawn the hidden `1x1` Cipher WebView for `convallis-pipe`.
   - Integrate macOS `MPNowPlayingInfoCenter`, media keys, Menu Bar tray icon, and battery power-source listener (auto-toggling Super Performance Mode & 0% GPU background hibernation).
2. **Multi-Platform Host Scaffolds**:
   - Maintain build and bridge configurations for `hosts/qc-specialist/` (iOS), `hosts/foreman-specialist/` (Android), and `hosts/trimounts-liaison/` (HarmonyOS NEXT via Tauri OpenHarmony NAPI adapter).

## Verification Command
```bash
cargo check --manifest-path hosts/load-bearing-crew/src-tauri/Cargo.toml
```
