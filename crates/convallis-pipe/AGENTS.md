# AGENTS.md — Agent-2: `convallis-pipe` (河谷骨哨)

## Territory & Boundaries
- **Allowed Write Path**: `crates/convallis-pipe/**`
- **Read-Only Contract**: `../../contracts/ipc-models.rs`
- **Forbidden**: Avoid modifying without Orchestrator sync: `contracts/`, `crates/uraniborg-core/`, `packages/`, or `hosts/`.

## Core Responsibilities
1. **Signature Cipher & PO-Token Sandbox Bridge (`src/cipher_sandbox/`)**:
   - Communicate with the hidden `1x1` WebView (or pure Rust extractor fallback) to decipher `signatureCipher`, transform the throttling `n` parameter, and generate `BotGuard PO Tokens`.
2. **Local HTTP Range Proxy with 500ms Fast-Start & 0ms Backward Scrubbing (`src/proxy_server/`)**:
   - Bind an `axum` server on `127.0.0.1:0` serving `/stream/:video_id?mode=audio|video`.
   - Buffer the entire played stream in a memory Ring Buffer backed by an on-disk LRU cache (`512MB - 20GB`), enabling `500ms` initial playback start and `0ms` instant backward seeking without network re-fetches (inspired by `BitChord` v1.1).
3. **Closed-Loop Vault Downloader (`src/vault_downloader/`)**:
   - Strictly forbid external local file imports.
   - Provide an 8-thread native HTTP Range segment downloader writing encrypted/indexed `.opus` and `.mp4` files into `{app_data_dir}/vault/`, with a managed `yt-dlp` binary fallback.

## Verification Command
```bash
cargo test -p convallis-pipe
```
