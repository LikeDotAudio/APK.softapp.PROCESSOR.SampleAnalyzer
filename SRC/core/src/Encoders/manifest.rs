//! The slim library manifest — a compact index over the full `.PEAK` sidecars.
//!
//! One record has three audiences at wildly different weights: the aggregate views
//! (Examiner list, 2D Stats, 3D Cloud, grouping) read only a handful of light scalars,
//! while the Examiner *detail* panel needs the whole thing (regions, MFCC, chromagram,
//! PCA, spectral extras). The manifest carries only the light set — about an eighth of a
//! record's bytes — so the app can load a huge library fast and lazy-load a full record
//! only when a file is opened.
//!
//! The per-file `.PEAK` sidecars remain the canonical record. The manifest is a
//! rebuildable *cache*: if it is missing, stale, or corrupt, the reader falls back to the
//! sidecars and rebuilds it. Nothing's correctness ever depends on it.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::version::ANALYZER_VERSION;

/// The manifest filename, written at the scanned-directory root beside the audio.
/// Deliberately NOT `manifest.json` — the demo pack already uses that name for its
/// audio file-list; this parallels the aggregate `sample_cloud_data.PEAK`.
pub const MANIFEST_NAME: &str = "sample_cloud_manifest.json";
/// Bump when the slim-row shape changes so a reader can reject an old layout.
pub const MANIFEST_VERSION: u32 = 1;

/// Copy `fields` from `src[section]` into `dst[section]`, keeping only those present.
fn pick(src: &Value, dst: &mut Map<String, Value>, section: &str, fields: &[&str]) {
    let Some(obj) = src.get(section).and_then(|v| v.as_object()) else { return };
    let mut out = Map::new();
    for f in fields {
        if let Some(v) = obj.get(*f) {
            out.insert((*f).to_string(), v.clone());
        }
    }
    if !out.is_empty() {
        dst.insert(section.to_string(), Value::Object(out));
    }
}

/// Project one full record `Value` down to a slim manifest row. Works on any grouped
/// `.PEAK` object — a freshly-analyzed `Peak` serialized to `Value`, or an existing
/// sidecar read off disk — so scan-time and reindex-from-sidecars share one field list.
///
/// The row keeps the grouped shape (`metadata`, `ucs`, …) so the front-end's
/// `normalizePeakRecords` ingests it unchanged: a manifest row is just a `.PEAK` with the
/// heavy sections omitted, and every view already optional-accesses its fields.
pub fn project_slim(full: &Value) -> Value {
    let mut row = Map::new();

    pick(full, &mut row, "metadata",
         &["name", "path", "folder", "length_seconds", "analyzer_version", "source_format"]);
    pick(full, &mut row, "classification",
         &["group", "subgroup", "timbre", "length_class", "music_production_category"]);
    pick(full, &mut row, "envelope",
         &["transient_count", "attack_seconds", "envelope_sustain_level"]);
    pick(full, &mut row, "spectral_features",
         &["spectral_centroid_hz", "harmonicity", "root_mean_square_level",
           "zero_crossings_per_second", "crest_factor", "spectral_flatness", "complexity"]);
    pick(full, &mut row, "musicality",
         &["pitch_hz", "beats_per_minute", "root_note_name"]);
    pick(full, &mut row, "unsupervised", &["cluster"]);

    // classification.reason — keep only the first line (the Examiner "Reason" column
    // reads reason[0]); the rest is detail-panel weight.
    if let Some(first) = full.get("classification")
        .and_then(|c| c.get("reason"))
        .and_then(|r| r.as_array())
        .and_then(|a| a.first())
    {
        row.entry("classification".to_string())
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .unwrap()
            .insert("reason".to_string(), Value::Array(vec![first.clone()]));
    }

    // ucs — scalars plus the top-3 alternatives slimmed to {category, subcategory,
    // probability}. Drops synonyms / id / reason (detail-panel only).
    if let Some(ucs) = full.get("ucs").and_then(|v| v.as_object()) {
        let mut u = Map::new();
        for f in ["category", "subcategory", "id", "confidence"] {
            if let Some(v) = ucs.get(f) { u.insert(f.to_string(), v.clone()); }
        }
        if let Some(alts) = ucs.get("alternatives").and_then(|v| v.as_array()) {
            let slim_alts: Vec<Value> = alts.iter().take(3).map(|a| {
                match a.as_object() {
                    Some(o) => {
                        let mut m = Map::new();
                        for f in ["category", "subcategory", "probability"] {
                            if let Some(v) = o.get(f) { m.insert(f.to_string(), v.clone()); }
                        }
                        Value::Object(m)
                    }
                    // Legacy packed-string alternative — keep as-is.
                    None => a.clone(),
                }
            }).collect();
            u.insert("alternatives".to_string(), Value::Array(slim_alts));
        }
        if !u.is_empty() { row.insert("ucs".to_string(), Value::Object(u)); }
    }

    // regions — the count only. The nested per-region analysis is the single heaviest
    // payload in a record and never appears in an aggregate view.
    if let Some(count) = full.get("regions").and_then(|r| r.get("count")) {
        let mut r = Map::new();
        r.insert("count".to_string(), count.clone());
        row.insert("regions".to_string(), Value::Object(r));
    }

    Value::Object(row)
}

/// Build the full manifest object: a small header (version, engine, totals, a
/// per-category histogram for an instant group tree) plus the slim rows.
pub fn build_manifest(root: &Path, records: &[Value]) -> Value {
    let rows: Vec<Value> = records.iter().map(project_slim).collect();

    // Per-UCS-category counts — the project-level index. A few hundred bytes that give
    // the scope bar / Groups tab their totals without touching a single row.
    let mut categories: Map<String, Value> = Map::new();
    for row in &rows {
        let cat = row.get("ucs")
            .and_then(|u| u.get("category"))
            .and_then(|c| c.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("UNCLASSIFIED");
        let n = categories.get(cat).and_then(|v| v.as_u64()).unwrap_or(0) + 1;
        categories.insert(cat.to_string(), Value::from(n));
    }

    let generated_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    serde_json::json!({
        "manifest_version": MANIFEST_VERSION,
        "analyzer_version": ANALYZER_VERSION,
        "root": root.to_string_lossy(),
        "total_files": rows.len(),
        "generated_unix": generated_unix,
        "categories": Value::Object(categories),
        "records": rows,
    })
}

/// Write `<root>/manifest.json`. Returns true on success. Best-effort and additive —
/// a failure here never fails a scan; the sidecars are already on disk.
pub fn write_manifest(root: &Path, records: &[Value]) -> bool {
    let manifest = build_manifest(root, records);
    let path = root.join(MANIFEST_NAME);
    // Compact, not pretty: this file is machine-read and can reach tens of MB.
    serde_json::to_string(&manifest)
        .ok()
        .and_then(|js| std::fs::write(&path, js).ok())
        .is_some()
}

/// Is this manifest object one THIS engine wrote? A slim row measured by different
/// extractor code is not interchangeable, so a version mismatch means "ignore the
/// cache and fall back to the sidecars." Mirrors `sidecar::read_sidecar`'s test.
pub fn is_current(manifest: &Value) -> bool {
    manifest.get("analyzer_version").and_then(|v| v.as_str()) == Some(ANALYZER_VERSION)
        && manifest.get("manifest_version").and_then(|v| v.as_u64()) == Some(MANIFEST_VERSION as u64)
}
