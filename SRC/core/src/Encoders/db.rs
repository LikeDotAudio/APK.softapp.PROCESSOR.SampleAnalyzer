//! Contributing records to the shared cloud database over HTTP.
//!
//! Not a database client despite the name — `Pool` is a vestige of the direct
//! MySQL connection this replaced, kept so the Tauri command's call sites did
//! not have to change. Everything here POSTs JSON to `upload_peak.php`, at the
//! endpoint `--db-url` names (see `endpoint`): the public host, the bench's own
//! scan-api, or a test server. It used to ignore the value and always post to
//! the public host.
//!
//! Records are trimmed to the columns the endpoint stores before being sent.
//! They used to go up whole, which was two bugs at once: a 500-record chunk of
//! full records is ~5.7 MB and exceeds `post_max_size` on typical shared hosting
//! (PHP then discards the body and the script sees an empty payload), and the
//! `Vec<String>` fields arrived as JSON arrays that PDO stringified into the
//! literal text "Array" — 1,000 rows in the live database still read that way.
use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::peak::Peak;

/// The endpoint's file name, appended when `--db-url` names a site rather than
/// the script — the same `<base>/api/upload_peak.php` the web front end posts to.
const UPLOAD_SCRIPT: &str = "api/upload_peak.php";

/// Records per POST. Small enough that a trimmed batch stays a few hundred KB.
const CHUNK: usize = 250;

pub struct Pool {
    url: String,
}

/// Where `--db-url` sends the records. A URL ending in `.php` is the endpoint
/// itself; any other http(s) URL is a site base and gets `api/upload_peak.php`
/// appended. Anything else (an old `mysql://` string) is refused, not quietly
/// redirected to some other host.
pub fn endpoint(db_url: &str) -> Result<String, String> {
    let u = db_url.trim();
    if !(u.starts_with("http://") || u.starts_with("https://")) {
        return Err(format!(
            "--db-url must be an http(s) URL of upload_peak.php or of the site serving it, got `{u}`"
        ));
    }
    let path = u.split(['?', '#']).next().unwrap_or(u);
    if path.ends_with(".php") {
        Ok(u.to_string())
    } else {
        Ok(format!("{}/{UPLOAD_SCRIPT}", u.trim_end_matches('/')))
    }
}

pub fn get_pool(db_url: &str) -> Result<Pool, String> {
    Ok(Pool { url: endpoint(db_url)? })
}

pub fn init_db(_pool: &Pool) -> Result<(), String> {
    Ok(())
}

/// The endpoint's allow-list of stored fields, as `upload_peak.php` answers a GET
/// with `?columns` (the file `upload_columns.json` beside it, in SCANALYZER.Like.audio
/// `Web_Front/public/api/`). That one file is what the endpoint stores, what the web
/// front end's `slimForUpload` keeps, and what this module keeps — it used to be three
/// hand-kept lists (this struct tree, `peakUpload.ts`, the PHP) "kept in step" by a
/// comment. Asking the endpoint also means a record is trimmed to what THAT server
/// stores, whichever version it runs.
#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct Columns {
    pub sections: BTreeMap<String, Section>,
}

/// One top-level section of a record: scalar fields kept as they are, plus lists
/// kept as their first `limit` elements, each cut to `fields`.
#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Section {
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub lists: BTreeMap<String, List>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct List {
    pub limit: usize,
    pub fields: Vec<String>,
}

fn pick(source: Option<&Value>, fields: &[String]) -> Map<String, Value> {
    let mut out = Map::new();
    if let Some(Value::Object(src)) = source {
        for f in fields {
            if let Some(v) = src.get(f) {
                out.insert(f.clone(), v.clone());
            }
        }
    }
    out
}

/// A record cut down to the stored columns — same rule as `slimForUpload` in
/// Scanalyzer's `Web_Front/src/peakUpload.ts`, over the same `Columns`.
pub fn slim(record: &Value, columns: &Columns) -> Value {
    let mut out = Map::new();
    for (name, section) in &columns.sections {
        let src = record.get(name);
        let mut kept = pick(src, &section.fields);
        for (list, spec) in &section.lists {
            let items: Vec<Value> = src
                .and_then(|s| s.get(list))
                .and_then(Value::as_array)
                .map(|a| a.iter().take(spec.limit).map(|item| Value::Object(pick(Some(item), &spec.fields))).collect())
                .unwrap_or_default();
            kept.insert(list.clone(), Value::Array(items));
        }
        out.insert(name.clone(), Value::Object(kept));
    }
    Value::Object(out)
}

/// The URL that answers with the endpoint's `Columns`.
fn columns_url(endpoint: &str) -> String {
    let sep = if endpoint.contains('?') { '&' } else { '?' };
    format!("{endpoint}{sep}columns")
}

