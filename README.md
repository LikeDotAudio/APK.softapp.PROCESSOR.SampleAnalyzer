# PROCESSOR.SampleAnalyzer

> ⚙️ **Sample Analyzer** — the Rust DSP core of the Scanalyzer suite. It walks an audio library, analyzes every file in parallel, classifies it against the UCS sound taxonomy, and writes a `.PEAK` JSON record beside each file plus one aggregate for the library.

---

## 🧩 Architectural Role
* **Family**: PROCESSOR (data processing and transformations)
* **Category**: Scanalyzer / DSP Core (`display.json` `group` / `subGroup`)
* **Pipeline Stage**: analysis — audio files in, `.PEAK` feature records out; feeds the Scanalyzer views (3D cloud, Examiner, Groups) and the shared cloud database

---

## ⚙️ Repository Overview
A standalone Git repository, mounted as a submodule at `PODS/POD.THIN/Softapps/PROCESSOR/PROCESSOR.SampleAnalyzer` in the APK.audio monorepo. It is one crate, `oa-sample-analyzer`, used four ways:

| Consumer | How |
|---|---|
| The `oa_sample_analyzer` CLI (`src/main.rs`) | Batch scan of a directory |
| Scanalyzer desktop app (`SCANALYZER_SUITE/SCANALYZER.Like.audio/Web_Front/src-tauri`) | Path dependency; calls `analyze::analyze_buffer`, `decode::read_audio`, `db::write_peaks` in-process |
| Scanalyzer WASM engine (`SCANALYZER_SUITE/SCANALYZER.Like.audio/Web_Front/wasm_analyzer`) | Path dependency; the same analysis compiled for the browser (`db` is excluded on `wasm32`) |
| The worker `Worker-SAMPLE_ANALYZER` (`SRC/`, `Docker/`; pod `PODS/POD.workers`) | Path dependency; calls `analyze::analyze` on one file posted as a job on the bus and answers with its `.PEAK` record |

### Build-time inputs (`build.rs`)
Two sibling repositories feed the build:

* `CAPTURE.Extractor-Engine` — the shared region-cutting DSP (`extractor_engine` crate), pinned **by commit** (`git = …, rev = …` in `Cargo.toml`). `regions.rs` delegates to it so stored `.PEAK` regions and the web Extractor run one implementation, and its `PeakRegion` is the one definition of a `.PEAK` region. In the monorepo a locally excluded `.cargo/config.toml` (and `SRC/.cargo/config.toml`) patches the URL back to `../../CAPTURE/CAPTURE.Extractor-Engine`, so local edits compile in without a push.
* `DATATYPE.soundfx.UCS` `categories/*.json` and `producer_synonyms.json` — the UCS category signatures and the producer-vocabulary overlay, read from `$APK_UCS_DIR` when set (a standalone clone sets it to a UCS checkout), else from the sibling `../../DATATYPE/DATATYPE.soundfx.UCS`. `build.rs` splices them (minified) into `ucs_signed.json` and `ucs_roles.json` in `OUT_DIR`, embedded with `include_str!`. Files with `"matchable": false` (MUSICPROD) go to the roles bundle, not the matcher. An overlay id that matches no UCS `category_id` fails the build.

`build.rs` also stamps `ANALYZER_HASH` — `ANALYZER_VERSION` in `src/Core/version.rs` — a 12-hex-digit FNV-1a hash of `src/**/*.rs` and the extractor engine's `src/**/*.rs` (found through its `links` metadata, `DEP_EXTRACTOR_ENGINE_SRC_DIR`) with comments and whitespace stripped, plus the bundled UCS data. A comment or formatting edit keeps the version; the same source gives the same version on any machine.

> ⚠️ A code or UCS data change produces a new analyzer version, and every existing `.PEAK` sidecar is then re-analyzed on the next scan. The build prints `warning: … ANALYZER_VERSION changed <old> -> <new>` when that happens, and a scan that re-analyzes such sidecars prints `Note: N .PEAK sidecar(s) were written by analyzer <old>…` and emits a `{"type":"stale"}` event (`stale` is also counted in `done`).

### Source layout
`src/lib.rs` maps each module to its folder with `#[path]`:

