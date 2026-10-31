//! Re-score a library's UCS verdicts straight from its .PEAK sidecars.
//!
//! `ucs::classify()` runs against a *finished* `Peak`, so every feature it reads
//! is already in the sidecar. That means we can replay the classifier over a
//! whole library without decoding a single WAV — a 36k-file rescore takes
//! seconds instead of an hour, which is what makes it usable as a before/after
//! measurement while tuning the scorer.
//!
//! It reports the verdict distribution, not just a MUSICAL count: the failure
//! mode we are guarding against is not "too few MUSICAL" but "everything
//! collapses into whichever bucket is vaguest", and only the full distribution
//! shows that happening.
//!
//!   cargo run --release --example ucs_rescore -- <dir> [--top N] [--csv out.csv]

use oa_sample_analyzer::peak::Peak;
use oa_sample_analyzer::ucs;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

fn sidecars(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            sidecars(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("peak"))
        {
            out.push(path);
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let root = args.next().unwrap_or_else(|| {
        eprintln!("usage: ucs_rescore <dir> [--top N] [--csv out.csv]");
        std::process::exit(2);
    });
    let mut top = 25usize;
    let mut csv: Option<String> = None;
    let mut fix_folder = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--top" => top = args.next().and_then(|v| v.parse().ok()).unwrap_or(25),
            "--csv" => csv = args.next(),
            "--fix-folder" => fix_folder = true,
            _ => {}
        }
    }

    let mut files = Vec::new();
    sidecars(Path::new(&root), &mut files);
    files.sort();
    eprintln!("found {} .PEAK sidecars under {}", files.len(), root);

    let mut by_category: HashMap<String, usize> = HashMap::new();
    let mut by_sub: HashMap<String, usize> = HashMap::new();
    let mut confidence_sum = 0.0f64;
    let mut unreadable = 0usize;
    let mut rows: Vec<String> = Vec::new();
    let mut scored = 0usize;

    for path in &files {
        let Ok(text) = fs::read_to_string(path) else {
            unreadable += 1;
            continue;
        };
        let peak = match serde_json::from_str::<Peak>(&text) {
            Ok(p) => p,
            Err(e) => {
                if unreadable < 3 {
                    eprintln!("unreadable {}: {e}", path.display());
                }
                unreadable += 1;
                continue;
            }
        };

        // `--fix-folder` replays what a rescan would produce now that the front end
        // stamps a real relative path. Sidecars written by the buggy build carry the
        // literal string "folder", which the classifier reads as the office-supplies
        // word and matches to OBJECTS/OFFICE — so measuring against them measures the
        // bug, not the scorer.
        let mut peak = peak;
        if fix_folder {
            let rel = path
                .parent()
                .and_then(|p| p.strip_prefix(Path::new(&root)).ok())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            peak.metadata.folder = rel;   //  at the root — never a placeholder word
        }

        let v = ucs::classify(&peak);
        let category = if v.category.is_empty() {
            "(abstained)".to_string()
        } else {
            v.category.clone()
        };
        *by_category.entry(category.clone()).or_default() += 1;
        *by_sub
            .entry(format!("{}/{}", category, v.subcategory))
            .or_default() += 1;
        confidence_sum += v.confidence;
        scored += 1;

        if csv.is_some() {
            rows.push(format!(
                "{},{},{},{:.4},{}",
                peak.metadata.name.replace(',', ";"),
                category,
                v.subcategory,
                v.confidence,
                v.reason.replace(',', ";")
            ));
        }
    }

    if scored == 0 {
        eprintln!("nothing scored");
        std::process::exit(1);
    }

    let mut cats: Vec<_> = by_category.iter().collect();
    cats.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    println!("\n=== UCS category distribution ({scored} scored, {unreadable} unreadable)");
    for (name, n) in cats.iter().take(top) {
        let pct = **n as f64 / scored as f64 * 100.0;
        println!("{:>7}  {:5.1}%  {}", n, pct, name);
    }
    if cats.len() > top {
        println!("         … and {} more categories", cats.len() - top);
    }

    let musical = by_category.get("MUSICAL").copied().unwrap_or(0);
    println!(
        "\nMUSICAL: {} / {} ({:.2}%)   mean confidence: {:.3}   distinct categories: {}",
        musical,
        scored,
        musical as f64 / scored as f64 * 100.0,
        confidence_sum / scored as f64,
        cats.len()
    );

    let mut subs: Vec<_> = by_sub.iter().filter(|(k, _)| k.starts_with("MUSICAL/")).collect();
    subs.sort_by(|a, b| b.1.cmp(a.1));
    if !subs.is_empty() {
        println!("\n=== MUSICAL subcategories reached");
        for (name, n) in subs {
            println!("{:>7}  {}", n, name);
        }
    }

    if let Some(out) = csv {
        let mut body = String::from("name,category,subcategory,confidence,reason\n");
        body.push_str(&rows.join("\n"));
        fs::write(&out, body).expect("write csv");
        eprintln!("\nwrote {out}");
    }
}
