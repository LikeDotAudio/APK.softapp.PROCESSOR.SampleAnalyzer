//! The published `.PEAK` record contract: `schema/peak.schema.json`, its version, and
//! a small checker for the subset of JSON Schema that file uses.
//!
//! `.PEAK` is read outside this crate — the Scanalyzer front end, the corpus API, and
//! every client of Worker-SAMPLE_ANALYZER's `incoming/result` — so it is a published
//! format, not an internal struct. The schema file is the contract; `peak.rs` is the
//! implementation, and the tests below hold the two together (an analysed record must
//! validate, and the version stamped into `metadata.schema_version` must be the
//! schema's `x-schema-version`).
//!
//! VERSIONING. `PEAK_SCHEMA_VERSION` moves only when a field is renamed or removed or
//! changes type — a breaking change for a reader. Adding a field does not move it
//! (every reader already ignores fields it does not know). `analyzer_version` is a
//! different number with a different job: "does this record need re-analysis".
//!
//! The checker exists so the contract test needs no JSON-Schema crate; it understands
//! `type` (one or a list), `required`, `properties`, `items` and `minimum`, which is
//! everything `peak.schema.json` uses. Keywords it does not know are ignored.
use serde_json::Value;

/// The `.PEAK` schema version this analyzer writes (`metadata.schema_version`).
/// Must equal `x-schema-version` in `schema/peak.schema.json`.
pub const PEAK_SCHEMA_VERSION: u32 = 1;

/// `schema/peak.schema.json`, compiled in so any consumer of this crate (the WASM
/// engine, the worker, the Tauri app) can hand the contract on without a file path.
pub const PEAK_SCHEMA: &str = include_str!("../../schema/peak.schema.json");

/// Check `record` against [`PEAK_SCHEMA`]. `Err` lists every violation as
/// `<json pointer>: <what is wrong>`.
pub fn validate(record: &Value) -> Result<(), Vec<String>> {
    let schema: Value = serde_json::from_str(PEAK_SCHEMA).expect("peak.schema.json is valid JSON");
    validate_against(&schema, record)
}

/// Check `value` against an arbitrary `schema` (same subset as [`validate`]).
pub fn validate_against(schema: &Value, value: &Value) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    check(schema, value, "", &mut errors);
    if errors.is_empty() { Ok(()) } else { Err(errors) }
}

fn type_matches(name: &str, v: &Value) -> bool {
    match name {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        "number" => v.is_number(),
        "integer" => v.is_i64() || v.is_u64(),
        _ => true,
    }
}

fn check(schema: &Value, v: &Value, at: &str, errors: &mut Vec<String>) {
    let here = if at.is_empty() { "/" } else { at };
    if let Some(t) = schema.get("type") {
        let names: Vec<&str> = match t {
            Value::String(s) => vec![s.as_str()],
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        if !names.is_empty() && !names.iter().any(|n| type_matches(n, v)) {
            errors.push(format!("{here}: expected {}, found {}", names.join(" or "), kind(v)));
            return;
        }
    }
    if let (Some(min), Some(x)) = (schema.get("minimum").and_then(Value::as_f64), v.as_f64()) {
        if x < min {
            errors.push(format!("{here}: {x} is below the minimum {min}"));
        }
    }
    if let Some(obj) = v.as_object() {
        for key in schema.get("required").and_then(Value::as_array).into_iter().flatten() {
            if let Some(k) = key.as_str() {
                if !obj.contains_key(k) {
                    errors.push(format!("{here}: missing required `{k}`"));
                }
            }
        }
        if let Some(props) = schema.get("properties").and_then(Value::as_object) {
            for (k, sub) in props {
                if let Some(child) = obj.get(k) {
                    check(sub, child, &format!("{at}/{k}"), errors);
                }
            }
        }
    }
    if let (Some(items), Some(arr)) = (schema.get("items"), v.as_array()) {
        for (i, child) in arr.iter().enumerate() {
            check(items, child, &format!("{at}/{i}"), errors);
        }
    }
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_constant_is_the_schema_files_version() {
        let schema: Value = serde_json::from_str(PEAK_SCHEMA).unwrap();
        assert_eq!(schema["x-schema-version"].as_u64(), Some(PEAK_SCHEMA_VERSION as u64));
    }

    #[test]
    fn a_wrong_type_and_a_missing_field_are_both_reported() {
        let schema: Value = serde_json::from_str(PEAK_SCHEMA).unwrap();
        let bad = serde_json::json!({"metadata": {"name": 3}});
        let errors = validate_against(&schema, &bad).unwrap_err();
        assert!(errors.iter().any(|e| e.contains("missing required `classification`")), "{errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("/metadata/name: expected string")), "{errors:?}");
    }

    /// The contract test: what the analyzer actually writes for a real file — two
    /// bursts, so `regions` carries two PeakRegions with their own analysis —
    /// validates against the published schema and carries the schema's version.
    #[test]
    fn an_analysed_record_satisfies_the_published_schema() {
        let dir = std::env::temp_dir().join(format!("peak-schema-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("two_bursts.wav");
        let spec = hound::WavSpec { channels: 1, sample_rate: 44_100, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for n in 0..(44_100 * 3 / 2) {
            let t = n as f64 / 44_100.0;
            let on = t < 0.4 || (0.9..1.3).contains(&t);
            let v = if on { (2.0 * std::f64::consts::PI * 220.0 * t).sin() * 12_000.0 } else { 0.0 };
            w.write_sample(v as i16).unwrap();
        }
        w.finalize().unwrap();
        let peak = crate::analyze::analyze(&path, &dir, 600.0).expect("the probe decodes");
        let _ = std::fs::remove_dir_all(&dir);
        let record = serde_json::to_value(&peak).unwrap();
        assert_eq!(record["metadata"]["schema_version"].as_u64(), Some(PEAK_SCHEMA_VERSION as u64));
        if let Err(errors) = validate(&record) {
            panic!("the analyzer's own record breaks peak.schema.json: {errors:#?}");
        }
    }
}
