//! Two jobs at compile time:
//!
//! 1. Bundle the UCS category data. The signed category files in UCS/categories/
//!    are the source of truth; the analyzer used to carry its own stale copy,
//!    which drifted. We now splice them into one array in OUT_DIR and embed
//!    that, so the two cannot disagree.
//!
//! 2. Stamp the build with an analyzer version: a hash of what the analyzer
//!    computes from — the Rust under src/ and the shared extractor engine's src/
//!    with comments and layout stripped, plus the bundled UCS data. A comment-only
//!    or reformatting edit leaves it unchanged; a code or data edit changes it,
//!    and every existing .PEAK sidecar carrying another version is then
//!    re-analyzed. When the version moves, the build says so (cargo:warning).
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
    println!("cargo:rerun-if-changed={EXTRACTOR_SRC}");
    println!("cargo:rerun-if-changed=../../DATATYPE/DATATYPE.soundfx.UCS/categories");
    println!("cargo:rerun-if-changed=../../DATATYPE/DATATYPE.soundfx.UCS/producer_synonyms.json");

    let ucs_dir_path = if Path::new("../../DATATYPE/DATATYPE.soundfx.UCS/categories").exists() {
        Path::new("../../DATATYPE/DATATYPE.soundfx.UCS/categories")
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

    // The producer-vocabulary overlay. UCS 8.2.1 is a sound-effects standard: none of its
    // subcategories contains "hihat", "cymbal", "808" or "oneshot", and "hat" belongs to
    // OBJECTS/FASHION. A music library therefore has no UCS words to match on. The overlay
    // adds those words, keyed by UCS category_id, and is merged here rather than edited
    // into UCS/categories/ so the vendored spec stays pristine and upgradeable.
    let overlay_path = if Path::new("../../DATATYPE/DATATYPE.soundfx.UCS/producer_synonyms.json").exists() {
        Path::new("../../DATATYPE/DATATYPE.soundfx.UCS/producer_synonyms.json")
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

    // Stamp the build: a hash of the analyzer's inputs, not a timestamp. The
    // timestamp re-stamped on any edit under src/, so a comment fix made every
    // library re-analyze. Comments and whitespace are stripped before hashing,
    // so only a change to what the code or data says moves the version.
    let mut h = Fnv::new();
    for dir in [Path::new("src"), Path::new(EXTRACTOR_SRC)] {
        if !dir.exists() {
            continue;
        }
        let mut files = Vec::new();
        rust_files(dir, &mut files);
        files.sort();
        for f in files {
            let rel = f.strip_prefix(dir).unwrap_or(&f).to_string_lossy().replace('\\', "/");
            let text = fs::read_to_string(&f).unwrap_or_else(|e| panic!("read {}: {e}", f.display()));
            h.feed(rel.as_bytes());
            h.feed(&[0]);
            h.feed(strip_rust(&text).as_bytes());
            h.feed(&[0]);
        }
    }
    h.feed(bundle.as_bytes());
    h.feed(&[0]);
    h.feed(role_bundle.as_bytes());
    let version = format!("{:012x}", h.0 & 0xffff_ffff_ffff);

    // Say so when it moves: every .PEAK sidecar from the old version will be
    // re-analyzed at the next scan, and that should never be a surprise.
    let last = out.join("analyzer_version.txt");
    if let Ok(prev) = fs::read_to_string(&last) {
        let prev = prev.trim();
        if !prev.is_empty() && prev != version {
            println!(
                "cargo:warning=ANALYZER_VERSION changed {prev} -> {version}: every .PEAK sidecar from {prev} will be re-analyzed at the next scan"
            );
        }
    }
    let _ = fs::write(&last, &version);
    println!("cargo:rustc-env=ANALYZER_HASH={version}");
}

/// The shared region-cutting DSP; its code shapes stored regions, so it is an input too.
const EXTRACTOR_SRC: &str = "../../CAPTURE/CAPTURE.Extractor-Engine/src";

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
            out.push(p);
        }
    }
}

/// 64-bit FNV-1a: stable across machines and Rust versions (std's hasher is not).
struct Fnv(u64);
impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn feed(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= *b as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

/// Rust source with comments removed and every whitespace run outside literals
/// collapsed to one space. String, raw-string, byte-string and char literals are
/// kept verbatim (a `//` inside "http://…" is not a comment); a lifetime `'a`
/// is told from a char literal by the closing quote.
fn strip_rust(src: &str) -> String {
    let c: Vec<char> = src.chars().collect();
    let n = c.len();
    let mut out = String::with_capacity(n);
    let mut i = 0;
    let mut space = false;
    let push = |out: &mut String, space: &mut bool, ch: char| {
        if *space && !out.is_empty() {
            out.push(' ');
        }
        *space = false;
        out.push(ch);
    };
    while i < n {
        let ch = c[i];
        if ch == '/' && i + 1 < n && c[i + 1] == '/' {
            while i < n && c[i] != '\n' {
                i += 1;
            }
            space = true;
        } else if ch == '/' && i + 1 < n && c[i + 1] == '*' {
            let mut depth = 1;
            i += 2;
            while i < n && depth > 0 {
                if c[i] == '/' && i + 1 < n && c[i + 1] == '*' {
                    depth += 1;
                    i += 2;
                } else if c[i] == '*' && i + 1 < n && c[i + 1] == '/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            space = true;
        } else if ch.is_whitespace() {
            space = true;
            i += 1;
        } else if ch == 'r' && i + 1 < n && (c[i + 1] == '"' || c[i + 1] == '#')
            && !(i > 0 && (c[i - 1].is_alphanumeric() || c[i - 1] == '_'))
        {
            // raw string r"…" / r#"…"#
            let mut j = i + 1;
            let mut hashes = 0;
            while j < n && c[j] == '#' {
                hashes += 1;
                j += 1;
            }
            if j < n && c[j] == '"' {
                j += 1;
                loop {
                    if j >= n {
                        break;
                    }
                    if c[j] == '"' && (0..hashes).all(|k| j + 1 + k < n && c[j + 1 + k] == '#') {
                        j += 1 + hashes;
                        break;
                    }
                    j += 1;
                }
                for &x in &c[i..j.min(n)] {
                    push(&mut out, &mut space, x);
                }
                i = j;
            } else {
                push(&mut out, &mut space, ch);
                i += 1;
            }
        } else if ch == '"' {
            let mut j = i + 1;
            while j < n && c[j] != '"' {
                if c[j] == '\\' {
                    j += 1;
                }
                j += 1;
            }
            j = (j + 1).min(n);
            for &x in &c[i..j] {
                push(&mut out, &mut space, x);
            }
            i = j;
        } else if ch == '\'' {
            // char literal 'x' / '\n' / '\u{..}', else a lifetime or label
            let j = if i + 1 < n && c[i + 1] == '\\' {
                let mut j = i + 2;
                while j < n && c[j] != '\'' {
                    j += 1;
                }
                (j + 1).min(n)
            } else if i + 2 < n && c[i + 2] == '\'' {
                i + 3
            } else {
                i + 1
            };
            for &x in &c[i..j] {
                push(&mut out, &mut space, x);
            }
            i = j;
        } else {
            push(&mut out, &mut space, ch);
            i += 1;
        }
    }
    out
}
