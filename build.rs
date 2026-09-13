//! Two jobs at compile time:
//!
//! 1. Bundle the UCS category data. The signed category files in UCS/categories/
//!    are the source of truth; the analyzer used to carry its own stale copy,
//!    which drifted. We now splice them into one array in OUT_DIR and embed
//!    that, so the two cannot disagree.
//!
//! 2. Stamp the build with an analyzer version: the timestamp, to the minute.
//!    The `rerun-if-changed` lines below scope it — this file re-runs only when
//!    an extractor under src/ or a UCS category signature changes, so the stamp
//!    marks when the analyzer's inputs last moved, and every existing .PEAK
//!    sidecar carrying an older stamp is invalidated and re-analyzed.
use std::fs;
use std::path::{Path, PathBuf};

fn sorted_files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some(ext))
        .collect();
    v.sort();
    v
}

fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=../../UCS/categories");
    println!("cargo:rerun-if-changed=../../UCS/producer_synonyms.json");

    let ucs_dir_path = if Path::new("../../UCS/categories").exists() {
        Path::new("../../UCS/categories")
    } else {
        Path::new("../UCS/categories")
    };
    let cat_files: Vec<PathBuf> = sorted_files(ucs_dir_path, "json")
        .into_iter()
        .filter(|p| p.file_name().and_then(|x| x.to_str()) != Some("index.json"))
        .collect();
    assert!(
        !cat_files.is_empty(),
        "no UCS category files under {} — the analyzer cannot classify without them",
        ucs_dir_path.display()
    );

    // Splice the per-category files into a single JSON array, MINIFIED.
    //
    // The category files on disk are pretty-printed for humans to read and diff,
    // and that indentation is ~29 % of their bytes. This bundle is `include_str!`d
    // into the binary, so every one of those spaces used to be compiled into the
    // analyzer *and* shipped to the browser inside the WASM engine — 822 KB of
    // whitespace downloaded by every visitor. Re-serializing compact here costs a
    // build-time parse and changes nothing about what the data MEANS: serde parses
    // both forms to the same structs.
    //
    // A file may opt out of the matcher with `"matchable": false`. Those describe a
    // different axis (MUSICPROD answers "what role does this play in a production",
    // not "what is this sound"), and they reuse the synonyms and signatures of a real
    // UCS category. Letting one into the index would give every music sample a twin
    // candidate with an identical signature — splitting the posterior and lowering
    // the IDF of every music token. They are bundled separately instead.
    let matchable = |p: &PathBuf| -> bool {
        let text = fs::read_to_string(p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
        let value: serde_json::Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", p.display()));
        value.get("matchable").and_then(|m| m.as_bool()).unwrap_or(true)
    };

    // The producer-vocabulary overlay. UCS is a sound-effects standard: none of its 756
    // subcategories contains "hihat", "cymbal", "808" or "oneshot", and "hat" belongs to
    // OBJECTS/FASHION. A music library therefore has no UCS words to match on. The overlay
    // adds those words, keyed by UCS category_id, and is merged here rather than edited
    // into UCS/categories/ so the vendored spec stays pristine and upgradeable.
    let overlay_path = if Path::new("../../UCS/producer_synonyms.json").exists() {
        Path::new("../../UCS/producer_synonyms.json")
    } else {
        Path::new("../UCS/producer_synonyms.json")
    };
    let overlay: serde_json::Value = {
        let text = fs::read_to_string(overlay_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", overlay_path.display()));
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", overlay_path.display()))
    };
    let additions = overlay
        .get("additions")
        .and_then(|a| a.as_object())
        .unwrap_or_else(|| panic!("{} has no `additions` object", overlay_path.display()))
        .clone();

    let mut merged_ids: Vec<String> = Vec::new();
    let bodies: Vec<String> = cat_files
        .iter()
        .filter(|p| matchable(p))
        .map(|p| {
            let text = fs::read_to_string(p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
            let mut value: serde_json::Value = serde_json::from_str(&text)
                .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", p.display()));

            if let Some(subs) = value.get_mut("subcategories").and_then(|s| s.as_array_mut()) {
                for sub in subs {
                    let Some(id) = sub
                        .get("category_id")
                        .and_then(|i| i.as_str())
                        .map(str::to_string)
                    else {
                        continue;
                    };
                    let Some(extra) = additions.get(&id).and_then(|e| e.as_array()) else {
                        continue;
                    };
                    let Some(syns) = sub.get_mut("synonyms").and_then(|s| s.as_array_mut()) else {
                        continue;
                    };
                    for word in extra {
                        if !syns.contains(word) {
                            syns.push(word.clone());
                        }
                    }
                    merged_ids.push(id);
                }
            }
            serde_json::to_string(&value).expect("re-serialize category")
        })
        .collect();

    // A typo'd category_id would silently add nothing at all, and the only symptom
    // would be a classifier that quietly keeps misfiling drums. Fail the build instead.
    for id in additions.keys() {
        assert!(
            merged_ids.contains(id),
            "{}: `{id}` matches no UCS category_id — the overlay would be a no-op",
            overlay_path.display()
        );
    }

    let bundle = format!("[{}]", bodies.join(","));

    // The opted-out files, bundled on their own. MUSICPROD is read from here.
    let role_bodies: Vec<String> = cat_files
        .iter()
        .filter(|p| !matchable(p))
        .map(|p| {
            let text = fs::read_to_string(p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
            let value: serde_json::Value = serde_json::from_str(&text)
                .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", p.display()));
            serde_json::to_string(&value).expect("re-serialize category")
        })
        .collect();
    let role_bundle = format!("[{}]", role_bodies.join(","));

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    fs::write(out.join("ucs_signed.json"), &bundle).expect("write ucs_signed.json");
    fs::write(out.join("ucs_roles.json"), &role_bundle).expect("write ucs_roles.json");

    // Stamp the build. The `rerun-if-changed` lines above are what make this a
    // meaningful version: this file only re-runs when `src/` or the UCS data
    // changes, so the timestamp marks the minute the analyzer's inputs last
    // changed, and a record stamped with an older one needs re-analysis.
    let date_output = std::process::Command::new("date")
        .arg("+%Y%m%d.%H%M")
        .output()
        .expect("failed to execute date command");
    let date_str = String::from_utf8_lossy(&date_output.stdout).trim().to_string();
    println!("cargo:rustc-env=ANALYZER_DATE={}", date_str);
}
