//! Shared test fixtures for the syntax modules.
//!
//! Every test here is an integration test in miniature — the interesting
//! behavior (does a wobble invent a vocabulary, does a fading gap read as a
//! tail) only exists once regions, envelope and entry point are assembled
//! together. Rather than rebuild that scaffolding in each module's `tests`, it
//! lives here and each module tests its own subject through it.

use super::{bioacoustic_syntax, BioacousticSyntax};
use crate::peak::{Region, Regions};

/// A region carrying just enough analysis for the vector: the fields
/// `slice_vector` reads. `kind` picks one of two very different timbres.
pub(super) fn region(index: usize, start: f64, duration: f64, kind: usize) -> Region {
    let mut p = crate::peak::Peak {
        metadata: crate::peak::Metadata {
            analyzer_version: String::new(),
            name: String::new(),
            folder: String::new(),
            sub: String::new(),
            path: String::new(),
            length_seconds: duration,
            sample_rate: 44100,
            bit_depth: 16,
            channels: 1,
            source_format: String::new(),
            lossy_source: false,
            dc_offset: 0.0,
            trailing_silence_ms: 0.0,
            analysis_depth: "full".to_string(),
        },
        classification: Default::default(),
        envelope: Default::default(),
        spectral_features: Default::default(),
        musicality: Default::default(),
        unsupervised: Default::default(),
        ucs: Default::default(),
        regions: Default::default(),
        preview: Default::default(),
        bioacoustic_syntax: None,
    };
    // Two well-separated timbres so the clustering has something real to find.
    let (centroid, harmonicity, mfcc) = if kind == 0 {
        (400.0, 0.9, vec![0.0, 5.0, -3.0, 1.0, 0.5, 0.2])
    } else {
        (6000.0, 0.1, vec![0.0, -6.0, 4.0, -2.0, -1.0, 0.8])
    };
    p.spectral_features.spectral_centroid_hz = centroid;
    p.spectral_features.spectral_rolloff_hz = centroid * 2.0;
    p.spectral_features.harmonicity = harmonicity;
    p.spectral_features.spectral_flatness = 1.0 - harmonicity;
    p.spectral_features.mel_frequency_cepstral_coefficients = mfcc;
    p.envelope.envelope_temporal_centroid = Some(0.3 + 0.2 * kind as f64);
    Region {
        index,
        start_seconds: start,
        end_seconds: start + duration,
        duration_seconds: duration,
        peak_amplitude: 0.8,
        name: String::new(),
        analysis: Some(Box::new(p)),
    }
}

/// Regions laid out on a fixed period, alternating between `kinds`.
pub(super) fn sequence_of(kinds: &[usize], duration: f64, gap: f64) -> Regions {
    let regions: Vec<Region> = kinds
        .iter()
        .enumerate()
        .map(|(i, &k)| region(i, i as f64 * (duration + gap), duration, k))
        .collect();
    Regions { count: regions.len(), regions, ..Default::default() }
}

/// A flat, quiet envelope covering `seconds` at 200 fps, with the regions
/// themselves loud — enough for the junction measurements to run.
pub(super) fn envelope_for(regions: &Regions, seconds: f64) -> (Vec<f64>, f64) {
    let rate = 200.0;
    let frames = (seconds * rate).ceil() as usize;
    let mut v = vec![0.0005; frames];
    for r in &regions.regions {
        let a = (r.start_seconds * rate) as usize;
        let b = ((r.end_seconds * rate) as usize).min(frames);
        for x in v.iter_mut().take(b).skip(a) {
            *x = 0.8;
        }
    }
    (v, rate)
}

pub(super) fn analyze(regions: &Regions, seconds: f64) -> BioacousticSyntax {
    let (env, rate) = envelope_for(regions, seconds);
    bioacoustic_syntax(regions, &[], 44100.0, 2048, 512, &env, rate).expect("syntax")
}
