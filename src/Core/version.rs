//! The analyzer version written into every record: a 12-hex-digit hash of the
//! analyzer's inputs, stamped by build.rs at compile time.
//!
//! It answers exactly one question — "was this record produced by the current
//! analyzer, or does it need re-analysis?" — and every consumer compares it for
//! equality only. Nothing parses it, so the format is free to be short, and it
//! is: this string is stored once per record in every `.PEAK` sidecar and once
//! per row in the cloud database.
//!
//! The hash covers the Rust under `src/` and the extractor engine's `src/` with
//! comments and whitespace stripped, plus the bundled UCS data. A comment or
//! formatting edit therefore keeps it; a code or data change moves it, and the
//! build prints a `cargo:warning` naming the old and new version. Two builds of
//! the same source give the same version on any machine.
pub const ANALYZER_VERSION: &str = env!("ANALYZER_HASH");
