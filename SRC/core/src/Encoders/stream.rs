//! The streamed stdout protocol: one JSON line per analyzed (or skipped) file,
//! consumed live by the GUI. Keys match the .PEAK data model exactly — full
//! English, no abbreviations.
use crate::emit::emit;
use crate::peak::Peak;

/// Emit the "result" line for one successfully analyzed file.
///
/// This serializes the WHOLE record rather than hand-listing fields. The old
/// hand-written list had silently gone stale: it never carried `ucs_category`,
/// `ucs_subcategory`, `source_format` or `lossy_source`, so those fields existed
/// in every .PEAK on disk but were absent from the live records the GUI builds
/// from this stream — the cloud had nothing to colour a UCS category by, and the
/// inspector had nothing to show. Serializing the struct means the stream and
/// the .PEAK can never disagree again.
pub fn emit_result(p: &Peak, done: usize, total: usize, tid: usize) {
    let mut v = serde_json::to_value(p).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(o) = v.as_object_mut() {
        // Nothing is withheld any more. The one field that used to be — the raw
        // onset envelope, thousands of floats nobody read — is now reduced to
        // `onset_periodicity` before it ever reaches the record, so the stream
        // and the .PEAK carry exactly the same thing.
        o.insert("type".into(), serde_json::json!("result"));
        o.insert("done".into(), serde_json::json!(done));
        o.insert("total".into(), serde_json::json!(total));
        o.insert("thread_id".into(), serde_json::json!(tid));
    }
    emit(&v);
}

/// Emit the "skip" line for a file that could not be analyzed.
pub fn emit_skip(name: &str, done: usize, total: usize, tid: usize) {
    emit(&serde_json::json!({ "type": "skip", "done": done, "total": total, "name": name, "thread_id": tid }));
}