/// Ask the endpoint which fields it stores. An endpoint too old to say is refused
/// rather than guessed at: guessing is the hand-kept copy this replaced.
pub fn fetch_columns(pool: &Pool) -> Result<Columns, String> {
    let url = columns_url(&pool.url);
    let resp = ureq::get(&url).call().map_err(|e| {
        format!("{url} did not list its upload columns ({e}) — deploy the current upload_peak.php")
    })?;
    resp.into_json::<Columns>().map_err(|e| format!("{url}: not an upload-columns document: {e}"))
}

/// The upload token, when the endpoint requires one (`SCAN_UPLOAD_TOKEN` set on the
/// server): `APK_SCAN_UPLOAD_TOKEN` in this process's environment, sent as
/// `X-Upload-Token`. Never logged.
fn upload_token() -> Option<String> {
    std::env::var("APK_SCAN_UPLOAD_TOKEN").ok().filter(|t| !t.trim().is_empty())
}

/// POST every record, in chunks. Returns how many rows the SERVER reports it
/// stored — which is not the same as how many were sent, and the difference is
/// the whole point: the old version returned `Ok(())` on any 200 and the caller
/// reported the full input length as a success.
pub fn write_peaks(pool: &Pool, peaks: &[Peak]) -> Result<usize, String> {
    if peaks.is_empty() {
        return Ok(0);
    }

    let columns = fetch_columns(pool)?;
    let token = upload_token();
    let mut stored = 0usize;
    for chunk in peaks.chunks(CHUNK) {
        let body: Vec<Value> = chunk
            .iter()
            .map(|p| serde_json::to_value(p).map(|v| slim(&v, &columns)))
            .collect::<Result<_, _>>()
            .map_err(|e| format!("a record does not serialise: {e}"))?;
        let mut req = ureq::post(&pool.url).set("Content-Type", "application/json");
        if let Some(t) = &token {
            req = req.set("X-Upload-Token", t);
        }
        let resp = req
            .send_json(&body)
            .map_err(|e| match e {
                // A 4xx/5xx carries the endpoint's own explanation — surface it
                // instead of the generic "http status 400", which told nobody
                // that the payload had been too large.
                ureq::Error::Status(code, resp) => format!(
                    "HTTP {code}: {}",
                    resp.into_string().unwrap_or_default().chars().take(300).collect::<String>()
                ),
                other => other.to_string(),
            })?;

        let text = resp.into_string().map_err(|e| e.to_string())?;
        let parsed: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("bad response: {e} — {text}"))?;
        stored += parsed
            .get("stored")
            .or_else(|| parsed.get("inserted"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
    }

    Ok(stored)
}

#[cfg(test)]
mod tests {
    use super::{columns_url, endpoint, slim, Columns};
    use serde_json::json;

    /// The shape `upload_columns.json` has; trimmed to one field per kind of rule.
    fn columns() -> Columns {
        serde_json::from_value(json!({
            "sections": {
                "metadata": {"fields": ["name", "analyzer_version"]},
                "ucs": {"fields": ["category"], "lists": {"alternatives": {"limit": 2, "fields": ["category"]}}},
                "regions": {"fields": ["count"]}
            }
        }))
        .unwrap()
    }

    #[test]
    fn slim_keeps_only_the_listed_fields() {
        let record = json!({
            "metadata": {"name": "kick.wav", "analyzer_version": "abc", "path": "/x/kick.wav"},
            "ucs": {"category": "DRUMS", "confidence": 0.9,
                    "alternatives": [{"category": "A", "score": 1}, {"category": "B"}, {"category": "C"}]},
            "regions": {"count": 2, "regions": [{"index": 0}]},
            "preview": {"peak_data_base64": "AAAA"}
        });
        let out = slim(&record, &columns());
        assert_eq!(out, json!({
            "metadata": {"name": "kick.wav", "analyzer_version": "abc"},
            "ucs": {"category": "DRUMS", "alternatives": [{"category": "A"}, {"category": "B"}]},
            "regions": {"count": 2}
        }));
    }

    #[test]
    fn a_missing_section_is_sent_empty_not_dropped() {
        let out = slim(&json!({}), &columns());
        assert_eq!(out["metadata"], json!({}));
        assert_eq!(out["ucs"]["alternatives"], json!([]));
    }

    #[test]
    fn the_columns_question_is_a_query_on_the_endpoint() {
        assert_eq!(columns_url("https://h/api/upload_peak.php"), "https://h/api/upload_peak.php?columns");
        assert_eq!(columns_url("https://h/u.php?x=1"), "https://h/u.php?x=1&columns");
    }

    #[test]
    fn db_url_names_the_endpoint() {
        let full = "https://scanalyzer.like.audio/api/upload_peak.php";
        assert_eq!(endpoint(full).unwrap(), full);
        assert_eq!(
            endpoint("http://localhost:8080/scan-api/upload_peak.php").unwrap(),
            "http://localhost:8080/scan-api/upload_peak.php"
        );
        assert_eq!(
            endpoint("http://localhost:8080/").unwrap(),
            "http://localhost:8080/api/upload_peak.php"
        );
        assert!(endpoint("mysql://user:pw@host/db").is_err());
    }
}
