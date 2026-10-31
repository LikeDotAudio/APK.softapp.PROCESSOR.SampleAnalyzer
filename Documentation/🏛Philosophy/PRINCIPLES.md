# Principles — PROCESSOR.SampleAnalyzer

The rules the analyzer keeps, each with the code that keeps it (paths under `src/` unless named). Ecosystem rules are cited, not copied: [Refusal over fabrication](../../../../../../../Documentation/%F0%9F%8F%9BPhilosophy/Doctrine/readme.Refusal%20over%20fabrication.md), [Data handoff](../../../../../../../Documentation/%F0%9F%8F%9BPhilosophy/DATA%20HANDOFF%20PHILOSOPHY.md), [Units](../../../../../../../Documentation/%F0%9F%8F%9BPhilosophy/Units).

← [Wiki home](../README.md)

| Principle | Kept by | Where it breaks |
|---|---|---|
| **The sidecar is the record; everything else is a cache.** Each `.PEAK` sits beside its audio and is canonical; the slim manifest is a rebuildable cache. Sidecars are written as each file finishes, so an interrupted scan keeps its work. | `Encoders/manifest.rs` header (line 11); `write_sidecar` in `Pipeline/run.rs` (line 102) | — |
| **Re-analyze only when the analyzer changed.** A sidecar is reused when its `analyzer_version` equals this build's and it names the same file. | `read_sidecar` in `Encoders/sidecar.rs`; `ANALYZER_VERSION` in `Core/version.rs`; `strip_rust` + `Fnv` in `build.rs` | The version is a hash of the code with comments and whitespace stripped, so a comment edit keeps it. A real change re-analyzes every sidecar; the build warns and the scan reports the `stale` count. Cargo.toml dependency bumps are not hashed. |
| **One bad file never stops a scan.** Each file runs under `catch_unwind`; a panic becomes a `skip` line and the panic hook is silenced. Manifest and DB upload are best-effort. | `Pipeline/run.rs` line 90; `main.rs` line 3 | — |
| **Report what was ignored, never drop silently.** The walk counts every skipped extension and the `start` line reports it; the DB step reports what the server *stored*, not what was sent. | `discover_audio` in `Pipeline/discover.rs` (line 17); `db_export` in `Pipeline/run.rs` (lines 156–170); `write_peaks` in `Encoders/db.rs` | — |
| **Refusal over fabrication.** A weak or too-close UCS win falls back to the category's MISC (abstention); provenance categories are never auto-asserted. | `NameSorting/ucs.rs` (stage 5, line 15; fallback near line 565); test `provenance_is_never_auto_asserted` (line 865); `examples/ucs_scorecard.rs` counts abstentions apart | — |
| **One implementation, no drift.** Regions are the shared `extractor_engine` crate; UCS data is spliced from `DATATYPE.soundfx.UCS` at build time and a typo'd overlay id fails the build; the stdout `result` line serializes the whole `Peak`. | `Scananalyzers/Temporal/regions.rs` (line 8); `build.rs`; `emit_result` in `Encoders/stream.rs` (line 16) | — |
| **Say where it goes.** `--db-url` is the upload endpoint; a non-http value is refused rather than redirected. | `endpoint` in `Encoders/db.rs`; test `db_url_names_the_endpoint` | — |
| **Full English keys.** Record and stream keys are whole words (`length_seconds`, `envelope_attack_seconds`). | `Core/peak.rs`, `Encoders/stream.rs` | — |
| **The record is a published format.** `.PEAK` has one strict schema file, owned here, and every record declares `format` + `schema_version`; the worker's bus result and an analysed record are tested against it. | `schema/peak.schema.json`; `Core/schema.rs` | — |
| **Siblings by version, not by folder.** Extractor-Engine by `rev`, the SDK by tag; local `[patch]` files make the monorepo build the local copies. | `Cargo.toml`, `SRC/Cargo.toml` | — |
| **The worker's job session is its own** (kept by design, audit 2026-10-30): an analysis takes seconds and must not hold up the runner's config, will or self-test door, so the job loop opens a second MQTT session instead of riding the runner's. | `run_job_agent` in `SRC/src/lib.rs` | — |
| **One purpose per module.** Each extractor is its own file, grouped by domain folder and mapped with `#[path]`. | `lib.rs` | — |
