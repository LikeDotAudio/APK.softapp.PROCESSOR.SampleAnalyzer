# 🔍 Architecture audit — PROCESSOR.SampleAnalyzer (2026-10-30)

Audited against the APK.audio multi-repo architecture standard: coupling, boundaries, duplication, contracts.
← [Audits](INDEX.md) · Improvements: [🛣Roadmap/IMPROVEMENTS.md](../%F0%9F%9B%A3Roadmap/IMPROVEMENTS.md)

## Bounded context
The Scanalyzer DSP core: audio file → `.PEAK` feature record (envelope, pitch, spectral, UCS class, regions, syntax), as a CLI, a library for the Tauri app and WASM engine, and the bus worker `Worker-SAMPLE_ANALYZER` (`SRC/`). Rust, ~70 source files. Sampled: `Cargo.toml`, `build.rs`, `src/lib.rs`, `src/Encoders/db.rs`, `src/Pipeline/analyze.rs`, the worker crate `SRC/` (Cargo.toml, `lib.rs`, `self_test.rs`, `config.ini`), `Docker/`, and the test inventory.

## Context map
| Talks to | How (topic / endpoint / file / import) | Relationship |
|---|---|---|
| CAPTURE.Extractor-Engine | `extractor_engine = { git = …, rev = … }` (`Cargo.toml`); its source is hashed via its `links` metadata `DEP_EXTRACTOR_ENGINE_SRC_DIR`; `PeakRegion` is the shared region schema | Shared Kernel by version |
| DATATYPE.soundfx.UCS | `apk-ucs-bundle` by `git = …, rev = f838404…`; `build.rs` reads its `DEP_APK_UCS_DIR` (`APK_UCS_DIR` overrides) | Conformist (versioned) |
| SCANALYZER.Like.audio `Web_Front/src-tauri`, `wasm_analyzer` | path dependency on this crate | Supplier |
| SCANALYZER Web_Front `upload_peak.php?columns` | `db.rs` fetches the endpoint's upload allow-list (`upload_columns.json`) and trims to it | Conformist to the endpoint |
| `upload_peak.php` (public site / scan-api) | HTTP POST via `ureq` in `src/Encoders/db.rs` | Customer |
| Broker | `…/Protocols/sample_analyzer/outgoing/job` → `…/incoming/result`; retained `incoming/api`, `config` via apk-plugin-runner | Published Language |
| APK.plugin.SDK (runner + contracts) | `SRC/Cargo.toml` by `tag = "sdk-v0.7"`; the image patches to the checkout's copies with `--config` | Conformist (versioned) |
| POD.workers | compose `name: workers`, external network `apk-audio_default` | Hosted |

## Scorecard
| Category | Score (0–3) | Finding |
|---|---|---|
| Coupling & Cohesion | 3 | Builds alone: engine, UCS bundle by `rev`, SDK by tag. The image's repo-root context is the house rule (kept by design). |
| Boundary Integrity | 3 | Decoders and HTTP upload sit behind `Decoders/` and `Encoders/db.rs`; the bus result is the published `.PEAK` schema, versioned and tested. |
| Duplication Control | 3 | One analyzer for CLI/Tauri/WASM/worker; one region definition; the upload trim follows the endpoint's one column file. The worker's second MQTT session is deliberate (kept by design). |
| Testing & Contracts | 3 | `SRC/check.sh` (97 tests incl. schema contract tests for the record and the worker result); `SRC/probe_job.sh` asks the live worker. |
| Change management | 3 | `ANALYZER_VERSION` (re-analysis) and `schema_version` (format) are separate, both stamped; SDK by tag, engine by rev. |

Score key: 3 meets the standard · 2 minor gaps · 1 significant gaps · 0 absent / violated.

## Findings
1. **Good — one implementation, four consumers.** CLI, Tauri app, WASM engine and worker all link this crate (README "Repository Overview"), and region cutting is delegated to Extractor-Engine so stored and interactive regions cannot drift.
2. **Good — data versioning by content hash.** `build.rs:186-196` stamps `ANALYZER_HASH` from the source and UCS bundle; every `.PEAK` carries `analyzer_version` (`src/Core/peak.rs:25`) so stale sidecars are detected and re-analysed.
4. **Hardened container.** `Docker/docker-compose.yml`: `cap_drop: [ALL]`, `read_only`, `no-new-privileges`, tmpfs sized to the job limit, no port — matches the worker pattern well.

Delivered items (engine by rev, SDK by tag, `.PEAK` schema, one upload column list, check.sh + live probe, and two kept-by-design decisions) are in [🌍Publishing/DELIVERY.md](../%F0%9F%8C%8DPublishing/DELIVERY.md).