| Folder | Modules |
|---|---|
| `src/Core/` | `args` (CLI), `peak` (the record struct), `version` |
| `src/Pipeline/` | `discover` (walk + skipped-extension count), `analyze` (one file → one `Peak`), `preview` (8-bit min/max waveform, base64), `run` (the whole scan) |
| `src/Decoders/` | `decode` (symphonia: WAV/MP3/FLAC/AIFF/OGG/M4A/AAC, lossy sources flagged), `wav` (hound) |
| `src/Scananalyzers/Temporal/` | amplitude, envelope (measured ADSR), regions, sustain, tempo, transients |
| `src/Scananalyzers/Spectral/` | spectrum, stft, flux, mfcc, framestats, partials (inharmonicity), distortion (THD / clipping) |
| `src/Scananalyzers/Musical/` | pitch, root, note_map, acid (ACID loop chunk), timbre, morphology, moments, advanced_stats, vad |
| `src/Scananalyzers/Sequence/syntax/` | bioacoustic syntax: cluster a file's regions into syllable types, transition probabilities, junctions |
| `src/NameSorting/` | normalize, categorize, label, family, `ucs` (the UCS classifier), `music_names.json` |
| `src/Tagging/` | `tags` (acoustic types, sound-design roles) |
| `src/Clustering/` | feature_vec, sqdist, kmeans, cluster, pca |
| `src/Encoders/` | `emit` / `stream` (stdout JSON lines), `sidecar`, `manifest`, `db` (HTTP upload) |

---

### As a worker (`Worker-SAMPLE_ANALYZER`, POD.workers)

