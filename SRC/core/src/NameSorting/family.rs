//! Hornbostel-Sachs instrument family — a best-effort guess at what is
//! physically vibrating: Membranophone (stretched skin), Idiophone (the body
//! itself), Chordophone (strings), Aerophone (air column), Electrophone
//! (oscillators/circuits), or Voice.
//!
//! A confident name/path match decides first; otherwise the acoustic tags are
//! consulted; "" when there is no honest basis for a guess.

/// Guess the family from the name-derived group/subgroup, falling back to the
/// acoustic signal types. Returns an array of matched families/subgroups.
pub fn classify_family(group: &str, subgroup: &str, acoustic: &[String], flux: f64, sustained: bool) -> Vec<String> {
    let mut families = Vec::new();

    // --- name-derived (authoritative when the taxonomy matched) ---
    let by_name = match group {
        "Kick" | "Snare" | "Tom" => vec!["Percussion", "Membranophone", "Struck Membranophone"],
        "Cymbal" | "Hi-Hat" | "Ride" | "Rim" | "Clap" => vec!["Percussion", "Idiophone", "Struck Idiophone"],
        "Perc" => match subgroup {
            "Conga" | "Bongo" | "Taiko" | "Tabla" | "Dumbek" | "Djembe" => vec!["Percussion", "Membranophone", "Struck Membranophone"],
            "Cowbell" | "Block" | "Woodblock" | "Bell" | "Chime" | "Triangle" | "Gong" | "Tam-tam" => vec!["Percussion", "Idiophone", "Struck Idiophone"],
            "Clave" | "Castanet" | "Crash" => vec!["Percussion", "Idiophone", "Concussion Idiophone"],
            "Shaker" | "Maraca" | "Shekere" | "Tambourine" | "Jingle" => vec!["Percussion", "Idiophone", "Shaken Idiophone"],
            "Guiro" | "Washboard" => vec!["Percussion", "Idiophone", "Scraped Idiophone"],
            "Kalimba" | "Mbira" => vec!["Percussion", "Idiophone", "Plucked Idiophone"],
            "Cuica" => vec!["Percussion", "Membranophone", "Friction Membranophone"],
            "Glass" | "Saw" => vec!["Percussion", "Idiophone", "Friction Idiophone"],
            "Kazoo" => vec!["Percussion", "Membranophone", "Singing Membrane"],
            _ => vec!["Percussion"],
        },
        "Guitar" | "Bass" | "Harp" => vec!["String", "Chordophone", "Plucked Chordophone"],
        "Strings" | "Violin" | "Cello" | "Viola" => vec!["String", "Chordophone", "Bowed Chordophone"],
        "Horn" | "Brass" | "Trumpet" | "Trombone" | "Tuba" => vec!["Brass", "Aerophone"],
        "Sax" | "Woodwind" | "Flute" | "Clarinet" | "Oboe" => vec!["Woodwind", "Aerophone"],
        "Keyboards" | "Piano" => match subgroup {
            "Piano" | "Clav" => vec!["Keyboard", "Chordophone", "Struck Chordophone"],
            "Electric Piano" | "Organ" | "Synth" => vec!["Keyboard", "Electrophone"],
            _ => vec!["Keyboard"],
        },
        "Vocal" | "Choir" | "Voice" => vec!["Voice"],
        "Scratch" | "DJ" | "FX" | "Synth" => vec!["Electrophone"],
        _ => vec![],
    };

    if !by_name.is_empty() {
        for f in by_name {
            families.push(f.to_string());
        }
        return families;
    }

    // --- acoustic fallback (only where the signal type is telling) ---
    let has = |t: &str| acoustic.iter().any(|x| x == t);
    
    // Using acoustic features (ZCR, inharmonicity, etc. encoded in acoustic tags)
    if has("Inharmonic") {
        families.push("Percussion".to_string());
        families.push("Idiophone".to_string()); // detuned metallic/wooden partials
    } else if has("Stochastic") && has("Impulsive") {
        families.push("Percussion".to_string());
        families.push("Membranophone".to_string()); // noisy burst with a fast transient
    } else if has("Harmonic") && sustained && flux < 0.05 {
        // A perfectly static harmonic spectrum held indefinitely — oscillator-like.
        families.push("Electrophone".to_string());
    } else if has("Harmonic") && !sustained {
        families.push("String".to_string());
        families.push("Chordophone".to_string());
    }

    families
}

#[cfg(test)]
mod tests {
    use super::classify_family;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn name_mapping_wins_then_acoustics() {
        assert_eq!(classify_family("Kick", "", &[], 0.5, false), vec!["Percussion", "Membranophone", "Struck Membranophone"]);
        assert_eq!(classify_family("Cymbal", "", &[], 0.5, false), vec!["Percussion", "Idiophone", "Struck Idiophone"]);
        assert_eq!(classify_family("Perc", "Conga", &[], 0.5, false), vec!["Percussion", "Membranophone", "Struck Membranophone"]);
        assert_eq!(classify_family("Perc", "Shaker", &[], 0.5, false), vec!["Percussion", "Idiophone", "Shaken Idiophone"]);
        // Unnamed: acoustic tags decide.
        assert_eq!(classify_family("Unclassified", "", &s(&["Inharmonic"]), 0.5, false), vec!["Percussion", "Idiophone"]);
        assert_eq!(
            classify_family("Unclassified", "", &s(&["Stochastic", "Impulsive"]), 0.5, false),
            vec!["Percussion", "Membranophone"]
        );
        assert_eq!(classify_family("Unclassified", "", &s(&["Harmonic"]), 0.01, true), vec!["Electrophone"]);
        // No honest basis → unknown.
        assert!(classify_family("Unclassified", "", &s(&["Stochastic"]), 0.3, false).is_empty());
    }
}
