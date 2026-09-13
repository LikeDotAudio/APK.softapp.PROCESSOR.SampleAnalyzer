//! Multi-label timbre tags. Timbre is multidimensional, so one sound can
//! legitimately carry several labels at once — a rimshot is both Stochastic
//! and Impulsive; a synth bass is both Harmonic and a Bass.
//!
//! Two independent tag sets:
//!   * acoustic signal types — what the spectrum IS
//!     (Harmonic / Inharmonic / Stochastic / Impulsive)
//!   * sound-design roles — how the envelope + spectrum BEHAVE in a mix
//!     (Pad / Pluck / Lead / Bass)
use crate::envelope::Envelope;
use crate::partials::Partials;

/// Acoustic signal types (any combination, never empty).
///
/// Harmonic   — clear pitch, overtones at integer multiples of the fundamental
/// Inharmonic — overtones present but detuned from the harmonic series (metallic)
/// Stochastic — energy smeared across the spectrum, no discernible pitch (noisy)
/// Impulsive  — a short burst of energy that decays almost instantly (transient)
pub fn acoustic_tags(
    harmonicity: f64,
    flatness: f64,
    partials: &Partials,
    length: f64,
    env: &Envelope,
) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    if harmonicity > 0.45 && partials.inharmonicity < 0.2 {
        tags.push("Harmonic".into());
    }
    if partials.count >= 3 && partials.inharmonicity >= 0.2 && flatness < 0.5 {
        tags.push("Inharmonic".into());
    }
    if flatness > 0.2 || (harmonicity < 0.15 && partials.count < 3) {
        tags.push("Stochastic".into());
    }
    if length < 0.6 && env.attack < 0.025 && env.sustain < 0.2 && env.decay < 0.2 {
        tags.push("Impulsive".into());
    }
    if tags.is_empty() {
        // Weakly pitched, mildly structured — pick the closer pole.
        tags.push(if harmonicity > 0.3 { "Harmonic".into() } else { "Stochastic".into() });
    }
    tags
}

/// Sound-design roles (any combination; drums/FX legitimately get none).
/// Driven primarily by the measured ADSR envelope:
///
/// Pad   — slow attack, high sustain, long release: fills space
/// Pluck — instant attack, fast decay, no sustain, tonal: rhythmic motion
/// Lead  — fast attack but held and bright enough to cut through a mix
/// Bass  — pitched, energy concentrated below ~200 Hz: the anchor
#[allow(clippy::too_many_arguments)]
pub fn sound_design_tags(
    env: &Envelope,
    transients: usize,
    harmonicity: f64,
    centroid: f64,
    low: f64,
    high: f64,
    pitch: f64,
    root_hz: f64,
) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    let one_shot = transients <= 1;
    let f0 = if root_hz > 0.0 { root_hz } else { pitch };
    let bass = low > 0.5 && f0 > 0.0 && f0 < 200.0 && harmonicity > 0.25;

    if one_shot && env.attack > 0.15 && env.sustain > 0.4 {
        tags.push("Pad".into());
    }
    if one_shot && env.attack < 0.03 && env.sustain < 0.2 && harmonicity > 0.3 {
        tags.push("Pluck".into());
    }
    if one_shot
        && !bass
        && env.attack <= 0.15
        && env.sustain > 0.5
        && harmonicity > 0.45
        && centroid > 1200.0
        && high > 0.15
    {
        tags.push("Lead".into());
    }
    if bass {
        tags.push("Bass".into());
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(attack: f64, decay: f64, sustain: f64) -> Envelope {
        Envelope { attack, decay, sustain, release: 0.1, centroid: 0.3, skew: 0.0, kurt: 0.0, shape: "", decay_time_60db: None }
    }

    #[test]
    fn acoustic_multi_labels() {
        // Rimshot-like: noisy AND over in an instant → two tags.
        let t = acoustic_tags(0.1, 0.4, &Partials { count: 1, inharmonicity: 0.0 }, 0.15, &env(0.002, 0.05, 0.05));
        assert!(t.contains(&"Stochastic".to_string()) && t.contains(&"Impulsive".to_string()), "{:?}", t);

        // Flute-like: pitched, integer overtones → Harmonic only.
        let t = acoustic_tags(0.8, 0.02, &Partials { count: 8, inharmonicity: 0.05 }, 2.0, &env(0.1, 0.2, 0.8));
        assert_eq!(t, vec!["Harmonic"]);

        // Bell-like: strong detuned partials → Inharmonic.
        let t = acoustic_tags(0.3, 0.05, &Partials { count: 7, inharmonicity: 0.5 }, 3.0, &env(0.005, 1.5, 0.3));
        assert!(t.contains(&"Inharmonic".to_string()), "{:?}", t);
    }

    #[test]
    fn sound_design_roles() {
        // Slow swell held high → Pad.
        let t = sound_design_tags(&env(0.4, 0.1, 0.8), 1, 0.7, 900.0, 0.3, 0.1, 220.0, 0.0);
        assert_eq!(t, vec!["Pad"]);

        // Instant tonal hit that dies → Pluck.
        let t = sound_design_tags(&env(0.005, 0.1, 0.05), 1, 0.6, 1500.0, 0.3, 0.2, 330.0, 0.0);
        assert_eq!(t, vec!["Pluck"]);

        // Bright, held, pitched → Lead.
        let t = sound_design_tags(&env(0.01, 0.05, 0.8), 1, 0.7, 2500.0, 0.2, 0.3, 440.0, 0.0);
        assert_eq!(t, vec!["Lead"]);

        // Low, pitched, sub-heavy → Bass; a plucky bassline gets both tags.
        let t = sound_design_tags(&env(0.005, 0.1, 0.05), 1, 0.6, 300.0, 0.8, 0.05, 55.0, 0.0);
        assert!(t.contains(&"Bass".to_string()) && t.contains(&"Pluck".to_string()), "{:?}", t);

        // A noisy drum hit matches no role.
        let t = sound_design_tags(&env(0.002, 0.05, 0.05), 1, 0.1, 3000.0, 0.3, 0.4, 0.0, 0.0);
        assert!(t.is_empty(), "{:?}", t);
    }
}
