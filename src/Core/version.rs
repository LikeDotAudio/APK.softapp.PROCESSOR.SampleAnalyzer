//! The analyzer version written into every record: the build timestamp to the
//! minute, stamped by build.rs at compile time.
//!
//! It answers exactly one question — "was this record produced by the current
//! analyzer, or does it need re-analysis?" — and every consumer compares it for
//! equality only. Nothing parses it, so the format is free to be short, and it
//! is: this string is stored once per record in every `.PEAK` sidecar and once
//! per row in the cloud database.
//!
//! build.rs re-runs only when `src/` or the UCS data changes, so the stamp is
//! the minute the analyzer's inputs last changed — not the minute of every
//! incremental build. A source hash used to be appended to make the version
//! reproducible across machines, but the timestamp in front of it already made
//! the whole string differ between two builds of identical source, so the hash
//! never delivered that property and was 17 bytes per record of nothing.
pub const ANALYZER_VERSION: &str = env!("ANALYZER_DATE");
