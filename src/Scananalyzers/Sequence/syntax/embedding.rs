//! The acoustic vector each slice is clustered and laid out in.
//!
//! One vector per slice, read off the full analysis the pipeline already ran on
//! that slice, then scaled so distances mean something. The scaling is the part
//! that matters — see `SLICE_FEATURE_FLOORS`.

/// How many MFCC coefficients join the slice vector.
pub(super) const MFCC_IN_SLICE_VECTOR: usize = 5;

/// The smallest difference in each feature that means anything — one
/// just-noticeable difference, in that feature's own units and in the same order
/// as `slice_vector` builds them.
///
/// These exist because plain z-scoring is actively wrong here. Standardizing by
/// the within-file spread divides by whatever variation happens to be present,
/// so when a file holds eight copies of ONE sound the only variation left is the
/// segmenter's frame quantization — region edges land on envelope frames, so
/// durations come out in discrete steps — and z-scoring blows that up to unit
/// variance and hands the clusterer two beautifully tight, entirely fictional
/// groups. (It did exactly that: eight identical snares read as "AAABBAAB".)
///
/// Scaling by `max(observed spread, floor)` instead means a feature that barely
/// varies stays barely varied. Nothing is manufactured, and a file whose slices
/// really are all the same lands in one cluster, which is the truth.
pub(super) const SLICE_FEATURE_FLOORS: [f64; 15 + MFCC_IN_SLICE_VECTOR] = [
    0.10, // ln duration — ~10 % longer or shorter
    0.10, // ln spectral centroid — ~10 % brighter
    0.10, // ln rolloff
    0.10, // ln zero-crossing rate
    0.05, // spectral flatness (0..1)
    0.05, // harmonicity (0..1)
    // Inharmonicity is coarser than the rest and its floor has to say so: it
    // rides on discrete partial peak-picking, so a region edge landing one frame
    // over flips a partial in or out and the value jumps. Measured on bit-
    // identical copies of one chirp segmented at slightly different offsets it
    // moved by 0.10 — so anything under that is the peak-picker twitching, not a
    // difference in the sound. At the old 0.05 it single-handedly supplied 84 %
    // of the distance that split those identical copies into two "types".
    0.15, // inharmonicity (0..1)
    0.03, // low-band energy share
    0.03, // mid-band energy share
    0.03, // high-band energy share
    0.10, // ln pitch — well under a semitone (0.058)
    0.05, // envelope temporal centroid (0..1)
    0.02, // ln attack — ≈20 ms, the audible edge between "plucked" and "bowed"
    0.05, // envelope sustain level (0..1)
    0.50, // crest factor (1..~20)
    0.75, 0.75, 0.75, 0.75, 0.75, // MFCC 1..5 — a perceptible timbre step
];

/// Center each column and divide by the larger of its spread and its
/// just-noticeable difference. The same geometry feeds the clustering, the
/// medoid search and the 2-D layout, so the map the UI draws is the space the
/// types were decided in.
pub(super) fn scale_slices(feats: &[Vec<f64>]) -> Vec<Vec<f64>> {
    if feats.is_empty() {
        return Vec::new();
    }
    let n = feats.len() as f64;
    let d = feats[0].len();
    let mean: Vec<f64> = (0..d).map(|j| feats.iter().map(|f| f[j]).sum::<f64>() / n).collect();
    let scale: Vec<f64> = (0..d)
        .map(|j| {
            let variance =
                feats.iter().map(|f| (f[j] - mean[j]).powi(2)).sum::<f64>() / n;
            variance.sqrt().max(SLICE_FEATURE_FLOORS.get(j).copied().unwrap_or(1.0))
        })
        .collect();
    feats
        .iter()
        .map(|f| (0..d).map(|j| (f[j] - mean[j]) / scale[j]).collect())
        .collect()
}

/// The acoustic vector for one slice, read off the full analysis the pipeline
/// already ran on that slice. This is deliberately NOT `feature_vec::feature_vec`:
/// that vector is tuned to separate whole files across a library (it leans on
/// loudness and length), while this one has to separate syllables inside ONE
/// recording, where absolute level is a property of the microphone distance and
/// says nothing about which syllable was sung. Timbre and shape carry it.
pub(super) fn slice_vector(region: &crate::peak::Region) -> Option<Vec<f64>> {
    let p = region.analysis.as_ref()?;
    let s = &p.spectral_features;
    let e = &p.envelope;
    // The ADSR terms are peak-relative and so are null at file level whenever a
    // slice happens to hold more than one transient. Fall back to that slice's
    // loudest event, the same reading `feature_vec` takes: a real measurement of
    // one event beats a 0.0 that would herd every multi-event slice into the
    // same corner of the space and cluster them by their nullness.
    let event = e.representative_slice();
    let temporal_centroid = e
        .envelope_temporal_centroid
        .or_else(|| event.map(|x| x.envelope_temporal_centroid))
        .unwrap_or(0.0);
    let attack_seconds = e
        .envelope_attack_seconds
        .or_else(|| event.map(|x| x.envelope_attack_seconds))
        .unwrap_or(0.0);
    let sustain_level = e
        .envelope_sustain_level
        .or_else(|| event.map(|x| x.envelope_sustain_level))
        .unwrap_or(0.0);
    let mut v = vec![
        (1.0 + p.metadata.length_seconds).ln(),
        (1.0 + s.spectral_centroid_hz).ln(),
        (1.0 + s.spectral_rolloff_hz).ln(),
        (1.0 + s.zero_crossings_per_second).ln(),
        s.spectral_flatness,
        s.harmonicity,
        s.inharmonicity,
        s.low_band_energy,
        s.mid_band_energy,
        s.high_band_energy,
        (1.0 + p.musicality.pitch_hz).ln(),
        temporal_centroid,
        (1.0 + attack_seconds).ln(),
        sustain_level,
        s.crest_factor,
    ];
    // MFCC 1..5 — the timbral fingerprint, and the closest thing this pipeline
    // has to the learned embedding an auto-encoder would produce. c0 is skipped
    // for the same reason level is: it is loudness.
    for j in 1..=MFCC_IN_SLICE_VECTOR {
        v.push(s.mel_frequency_cepstral_coefficients.get(j).copied().unwrap_or(0.0));
    }
    debug_assert_eq!(v.len(), SLICE_FEATURE_FLOORS.len(), "floors must cover every feature");
    if v.iter().any(|x| !x.is_finite()) {
        return None;
    }
    Some(v)
}
