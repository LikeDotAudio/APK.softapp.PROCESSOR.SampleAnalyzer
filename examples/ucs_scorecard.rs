//! Score the classifier against two labelled corpora at once.
//!
//! A single-corpus number is a trap: you can drive "MUSICAL %" on a drum library to
//! 100 by teaching the matcher that everything is a drum, and the only place that
//! shows up is on a library of doors and birdsong. So we always measure both.
//!
//!   MUSIC corpus  — a producer's library. Drums, loops, synths, guitars, vocals.
//!                   Almost everything here IS musical, so MUSICAL % is RECALL.
//!                   (Impulse responses are the honest exception: UCS files those
//!                   outside MUSICAL, so a perfect classifier still would not hit 100.)
//!   SFX corpus    — field recordings and sound effects. Almost nothing here is
//!                   musical, so MUSICAL % is the FALSE-POSITIVE rate.
//!
//! Abstentions are reported separately from commitments throughout. A verdict of
//! CATEGORY/MISC at confidence ~0 is the classifier saying "I don't know" — counting
//! those as hits would let us claim a win for refusing to answer.
//!
//!   cargo run --release --example ucs_scorecard -- <music-dir> <sfx-dir>

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
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("peak")) {
            out.push(path);
        }
    }
}

#[derive(Default)]
struct Tally {
    scored: usize,
    unreadable: usize,
    musical_committed: usize,
    musical_abstained: usize,
    abstained: usize,
    by_category: HashMap<String, usize>,
}

fn score(root: &Path) -> Tally {
    let mut files = Vec::new();
    sidecars(root, &mut files);
    files.sort();

    let mut t = Tally::default();
    for path in &files {
        let Ok(text) = fs::read_to_string(path) else {
            t.unreadable += 1;
            continue;
        };
        let Ok(mut peak) = serde_json::from_str::<Peak>(&text) else {
            t.unreadable += 1;
            continue;
        };

        // Sidecars written before the path fix carry the literal string "folder".
        // Reconstruct the real one so we measure the classifier, not that bug.
        let rel = path
            .parent()
            .and_then(|p| p.strip_prefix(root).ok())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        peak.metadata.folder = rel;

        let v = ucs::classify(&peak);
        let abstained = v.reason.contains("ABSTAINED") || v.subcategory == "MISC";
        t.scored += 1;
        if abstained {
            t.abstained += 1;
        }
        if v.category == "MUSICAL" {
            if abstained {
                t.musical_abstained += 1;
            } else {
                t.musical_committed += 1;
            }
        }
        *t.by_category.entry(v.category.clone()).or_default() += 1;
    }
    t
}

fn pct(n: usize, d: usize) -> f64 {
    if d == 0 { 0.0 } else { n as f64 / d as f64 * 100.0 }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: ucs_scorecard <music-dir> <sfx-dir>");
        std::process::exit(2);
    }

    let music = score(Path::new(&args[0]));
    let sfx = score(Path::new(&args[1]));

    println!("\n=================== UCS SCORECARD ===================");
    println!("                              MUSIC          SFX");
    println!("                          (want MUSICAL)  (want NOT)");
    println!("  scored              {:>12} {:>12}", music.scored, sfx.scored);
    println!("  unreadable          {:>12} {:>12}", music.unreadable, sfx.unreadable);
    println!();
    println!(
        "  MUSICAL committed   {:>8} {:>4.1}% {:>7} {:>4.1}%",
        music.musical_committed,
        pct(music.musical_committed, music.scored),
        sfx.musical_committed,
        pct(sfx.musical_committed, sfx.scored)
    );
    println!(
        "  MUSICAL abstained   {:>8} {:>4.1}% {:>7} {:>4.1}%",
        music.musical_abstained,
        pct(music.musical_abstained, music.scored),
        sfx.musical_abstained,
        pct(sfx.musical_abstained, sfx.scored)
    );
    println!(
        "  abstained (any cat) {:>8} {:>4.1}% {:>7} {:>4.1}%",
        music.abstained,
        pct(music.abstained, music.scored),
        sfx.abstained,
        pct(sfx.abstained, sfx.scored)
    );
    println!();
    println!(
        "  RECALL  (music -> MUSICAL, committed) : {:.1}%",
        pct(music.musical_committed, music.scored)
    );
    println!(
        "  FALSE POSITIVES (sfx -> MUSICAL, committed) : {:.1}%",
        pct(sfx.musical_committed, sfx.scored)
    );
    println!("====================================================");

    for (label, t) in [("MUSIC", &music), ("SFX", &sfx)] {
        let mut cats: Vec<_> = t.by_category.iter().collect();
        cats.sort_by(|a, b| b.1.cmp(a.1));
        println!("\n  {label} — top categories");
        for (name, n) in cats.iter().take(8) {
            println!("    {:>7}  {:5.1}%  {}", n, pct(**n, t.scored), name);
        }
    }
}
