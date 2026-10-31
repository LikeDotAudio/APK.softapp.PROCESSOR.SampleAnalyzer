# Delivery — PROCESSOR.SampleAnalyzer

← [Wiki home](../README.md)

## Build and test

```bash
cargo build --release          # → <target>/release/oa_sample_analyzer
./check.sh                     # analyzer (85) + worker (12) tests, all passing (2026-10-30)
./check.sh --live              # ... plus SRC/probe_job.sh: a probe job answered by the running worker
```

A clone builds on its own (Extractor-Engine by `rev`, the SDK by tag `sdk-v0.5`) given
`APK_UCS_DIR=<a DATATYPE.soundfx.UCS checkout>`. In the monorepo, locally excluded
`.cargo/config.toml` files patch both back to the local folders, and cargo's target directory
is the root `target/` (`cargo metadata` says where).

## Where it lands

| Form | Built by | Lands |
|---|---|---|
| WASM, inside the Scanalyzer web app | The APK.audio monorepo `.github/workflows/ci.yml` step "Build the Scanalyzer" (`wasm-pack build wasm_analyzer --target web --release`, then `vite build`) on every push | https://apk.audio/APK:Softapps/SCANALYZER_SUITE/SCANALYZER.Like.audio/Web_Front/dist/ (FTPS, `WEB.Frontend-Assets/.apk.scripts/deploy.py`); Scanalyzer's own public home is https://scanalyzer.like.audio |
| Native, inside the Scanalyzer desktop app | `npm run tauri build` in Scanalyzer's `Web_Front` (path dependency) | The operator's machine |
| The `oa_sample_analyzer` CLI | `cargo build --release` by hand, or Scanalyzer's `main.py` | The operator's machine; the desktop app finds it by `SCAN_ANALYZER_BIN` |
| The worker `apk-worker-sample-analyzer` (`SRC/`) | `Docker/Dockerfile`, context the repo root by design (it copies contracts, the runner, CAPTURE.Extractor-Engine and the UCS categories beside this crate, and patches the git names to them with `--config`, so the image never fetches GitHub); `cargo build --release --locked` | the **Worker-SAMPLE_ANALYZER** container (`worker-sample-analyzer:local`, project `workers`, no port); after an edit, `docker compose -f PODS/POD.workers/Docker/docker-compose.yml up -d --build sample-analyzer` |

One image runs it: the worker above. The web-static image copies the Softapps tree (sources), so this
repo's `.rs` files are in that image as files nginx refuses to serve; the root
`.dockerignore` keeps `target/` out.

## Not automated

- **Tests.** `./check.sh` is run by hand; the monorepo CI runs no tests (its header says so).
- **The CLI and the desktop app** are built and installed by hand.
- **A version bump re-analyzes every library.** A code or UCS data change (not a comment or
  formatting edit) changes `ANALYZER_VERSION`; the build warns, and the scan reports how many
  sidecars it re-analyzed and from which version.

