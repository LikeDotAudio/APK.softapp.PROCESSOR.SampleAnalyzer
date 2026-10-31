# Architecture — PROCESSOR.SampleAnalyzer

← [Wiki home](../README.md)

One crate, `oa-sample-analyzer`: a library plus the `oa_sample_analyzer` CLI. The repo
[README](../../README.md) carries the module map, the CLI flags and the record shape; this page
is how the pieces connect.

## Who calls it

| Caller | How |
|---|---|
| `oa_sample_analyzer <dir> …` | `src/main.rs` → `Config::parse` (`src/Core/args.rs`) → `run::run` (`src/Pipeline/run.rs`). No `<dir>` → usage, exit 2. |
| Scanalyzer desktop (`SCANALYZER_SUITE/SCANALYZER.Like.audio/Web_Front/src-tauri`) | Spawns the CLI for a scan (`--db-url` from `cloud_db_url()`), and calls `analyze::analyze_buffer`, `decode::read_audio`, `db::get_pool` / `db::write_peaks` in-process. |
| Scanalyzer browser (`…/Web_Front/wasm_analyzer`) | `wasm-pack` build of this crate; uses `analyze_buffer`, `read_audio_buffer`, `version::ANALYZER_VERSION`. `db` and `ureq` are left out on `wasm32`. |
| `examples/ucs_rescore.rs`, `examples/ucs_scorecard.rs` | Offline UCS re-scoring from existing sidecars. |

## As a worker — `SRC/`

`SRC/src/lib.rs` `handle_job` decodes a job's `content_b64`, refuses one over `max_job_bytes`
(32 MiB), writes the bytes under a name reduced to one safe path element into a directory of the
job's own in the temp dir, runs `analyze::analyze` on it, rewrites `metadata.folder`/`path` to
the job's names, and removes the directory. It always returns
`{job_id, tool, ok, result, error, elapsed_ms}`. `run_job_agent` feeds it from
`APK.audio/System/Protocols/sample_analyzer/outgoing/job` and answers on `…/incoming/result`,
neither retained. The job runs on a thread of its own (`JobLane`), so the MQTT event loop keeps
pinging through a long job and the broker never drops the session mid-job; up to `JOB_QUEUE` (4)
jobs wait behind the running one, and a job past that is answered at once with `ok: false`
(busy) under its own `job_id`. `apk-plugin-runner` adds the retained `incoming/api` description, the config,
the will and the self test (`SRC/src/self_test.rs`: an in-memory 440 Hz WAV must come back fully
analysed within 5 Hz). The pod is `PODS/POD.workers`.

## Build-time inputs — `build.rs`

1. Reads `../../DATATYPE/DATATYPE.soundfx.UCS/categories/*.json` (minus `index.json`) and
   `producer_synonyms.json`, merges the overlay's synonyms into matching `category_id`s (a
   miss fails the build), minifies, and writes `ucs_signed.json` (matchable) and
   `ucs_roles.json` (`"matchable": false`, e.g. MUSICPROD) to `OUT_DIR`. `ucs.rs` embeds them
   with `include_str!`.
2. Stamps `ANALYZER_HASH`, which becomes `ANALYZER_VERSION`: FNV-1a over `src/` and the
   extractor engine's `src/` (comments and whitespace stripped) plus the UCS bundles, 12 hex
   digits. When it differs from the last build's (`OUT_DIR/analyzer_version.txt`) the build
   prints a `cargo:warning`. `rerun-if-changed` covers both `src/` trees and the UCS data.

`Cargo.toml` also path-depends on `../../CAPTURE/CAPTURE.Extractor-Engine`.

## One run — `run::run`

1. **DB.** With `--db-url`, `db::get_pool` resolves the endpoint (`db::endpoint`); a bad
   value prints `DB upload off: …` and the scan goes on without it.
2. **Discover.** `discover_audio` walks the root, returns audio files and a count of every
   skipped extension; `--stride` keeps every Nth file. A `start` line goes to stdout.
3. **Analyze, in parallel.** A Rayon pool of `--workers` threads. Per file: reuse the sidecar
   if `read_sidecar` accepts it (unless `--force`), else decode (`Decoders/`) and
   `analyze::analyze` under `catch_unwind` → one `Peak` (amplitude, envelope, pitch, root,
   tempo, ACID, spectral and frame features, distortion, partials, morphology, regions,
   syntax, UCS, tags, preview). Each result is streamed (`emit_result`) and its sidecar
   written at once; a panic or a too-long file becomes a `skip` line.
4. **Cluster.** `Clustering/` builds feature vectors, runs K-means (`--clusters`) and PCA,
   and writes `unsupervised` back into every record; sidecars are rewritten.
5. **Write.** The aggregate `sample_cloud_data.PEAK` (`--out`), per-file sidecars unless
   `--no-per-file`, `db::write_peaks` (trimmed records, 250 per POST, counts what the server
   says it stored → `db_export` line), `manifest::write_manifest` → `sample_cloud_manifest.json`
   at the root, and `done`.

## Files

| Path | Role |
|---|---|
| `src/Core/` | `args`, `peak` (the record), `version` |
| `src/Pipeline/` | `discover`, `analyze`, `preview`, `run` |
| `src/Decoders/` | symphonia + hound decode, lossy flag |
| `src/Scananalyzers/` | `Temporal/`, `Spectral/`, `Musical/`, `Sequence/syntax/` extractors |
| `src/NameSorting/` | name normalisation, taxonomy, the UCS classifier `ucs.rs` |
| `src/Tagging/`, `src/Clustering/` | tags; feature vectors, K-means, PCA |
| `src/Encoders/` | `emit`/`stream` (stdout), `sidecar`, `manifest`, `db` |
