# PROCESSOR.SampleAnalyzer

> ⚙️ **Sample Analyzer** — the Rust DSP core of the Scanalyzer suite. It walks an audio library, analyzes every file in parallel, classifies it against the UCS sound taxonomy, and writes a `.PEAK` JSON record beside each file plus one aggregate for the library.

---

## 🧩 Architectural Role
* **Family**: PROCESSOR (data processing and transformations)
* **Category**: Scanalyzer / DSP Core (`display.json` `group` / `subGroup`)
* **Pipeline Stage**: analysis — audio files in, `.PEAK` feature records out; feeds the Scanalyzer views (3D cloud, Examiner, Groups) and the shared cloud database

---

## ⚙️ Repository Overview
A standalone Git repository, mounted as a submodule at `PODS/POD.THIN/Softapps/PROCESSOR/PROCESSOR.SampleAnalyzer` in the APK.audio monorepo. It is one crate, `oa-sample-analyzer`, used three ways:

| Consumer | How |
|---|---|
| The `oa_sample_analyzer` CLI (`src/main.rs`) | Batch scan of a directory |
| Scanalyzer desktop app (`SCANALYZER_SUITE/SCANALYZER.Like.audio/Web_Front/src-tauri`) | Path dependency; calls `analyze::analyze_buffer`, `decode::read_audio`, `db::write_peaks` in-process |
| Scanalyzer WASM engine (`SCANALYZER_SUITE/SCANALYZER.Like.audio/Web_Front/wasm_analyzer`) | Path dependency; the same analysis compiled for the browser (`db` is excluded on `wasm32`) |

### Build-time inputs (`build.rs`)
The crate does not build on its own — it needs two sibling repos in the monorepo:

* `../../CAPTURE/CAPTURE.Extractor-Engine` — the shared region-cutting DSP (`extractor_engine` crate). `regions.rs` delegates to it so stored `.PEAK` regions and the web Extractor run one implementation.
* `../../DATATYPE/DATATYPE.soundfx.UCS/categories/*.json` and `producer_synonyms.json` — the UCS category signatures and the producer-vocabulary overlay. `build.rs` splices them (minified) into `ucs_signed.json` and `ucs_roles.json` in `OUT_DIR`, embedded with `include_str!`. Files with `"matchable": false` (MUSICPROD) go to the roles bundle, not the matcher. An overlay id that matches no UCS `category_id` fails the build.

`build.rs` also stamps `ANALYZER_HASH` — `ANALYZER_VERSION` in `src/Core/version.rs` — a 12-hex-digit FNV-1a hash of `src/**/*.rs` and `../../CAPTURE/CAPTURE.Extractor-Engine/src/**/*.rs` with comments and whitespace stripped, plus the bundled UCS data. A comment or formatting edit keeps the version; the same source gives the same version on any machine.

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

## 🚀 Usage & Integration
* **Web Portal Entry**: none of its own — `display.json` registers it as a `rust` execution engine (`autostart: false`); it is reached through the Scanalyzer app (`SCANALYZER_SUITE/SCANALYZER.Like.audio`)
* **Remote Repository**: https://github.com/LikeDotAudio/APK.softapp.PROCESSOR.SampleAnalyzer.git

### Build and test
```bash
cargo build --release          # → target/release/oa_sample_analyzer
cargo test --release --lib     # unit tests (79 at the time of writing)
```

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
| `--db-url` | none | Upload trimmed records, 250 per POST, to this `upload_peak.php` endpoint — a URL ending in `.php` is used as is, any other http(s) URL gets `api/upload_peak.php` appended, anything else is refused (`endpoint` in `src/Encoders/db.rs`). The desktop app passes `https://scanalyzer.like.audio/api/upload_peak.php` unless `SCAN_DB_URL` overrides it. |

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