## Delivered from the 2026-10-30 audit
- **Extractor-Engine by commit, UCS by explicit location** — `Cargo.toml` names `extractor_engine` by `git = …, rev = e66e6be…`; `build.rs` reads the engine's source from its `links` metadata (`DEP_EXTRACTOR_ENGINE_SRC_DIR`) and the UCS bundle from `APK_UCS_DIR` (or the sibling folder). A clean copy of the working tree with no `.cargo/` built the worker standalone (SDK fetched by tag, `APK_UCS_DIR` set). The UCS cargo package is a handoff (see the audit); the image keeps the repo-root context by design (below). Part of the Worker-SAMPLE_ANALYZER image (pod POD.workers) and of Scanalyzer's WASM/Tauri builds.
- **Worker on the SDK by tag** — `SRC/Cargo.toml` names `apk-plugin-runner` / `apkaudio-contracts` by `tag = "sdk-v0.5"` like the THICK plugins; the health probe is built with `-p apk-plugin-runner --bin apk-plugin-health` instead of a path into the runner's tree. `Docker/Dockerfile` patches the three git names with `--config` (the broker image's recipe). Verified: `docker build` of the image succeeded with both binaries; `cargo test` in `SRC/` 12/12.
- **Kept by design: repo-root image context** — the house rule for every image that links the SDK (`.apk.scripts/plugin_standalone.py`, DATABUS.Broker.MQTT's Dockerfile): the crate builds alone from git, the image builds the checkout with `[patch]`es so it never fetches GitHub and ships the bench's own runner, contracts and engine. Written in the Dockerfile header and PRINCIPLES.
- **`.PEAK` schema with `schema_version`, owned here** — Scanalyzer's strict `analyzerPeakSchema.json` (analyzer-peak 1.1.0) moved to `schema/peak.schema.json` (one copy; `junctions[].gap_level_above_noise_floor_decibels` made nullable, which the analyzer writes). `metadata.format` / `metadata.schema_version` are stamped again (`PEAK_FORMAT`, `PEAK_SCHEMA_VERSION` in `src/Core/schema.rs`, with `PEAK_SCHEMA` and `validate`). Tests: an analysed two-burst file (with nested per-region analyses) and the worker's job result both validate; the constant equals the `$id` semver. Scanalyzer's `wasm_analyzer` tests every tracked `.PEAK` its readers load against the same schema through this crate.
- **One upload column list** — `db.rs` no longer carries its own field tree: it asks the endpoint (`GET upload_peak.php?columns`, Scanalyzer's `Web_Front/public/api/upload_columns.json`) and trims with one generic `slim`; `peakUpload.ts` and `upload_peak.php` read the same file. Sends `X-Upload-Token` from `APK_SCAN_UPLOAD_TOKEN`. Tests: `slim_keeps_only_the_listed_fields`, `a_missing_section_is_sent_empty_not_dropped`.
- **One check command + a live probe** — `./check.sh` (both crates) and `SRC/probe_job.sh` (posts a probe WAV on `sample_analyzer/outgoing/job`, PASS on the matching `incoming/result`). Verified 2026-10-30: check.sh green; the probe answered `ok: true` by the running Worker-SAMPLE_ANALYZER.
- **Kept by design: the job loop's own MQTT session** — the runner's session carries config, will and the self-test door; a multi-second analysis on it would stall them (`run_job_agent` doc in `SRC/src/lib.rs`, PRINCIPLES). The runner still supplies config, LWT, api and health.
- **One `.PEAK` region definition** — `peak.rs::Region` flattens `extractor_engine::PeakRegion` and adds `analysis`; JSON unchanged. Note: this changes the analyzer's source, so `ANALYZER_VERSION` moves and existing sidecars re-analyze once.
- **CC-03 the crate under SRC/** (SDK/contracts audit) — the analyzer crate moved from the stack root to `SRC/core/` (Cargo.toml, Cargo.lock, build.rs, src/, examples/, schema/); the worker links it as `path = "core"` and excludes it from its workspace; `build.rs`'s UCS sibling path, `Docker/Dockerfile`, `display.json` `entry` and `check.sh` follow. Consumers by git find the package anywhere in the repository; Scanalyzer re-pinned to this commit. Verified: `./check.sh` green, worker image `docker build` OK.
- **WK-07 requires block** (SDK/contracts audit) — `display.json` declares `requires: {plugins: [sample_analyzer], services: [broker]}` and drops its unread topic.
- **Spec citations repointed** (S07 handoff) — `ucs.rs` and `morphology.rs` cite the UCS signature spec at its new home, DATATYPE.soundfx.UCS `Documentation/🧩Manual and Artifacts/ucs_signature_spec.md` (comments only; `ANALYZER_VERSION` unchanged).
- **Kept by design: the `.PEAK` schema is published here, not copied into root `contracts/`** (S07 asked for one there) — the format's owner holds the one schema (`SRC/core/schema/peak.schema.json`, `$id` https://apk.audio/schemas/analyzer-peak/1.1.0) and hands it on through the crate (`oa_sample_analyzer::schema::PEAK_SCHEMA`); a second copy under `contracts/` would be the duplicate this audit removed. contracts can reference it by `$id`. Checking feature names against DATATYPE.soundfx.UCS `signature_vocabulary.json` is a test that belongs with whichever repo owns that vocabulary.
