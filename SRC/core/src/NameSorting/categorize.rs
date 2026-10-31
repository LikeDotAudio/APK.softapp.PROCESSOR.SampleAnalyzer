//! The music-production taxonomy, read from UCS/categories/MUSICPROD.json.
//!
//! Like ucs.rs reads the UCS categories, this reads MUSICPROD — but a production role is
//! a FILE-NAME lookup, not an acoustic guess, so the data is synonyms and abbreviations
//! rather than priors and gates. The taxonomy used to be hardcoded here as a RULES table;
//! it now lives in the JSON, the single source of truth the frontend is generated from too.
use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::envelope::Envelope;
use crate::normalize::normalize_name;

// Anchor families the fallback logic names directly. They are validated against the JSON
// at load, so renaming a family in MUSICPROD.json without updating here fails fast rather
// than silently misfiling every unnamed sound.
const LOOP: &str = "LOOP";
const MELODIC: &str = "MELODIC";
const CORE_KIT: &str = "CORE KIT";
const IMPULSE_RESPONSE: &str = "IMPULSE RESPONSE";
const MISC: &str = "MISC";

#[derive(Deserialize)]
struct FamilyMeta {
    family: String,
    #[serde(default)]
    percussive: bool,
}

#[derive(Deserialize)]
struct Variation {
    name: String,
    #[serde(default)]
    phrases: Vec<String>,
    #[serde(default)]
    abbrevs: Vec<String>,
}

#[derive(Deserialize)]
struct Instrument {
    instrument: String,
    family: String,
    #[serde(default)]
    phrases: Vec<String>,
    #[serde(default)]
    abbrevs: Vec<String>,
    #[serde(default)]
    variations: Vec<Variation>,
}

#[derive(Deserialize)]
struct MusicProd {
    families: Vec<FamilyMeta>,
    instruments: Vec<Instrument>,
}

pub struct Taxonomy {
    doc: MusicProd,
    /// instrument name -> its family. First wins, so a duplicated instrument (the
    /// generic-drum Perc rule appears twice) keeps the family of its first entry.
    family_of: HashMap<String, String>,
    /// family name -> is it a struck/hit family (root note is usually noise).
    percussive: HashMap<String, bool>,
}

/// The producer file-name taxonomy — a PRIVATE analyzer asset, not a UCS category.
/// It carries name synonyms/abbreviations (not acoustic signatures), and only drives
/// the internal `group`/`subgroup` that family, clustering and labels read. It used to
/// live at `UCS/categories/MUSICPROD.json` and be bundled by build.rs; it now sits
/// beside this module so it is out of the UCS taxonomy entirely.
const ROLE_DATA: &str = include_str!("music_names.json");

fn load() -> Taxonomy {
    let doc: MusicProd = serde_json::from_str(ROLE_DATA)
        .expect("music_names.json does not match the taxonomy schema");

    let mut family_of = HashMap::new();
    for inst in &doc.instruments {
        family_of
            .entry(inst.instrument.clone())
            .or_insert_with(|| inst.family.clone());
    }

    let percussive: HashMap<String, bool> = doc
        .families
        .iter()
        .map(|f| (f.family.clone(), f.percussive))
        .collect();

    // Fail fast if the JSON renamed a family the fallback logic names by hand.
    for anchor in [LOOP, MELODIC, CORE_KIT, IMPULSE_RESPONSE, MISC] {
        assert!(
            percussive.contains_key(anchor),
            "MUSICPROD.json has no family {anchor:?} — the fallback logic in categorize.rs \
             names it directly and would misfile every unnamed sound"
        );
    }
    Taxonomy {
        doc,
        family_of,
        percussive,
    }
}

pub fn taxonomy() -> &'static Taxonomy {
    static T: OnceLock<Taxonomy> = OnceLock::new();
    T.get_or_init(load)
}

/// The family an instrument belongs to, or None if the name is unknown (the caller
/// falls back to the measured envelope). References into the 'static taxonomy.
pub fn family_of(group: &str) -> Option<&'static str> {
    taxonomy().family_of.get(group).map(String::as_str)
}

