//! The published `.PEAK` record contract: `schema/peak.schema.json`, its version, and
//! a small checker for the subset of JSON Schema that file uses.
//!
//! `.PEAK` is read outside this crate — the Scanalyzer front end, the corpus API, and
//! every client of Worker-SAMPLE_ANALYZER's `incoming/result` — so it is a published
//! format, not an internal struct. The schema file is the contract; `peak.rs` is the
//! implementation, and the tests below hold the two together (an analysed record must
//! validate, and the version stamped into `metadata.schema_version` must be the
//! semver in the schema's `$id`).
//!
//! VERSIONING (the schema's own rule, its `$comment`): `PEAK_SCHEMA_VERSION` is the
//! SHAPE's semver and the semver in the schema's `$id`; it moves when a field is added,
//! renamed or dropped. `analyzer_version` is a different number with a different job:
//! "does this record need re-analysis". Readers refuse an unknown MAJOR and read an
//! unknown MINOR (`identifyPeakDocument` in Scanalyzer's front end). Records written
//! before the declaration existed carry `format`/`schema_version` empty.
//!
//! The checker exists so the contract tests need no JSON-Schema crate; it understands
//! `type` (one or a list), `required`, `properties`, `additionalProperties` (false or a
//! schema), `items`, `enum`, `oneOf` and local `$ref` (`#/$defs/<name>`), which is
//! everything `peak.schema.json` uses. Keywords it does not know are ignored.
use serde_json::Value;

/// What a `.PEAK` record says it is (`metadata.format`).
pub const PEAK_FORMAT: &str = "analyzer-peak";

/// The `.PEAK` shape's semver (`metadata.schema_version`). Must equal the version at the
/// end of `$id` in `schema/peak.schema.json`.
pub const PEAK_SCHEMA_VERSION: &str = "1.1.0";

/// `schema/peak.schema.json`, compiled in so any consumer of this crate (the WASM
/// engine, the worker, the Tauri app) can hand the contract on without a file path.
pub const PEAK_SCHEMA: &str = include_str!("../../schema/peak.schema.json");

/// Check `document` (one record, or an aggregate array of them) against
/// [`PEAK_SCHEMA`]. `Err` lists every violation as `<json pointer>: <what is wrong>`.
pub fn validate(document: &Value) -> Result<(), Vec<String>> {
    let schema: Value = serde_json::from_str(PEAK_SCHEMA).expect("peak.schema.json is valid JSON");
    validate_against(&schema, document)
}

/// Check `value` against an arbitrary `schema` (same subset as [`validate`]); `$ref`s
/// resolve against `schema` itself.
pub fn validate_against(schema: &Value, value: &Value) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    check(schema, schema, value, "", &mut errors);
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
        "integer" => v.is_i64() || v.is_u64() || v.as_f64().is_some_and(|f| f.fract() == 0.0),
        _ => true,
    }
}

fn resolve<'a>(root: &'a Value, reference: &str) -> Option<&'a Value> {
    let path = reference.strip_prefix("#/")?;
    path.split('/').try_fold(root, |node, key| node.get(key))
}

fn check(root: &Value, schema: &Value, v: &Value, at: &str, errors: &mut Vec<String>) {
    let here = if at.is_empty() { "/" } else { at };
    if let Some(r) = schema.get("$ref").and_then(Value::as_str) {
        match resolve(root, r) {
            Some(target) => check(root, target, v, at, errors),
            None => errors.push(format!("{here}: unresolvable $ref {r}")),
        }
    }
    if let Some(options) = schema.get("oneOf").and_then(Value::as_array) {
        // The closest branch is one whose own type fits, then the one with fewest errors.
        let wrong_type = |e: &[String]| e.len() == 1 && e[0].starts_with(&format!("{here}: expected "));
        let mut best: Option<Vec<String>> = None;
        let matching = options
            .iter()
            .filter(|o| {
                let mut e = Vec::new();
                check(root, o, v, at, &mut e);
                let ok = e.is_empty();
                let better = |b: &Vec<String>| (wrong_type(&e), e.len()) < (wrong_type(b), b.len());
                if !ok && best.as_ref().is_none_or(better) {
                    best = Some(e);
                }
                ok
            })
            .count();
        if matching != 1 {
            if matching == 0 {
                // The closest branch's errors say more than "matched none".
                errors.extend(best.unwrap_or_default());
            } else {
                errors.push(format!("{here}: matches {matching} oneOf branches, not exactly one"));
            }
        }
    }
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
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array) {
        if !allowed.contains(v) {
            errors.push(format!("{here}: {v} is not one of {}", Value::Array(allowed.clone())));
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
        let props = schema.get("properties").and_then(Value::as_object);
        for (k, child) in obj {
            match props.and_then(|p| p.get(k)) {
                Some(sub) => check(root, sub, child, &format!("{at}/{k}"), errors),
                None => match schema.get("additionalProperties") {
                    Some(Value::Bool(false)) => errors.push(format!("{here}: `{k}` is not in the schema")),
                    Some(extra @ Value::Object(_)) => check(root, extra, child, &format!("{at}/{k}"), errors),
                    _ => {}
                },
            }
        }
    }
    if let (Some(items), Some(arr)) = (schema.get("items"), v.as_array()) {
        for (i, child) in arr.iter().enumerate() {
            check(root, items, child, &format!("{at}/{i}"), errors);
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
        let id = schema["$id"].as_str().unwrap();
        assert!(id.ends_with(&format!("/{PEAK_SCHEMA_VERSION}")), "$id {id} vs {PEAK_SCHEMA_VERSION}");
        let meta = &schema["$defs"]["record"]["properties"]["metadata"]["properties"];
        assert!(meta["schema_version"]["enum"].as_array().unwrap().contains(&Value::from(PEAK_SCHEMA_VERSION)));
        assert!(meta["format"]["enum"].as_array().unwrap().contains(&Value::from(PEAK_FORMAT)));
    }

    #[test]
    fn a_wrong_type_and_a_missing_field_are_both_reported() {
        let schema: Value = serde_json::from_str(PEAK_SCHEMA).unwrap();
        let bad = serde_json::json!({"metadata": {"name": 3, "surprise": true}});
        let errors = validate_against(&schema, &bad).unwrap_err();
        assert!(errors.iter().any(|e| e.contains("missing required `classification`")), "{errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("/metadata/name: expected string")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("`surprise` is not in the schema")), "{errors:?}");
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
        assert_eq!(record["metadata"]["schema_version"], PEAK_SCHEMA_VERSION);
        assert_eq!(record["metadata"]["format"], PEAK_FORMAT);
        if let Err(errors) = validate(&record) {
            panic!("the analyzer's own record breaks peak.schema.json: {errors:#?}");
        }
    }
}