The analyzer answers jobs on the bus (LOOSE-workers-convert-rest). A client subscribes to
`APK.audio/System/Protocols/sample_analyzer/incoming/result`, then publishes
`{"job_id": "<yours>", "filename": "kick.wav", "content_b64": "<the file's bytes>"}` (optional
`folder`, and `max_len` in seconds to lower the full-analysis cap) to
`APK.audio/System/Protocols/sample_analyzer/outgoing/job`. The answer carries the same `job_id`,
`ok`, and the file's `.PEAK` record under `result` — the record a batch scan writes beside the
file, with `metadata.path` = `folder/filename`. The file is staged in the container's `/tmp`
tmpfs for the one job (the decoder is chosen by extension, and the ACID chunk is read from the
file) and removed before the answer goes out. Limits are in `SRC/config.ini`: 32 MiB of audio,
600 s before a record is `preview_only`. Nothing is retained; the worker opens no port and does
not upload (`--db-url` is the CLI's). Measured 2026-10-30: a 1 s 441 Hz WAV and the same file as
FLAC both came back `ok: true`, `analysis_depth: full`, `pitch_hz: 441`.

```bash
docker compose -f PODS/POD.workers/Docker/docker-compose.yml up -d --build sample-analyzer
cargo test --manifest-path SRC/Cargo.toml        # handle_job on an in-memory 440 Hz WAV, the self test, the result against the .PEAK schema
SRC/probe_job.sh                                 # live: one probe job to the running worker, PASS on its answer
```

The worker links `apk-plugin-runner` and `apkaudio-contracts` from APK.plugin.SDK by tag (`sdk-v0.5`), like the THICK plugins; the image patches them (and the engine) back to the checkout's copies with `--config`, as the broker image does for SQL.

## 🚀 Usage & Integration
* **Web Portal Entry**: none of its own — `display.json` registers it as a `rust` execution engine (`autostart: false`); it is reached through the Scanalyzer app (`SCANALYZER_SUITE/SCANALYZER.Like.audio`)
* **Remote Repository**: https://github.com/LikeDotAudio/APK.softapp.PROCESSOR.SampleAnalyzer.git

### Build and test
```bash
cargo build --release          # → target/release/oa_sample_analyzer
./check.sh                     # both crates' tests (analyzer + worker, incl. the .PEAK schema contract)
./check.sh --live              # ... plus SRC/probe_job.sh against the running worker
```

### The `.PEAK` record is a published format
[`schema/peak.schema.json`](schema/peak.schema.json) (`analyzer-peak` 1.1.0, strict: `additionalProperties: false` at every level) is the contract every reader holds — the Scanalyzer front end, the corpus API, clients of the worker's `incoming/result`. It moved here from Scanalyzer's `Web_Front/src/analyzerPeakSchema.json` (2026-10-30): the format's owner keeps its schema. Every record declares itself: `metadata.format` = `analyzer-peak`, `metadata.schema_version` = the shape's semver (`PEAK_FORMAT` / `PEAK_SCHEMA_VERSION` in `src/Core/schema.rs`), which moves when a field is added, renamed or dropped. `metadata.analyzer_version` stays what it was: "does this record need re-analysis". `oa_sample_analyzer::schema::{PEAK_SCHEMA, validate}` hand the schema and a checker to any crate that links this one; tests hold an analysed record and the worker's result to it.

### CLI
```
oa_sample_analyzer <dir> [--out <path>] [--workers <n>] [--max-len <s>] [--clusters <k>]
                         [--no-per-file] [--force] [--stride <n>] [--db-url <x>]
```

| Flag | Default | Effect |
|---|---|---|
| `--out` | `<dir>/sample_cloud_data.PEAK` | Aggregate output file |
| `--workers` | 30 | Rayon worker threads |
| `--max-len` | 600 s | Longer files are skipped |
| `--clusters` | 8 | k for the blind K-means grouping |
| `--no-per-file` | off | Do not write per-file sidecars |
| `--force` | off | Re-analyze even when a sidecar's analyzer version matches |
| `--stride` | 1 | Analyze only every Nth file |
| `--db-url` | none | Upload trimmed records, 250 per POST, to this `upload_peak.php` endpoint — trimmed to the fields the endpoint itself lists (`GET upload_peak.php?columns`, its `upload_columns.json`), with `X-Upload-Token: $APK_SCAN_UPLOAD_TOKEN` when that is set — a URL ending in `.php` is used as is, any other http(s) URL gets `api/upload_peak.php` appended, anything else is refused (`endpoint` in `src/Encoders/db.rs`). The desktop app passes `https://scanalyzer.like.audio/api/upload_peak.php` unless `SCAN_DB_URL` overrides it. |

No directory → usage on stderr, exit 2. Unknown flags are ignored.

### What a run writes
* **stdout** — one JSON object per line: `start` (total, workers, skipped extensions, analyzer version), `thread_start`, `result` (the whole `Peak` plus done/total/thread) or `skip`, `clusters`, `per_file`, `db_export`, `manifest`, `done`.
* **`<file>.PEAK`** beside every audio file — the full record (`metadata`, `classification`, `envelope`, `spectral_features`, `musicality`, `unsupervised` (cluster + PCA), `ucs`, `regions`, `preview`, optional `bioacoustic_syntax`). Written as each file finishes, rewritten after clustering.
* **`sample_cloud_data.PEAK`** — the aggregate array.
* **`sample_cloud_manifest.json`** at the library root — a slim index for fast loading; a rebuildable cache, never canonical.

### Examples (offline re-scoring from existing sidecars)
```bash
cargo run --release --example ucs_rescore   -- <dir> [--top N] [--csv out.csv]
cargo run --release --example ucs_scorecard -- <music-dir> <sfx-dir>
```

---

## 🏛 Philosophy

* **The sidecar is the record; everything else is a cache.** Each `.PEAK` sits beside its audio and is canonical; the slim manifest is "a rebuildable cache … nothing's correctness ever depends on it" (`src/Encoders/manifest.rs`). Sidecars are written as each file finishes so an interrupted scan keeps its work (`src/Pipeline/run.rs`).
* **Re-analyze only when the analyzer changed.** A sidecar is reused when its `analyzer_version` equals this build's and it names the same file (`src/Encoders/sidecar.rs`); the version is a hash of the analyzer's code and data, comments excluded (`build.rs`, `src/Core/version.rs`).
* **One bad file never stops a scan.** Each file's analysis runs under `catch_unwind`, a panic becomes a `skip` line, and the panic hook is silenced (`src/main.rs`, `src/Pipeline/run.rs`). The manifest and DB upload are best-effort and never fail the scan.
* **Report what was ignored, never drop silently.** The walk counts every non-audio extension it skipped and reports it in the `start` line (`src/Pipeline/discover.rs`); the DB step reports what the server *stored*, not what was sent (`src/Pipeline/run.rs`, `db_export`).
* **Refusal over fabrication.** A weak or too-close UCS win falls back to the winning category's MISC (abstention) rather than commit to a guess, and the scorecard counts abstentions separately from hits (`src/NameSorting/ucs.rs`, `examples/ucs_scorecard.rs`); provenance is never auto-asserted (`ucs::tests::provenance_is_never_auto_asserted`).
* **One implementation, no drift.** Region detection is the shared `extractor_engine` crate, not a copy (`src/Scananalyzers/Temporal/regions.rs`); UCS data is spliced from `DATATYPE.soundfx.UCS` at build time instead of a vendored copy (`build.rs`); the stdout `result` line serializes the whole `Peak`, so stream and sidecar cannot disagree (`src/Encoders/stream.rs`).
* **Full English keys.** Record and stream keys are full words — `length_seconds`, `envelope_attack_seconds` — no abbreviations (`src/Core/peak.rs`, `src/Encoders/stream.rs`).
* **One purpose per module.** Each extractor is its own file, grouped by domain folder (`src/lib.rs`).

---

The wiki — [start here](Documentation/README.md): [PRINCIPLES](Documentation/%F0%9F%8F%9BPhilosophy/PRINCIPLES.md), [ARCHITECTURE](Documentation/%F0%9F%A7%A9Manual%20and%20Artifacts/ARCHITECTURE.md), [DELIVERY](Documentation/%F0%9F%8C%8DPublishing/DELIVERY.md).

---

## 🛣 Plans
Plans for this app live only in the monorepo roadmap: `Documentation/🛣Roadmap/roadmap.htm`.

<!-- wiki:smoke -->
**Smoke test** — ✅ PASS 1/1 since 2026-10-30 12:51 EDT — [results](Documentation/%F0%9F%A7%A9Manual%20and%20Artifacts/SMOKE.md)
<!-- /wiki:smoke -->