/// Assign the production FAMILY. A loop is a LOOP whatever it contains; a recognized
/// instrument maps to its family (MUSICPROD.json); anything the name could not place is
/// classified by its measured envelope. `subgroup` is unused — the family is decided by
/// the instrument alone — but kept so the call site and record schema do not change.
///
/// `file_shape` is the FILE's categorical shape ("Multi" when it holds several
/// transients); `env` is the representative single event's measurement. The two are
/// separate because a multi-event file has no single-event ADSR — its slices each
/// have one — so the shape must come from the file and the durations from a slice.
pub fn music_prod_category(
    group: &str,
    _subgroup: &str,
    is_loop: bool,
    file_shape: &str,
    env: &Envelope,
) -> &'static str {
    if is_loop {
        return LOOP;
    }
    family_of(group).unwrap_or_else(|| from_envelope(file_shape, env))
}

/// A struck / hit family, where a root note is usually meaningless noise. Read from the
/// `percussive` flag in MUSICPROD.json.
pub fn is_percussive_family(family: &str) -> bool {
    taxonomy().percussive.get(family).copied().unwrap_or(false)
}

/// Family for a file whose name told us nothing — decided by the envelope shape.
fn from_envelope(file_shape: &str, env: &Envelope) -> &'static str {
    match file_shape {
        "Multi" => LOOP,
        "Sustained" | "Swell" => MELODIC,
        "Plucky" => CORE_KIT,
        "Decaying" => {
            // Fast attack, no plateau: the length of the die-off separates a hit from a
            // ringing wash (a reverb-like tail rings well past half a second).
            if env.decay + env.release > 0.5 {
                IMPULSE_RESPONSE
            } else {
                CORE_KIT
            }
        }
        _ => MISC, // "Silent" or unmeasurable
    }
}

/// Categorize a sample by its (full-path) name, tolerant of the many spelling /
/// abbreviation conventions for drum elements. Phrases match as substrings of the
/// normalized name; ABBREVIATIONS match as whole tokens (so "bd" hits "BD_01" but not
/// "bird"). Instruments are tried in file order (most specific first); a variation is
/// tried before its instrument's base.
///
/// Returns (instrument, variation, matched-token), all borrowed from the 'static
/// taxonomy. `variation` is "" when only the base matched.
pub fn categorize(name: &str) -> (&'static str, &'static str, &'static str) {
    let t = taxonomy();
    let norm = normalize_name(name);
    let toks: Vec<&str> = norm.split_whitespace().collect();
    let tok = |s: &str| toks.iter().any(|x| *x == s);
    let ph = |p: &str| norm.contains(p);

    let hit = |phrases: &'static [String], abbrevs: &'static [String]| -> Option<&'static str> {
        if let Some(p) = phrases.iter().find(|p| ph(p)) {
            return Some(p.as_str());
        }
        if let Some(a) = abbrevs.iter().find(|a| tok(a)) {
            return Some(a.as_str());
        }
        None
    };

    for inst in &t.doc.instruments {
        for v in &inst.variations {
            if let Some(why) = hit(&v.phrases, &v.abbrevs) {
                return (inst.instrument.as_str(), v.name.as_str(), why);
            }
        }
        if let Some(why) = hit(&inst.phrases, &inst.abbrevs) {
            return (inst.instrument.as_str(), "", why);
        }
    }
    ("Unclassified", "", "")
}

#[cfg(test)]
mod tests {
    use super::{categorize, is_percussive_family, music_prod_category};
    use crate::envelope::Envelope;

    fn env(shape: &'static str, decay: f64, release: f64) -> Envelope {
        Envelope {
            attack: 0.005,
            decay,
            sustain: 0.05,
            release,
            centroid: 0.2,
            skew: 0.0,
            kurt: 0.0,
            shape,
            decay_time_60db: None,
        }
    }

