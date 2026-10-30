# Delivery — PROCESSOR.SampleAnalyzer

← [Wiki home](../README.md)

## Build and test

```bash
cargo build --release          # → <target>/release/oa_sample_analyzer
cargo test --release --lib     # 79 unit tests, all passing (2026-10-30)
```

It builds only inside the APK.audio checkout: `build.rs` reads
`../../DATATYPE/DATATYPE.soundfx.UCS/` and `Cargo.toml` path-depends on
`../../CAPTURE/CAPTURE.Extractor-Engine`. In the monorepo, cargo's target directory is the
root `target/`, not the crate's (`cargo metadata` says where).

## Where it lands

| Form | Built by | Lands |
|---|---|---|
| WASM, inside the Scanalyzer web app | The APK.audio monorepo `.github/workflows/ci.yml` step "Build the Scanalyzer" (`wasm-pack build wasm_analyzer --target web --release`, then `vite build`) on every push | https://apk.audio/APK:Softapps/SCANALYZER_SUITE/SCANALYZER.Like.audio/Web_Front/dist/ (FTPS, `WEB.Frontend-Assets/.apk.scripts/deploy.py`); Scanalyzer's own public home is https://scanalyzer.like.audio |
| Native, inside the Scanalyzer desktop app | `npm run tauri build` in Scanalyzer's `Web_Front` (path dependency) | The operator's machine |
| The `oa_sample_analyzer` CLI | `cargo build --release` by hand, or Scanalyzer's `main.py` | The operator's machine; the desktop app finds it by `SCAN_ANALYZER_BIN` |
| The worker `apk-worker-sample-analyzer` (`SRC/`) | `Docker/Dockerfile`, context the repo root (it copies contracts, the runner, CAPTURE.Extractor-Engine and the UCS categories beside this crate); `cargo build --release --locked` | the **Worker-SAMPLE_ANALYZER** container (`worker-sample-analyzer:local`, project `workers`, no port); after an edit, `docker compose -f PODS/POD.workers/Docker/docker-compose.yml up -d --build sample-analyzer` |

One image runs it: the worker above. The web-static image copies the Softapps tree (sources), so this
repo's `.rs` files are in that image as files nginx refuses to serve; the root
`.dockerignore` keeps `target/` out.

## Not automated

- **Tests.** No CI runs `cargo test`; the monorepo CI runs no tests (its header says so).
- **The CLI and the desktop app** are built and installed by hand.
- **A version bump re-analyzes every library.** A code or UCS data change (not a comment or
  formatting edit) changes `ANALYZER_VERSION`; the build warns, and the scan reports how many
  sidecars it re-analyzed and from which version.