    /// The pipeline's own pairing: the representative slice carries the shape on a
    /// one-shot, and the file overrides it with "Multi" when there are several.
    fn family(group: &str, subgroup: &str, is_loop: bool, e: &Envelope) -> &'static str {
        music_prod_category(group, subgroup, is_loop, e.shape, e)
    }

    #[test]
    fn instruments_map_to_their_families() {
        let e = env("Plucky", 0.05, 0.05);
        assert_eq!(family("Kick", "", false, &e), "CORE KIT");
        assert_eq!(family("Snare", "Rimshot", false, &e), "CORE KIT");
        assert_eq!(family("Crash", "", false, &e), "CYMBALS & METALS");
        assert_eq!(family("Shaker", "", false, &e), "HAND PERCUSSION");
        assert_eq!(family("Perc", "", false, &e), "HAND PERCUSSION");
        assert_eq!(family("Conga", "", false, &e), "WORLD & REGIONAL");
        assert_eq!(family("Marimba", "", false, &e), "ORCHESTRAL & PITCHED");
        assert_eq!(family("808", "", false, &e), "ELECTRONIC & DESIGN");
        assert_eq!(family("Vocal", "", false, &e), "ELECTRONIC & DESIGN");
        assert_eq!(family("Guitar", "", false, &e), "MELODIC");
        assert_eq!(family("Keyboards", "Synth", false, &e), "MELODIC");
        assert_eq!(family("IR", "", false, &e), "IMPULSE RESPONSE");
    }

    #[test]
    fn loop_wins_and_envelope_fallback() {
        let e = env("Plucky", 0.05, 0.05);
        assert_eq!(family("Kick", "", true, &e), "LOOP");
        assert_eq!(family("Loops/Patterns", "", false, &e), "LOOP");
        assert_eq!(family("Unclassified", "", false, &env("Plucky", 0.05, 0.05)), "CORE KIT");
        assert_eq!(family("Unclassified", "", false, &env("Decaying", 1.2, 0.8)), "IMPULSE RESPONSE");
        assert_eq!(family("Unclassified", "", false, &env("Decaying", 0.1, 0.1)), "CORE KIT");
        assert_eq!(family("Unclassified", "", false, &env("Sustained", 0.1, 0.4)), "MELODIC");
        assert_eq!(family("Unclassified", "", false, &env("Silent", 0.0, 0.0)), "MISC");
    }

    /// A multi-transient file is a LOOP on the FILE's shape, whatever the loudest
    /// slice measured — the slice is a single hit and will never say "Multi".
    #[test]
    fn a_multi_event_file_falls_back_to_loop_on_the_file_shape() {
        let loudest_hit = env("Plucky", 0.05, 0.05);
        assert_eq!(
            music_prod_category("Unclassified", "", false, "Multi", &loudest_hit),
            "LOOP"
        );
    }

    #[test]
    fn percussive_flag_comes_from_the_json() {
        assert!(is_percussive_family("CORE KIT"));
        assert!(is_percussive_family("CYMBALS & METALS"));
        assert!(is_percussive_family("WORLD & REGIONAL"));
        assert!(!is_percussive_family("MELODIC"));
        assert!(!is_percussive_family("ELECTRONIC & DESIGN"));
    }

    #[test]
    fn naming_conventions() {
        // (name, want_group, want_subgroup) — the producer's drum-sampler taxonomy.
        let cases: &[(&str, &str, &str)] = &[
            // === 1. Core kit ===
            ("Kick_01", "Kick", ""), ("BD_808", "Kick", ""), ("Bass Drum 3", "Kick", ""),
            ("Kk-tight", "Kick", ""), ("BassDrum", "Kick", ""), ("BDrum_7", "Kick", ""),
            ("BassD_2", "Kick", ""), ("BDR_4", "Kick", ""),
            ("Snare_Acoustic", "Snare", ""), ("SD_04", "Snare", ""), ("Snr_dry", "Snare", ""),
            ("Rimshot", "Snare", "Rimshot"), ("RS_dry", "Snare", "Rimshot"),
            ("Cross-stick", "Snare", "Cross-stick"), ("XSTK_01", "Snare", "Cross-stick"),
            ("Clap_big", "Clap", ""), ("Handclap", "Clap", ""), ("CP_room", "Clap", ""),
            ("Snap_01", "Snap", ""), ("Finger Snap", "Snap", ""),
            ("CHH_tight", "Hi-Hat", "Closed"), ("OHH_01", "Hi-Hat", "Open"),
            ("Pedal Hat", "Hi-Hat", "Pedal"), ("HH_01", "Hi-Hat", ""),
            ("ClosedHH1", "Hi-Hat", ""), ("808HH", "Hi-Hat", ""),
            ("High Tom", "Tom", "Hi"), ("HT_rack", "Tom", "Hi"), ("Tom1", "Tom", "Hi"),
            ("Mid Tom", "Tom", "Mid"), ("Tom2", "Tom", "Mid"),
            ("Floor Tom", "Tom", "Floor"), ("FT_low", "Tom", "Floor"), ("Tom3", "Tom", "Floor"),
            ("Tom_generic", "Tom", ""),
            // === 2. Cymbals & metals — each its own instrument ===
            ("Crash_01", "Crash", ""), ("CRSH", "Crash", ""), ("CC_loud", "Crash", ""),
            ("Ride_01", "Ride", ""), ("RD_ping", "Ride", ""), ("RC_jazz", "Ride", ""),
            ("Ride Bell", "Ride Bell", ""), ("RDB", "Ride Bell", ""),
            ("Splash", "Splash", ""), ("SPL_fast", "Splash", ""),
            ("China", "China", ""), ("CHN_trash", "China", ""),
            ("Gong_low", "Cymbal", "Gong"), ("OHCYM", "Cymbal", ""), ("Zildjian_18", "Cymbal", ""),
            // === 3. Hand percussion & shakers ===
            ("Cowbell", "Cowbell", ""), ("CB_hi", "Cowbell", ""), ("Cow_1", "Cowbell", ""),
            ("Shaker", "Shaker", ""), ("Maracas", "Shaker", ""), ("SHKR_loop", "Shaker", ""),
            ("Tambourine", "Tambourine", ""), ("TAMB", "Tambourine", ""),
            ("Woodblock", "Woodblock", ""), ("Claves", "Woodblock", ""), ("Castanet", "Woodblock", ""),
            ("Guiro", "Guiro", ""), ("Scraper", "Guiro", ""),
            ("Triangle_open", "Triangle", ""),
            ("Perc_shot", "Perc", ""), ("PRC_02", "Perc", ""),
            // === 4. World & regional ===
            ("Conga_open", "Conga", ""), ("CNG_hi", "Conga", ""), ("Tumba", "Conga", ""),
            ("Bongo_hi", "Bongo", ""), ("BNG", "Bongo", ""),
            ("Timbale", "Timbale", ""), ("Djembe", "Djembe", ""), ("DJM_slap", "Djembe", ""),
            ("Talking Drum", "Talking Drum", ""), ("Darbuka", "Darbuka", ""),
            ("Taiko_hit", "Taiko", ""), ("Cajon", "Cajon", ""), ("Surdo", "Surdo", ""),
            ("Tabla_na", "Tabla", ""),
            // === 5. Orchestral & pitched ===
            ("Marimba_A", "Marimba", ""), ("Vibraphone", "Vibraphone", ""),
            ("Xylophone", "Xylophone", ""), ("Glockenspiel", "Glockenspiel", ""),
            ("Timpani_roll", "Timpani", ""), ("Steel Pan", "Steel Pan", ""),
            ("Kalimba_A", "Kalimba", ""), ("Mbira", "Kalimba", ""),
            // === 6. Electronic & sound design ===
            ("808", "808", ""), ("808_sub", "808", ""),
            ("Vinyl_crackle", "Vinyl", ""), ("Crackle", "Vinyl", ""),
            ("Scratch_01", "Scratch", ""), ("Turntable_stop", "DJ", ""),
            ("Vox_chop", "Vocal", ""), ("Chant", "Vocal", ""),
            ("FX_riser", "FX", "Riser"), ("Impact_hit", "FX", "Impact"), ("Laser_zap", "FX", ""),
            // === Melodic (not a drum family, but the library is full of it) ===
            ("Guitar_riff", "Guitar", ""), ("Strings_A", "Strings", ""), ("Violin", "Strings", ""),
            ("Horn_stab", "Horn", ""), ("Trumpet", "Horn", ""), ("Saxophone_A2", "Sax", ""),
            ("Bassline_A", "Bass", ""), ("Slap Bass G", "Bass", ""),
            ("SynthLead", "Keyboards", "Synth"), ("Grand Piano C3", "Keyboards", "Piano"),
            ("Rhodes_ep", "Keyboards", "Electric Piano"), ("Hammond_organ", "Keyboards", "Organ"),
            ("Piano Note C3", "Keyboards", "Piano"), ("Note_C3", "Note", ""),
            // === IR + loops + fallthrough ===
            ("Hall_IR", "IR", ""), ("guitar_cab", "IR", ""), ("Convolution 01", "IR", ""),
            ("Loop_120", "Loops/Patterns", ""), ("Groove", "Loops/Patterns", ""),
            ("randomthing", "Unclassified", ""),
        ];
        for (name, want_g, want_sg) in cases {
            let (got, sub, why) = categorize(name);
            assert_eq!(got, *want_g, "categorize({:?}) group = {:?} (why {:?}), want {:?}", name, got, why, want_g);
            assert_eq!(sub, *want_sg, "categorize({:?}) subgroup = {:?}, want {:?}", name, sub, want_sg);
        }
    }
}
