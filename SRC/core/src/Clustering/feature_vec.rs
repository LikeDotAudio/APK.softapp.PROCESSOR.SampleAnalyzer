use crate::peak::Peak;

/// How many MFCC coefficients join the feature vector (c1…c5 — c0 is overall
/// log-energy, i.e. loudness, which the RMS level already covers).
const MFCC_IN_VEC: usize = 5;

/// The acoustic feature vector used for blind clustering and the PCA map
/// (name-independent): envelope, spectral, and timbral-fingerprint features.
pub fn feature_vec(p: &Peak) -> Vec<f64> {
    // The ADSR terms are peak-relative, so the file-level fields are null on a
    // multi-event file. k-means needs a number in every column, and dropping the
    // dimension for some rows is not an option — so read the loudest slice, which
    // is a real single-event measurement, rather than substituting 0.0 (which
    // would place every loop at the "no sustain, no skew" corner of the space and
    // cluster loops together by their nullness instead of by their sound).
    let slice = p.envelope.representative_slice();
    let sustain_level = p
        .envelope
        .envelope_sustain_level
        .or_else(|| slice.map(|s| s.envelope_sustain_level))
        .unwrap_or(0.0);
    let skewness = p
        .envelope
        .envelope_skewness
        .or_else(|| slice.map(|s| s.envelope_skewness))
        .unwrap_or(0.0);
    let mut v = vec![
        (1.0 + p.metadata.length_seconds).ln(),
        p.spectral_features.root_mean_square_level,
        p.spectral_features.zero_crossings_per_second,
        p.spectral_features.spectral_centroid_hz,
        p.spectral_features.harmonicity,
        p.spectral_features.low_band_energy,
        p.spectral_features.high_band_energy,
        p.spectral_features.crest_factor,
        p.envelope.attack_seconds,
        sustain_level,
        skewness,
        p.spectral_features.spectral_flux,
        p.spectral_features.inharmonicity,
        p.spectral_features.spectral_centroid_deviation_hz,
        // Distortion metrics: pull "clean vs dirty" versions of otherwise
        // identical sounds (clean guitar vs fuzz guitar) into separate clusters.
        p.spectral_features.total_harmonic_distortion,
        p.spectral_features.clipping_density,
    ];
    for j in 1..=MFCC_IN_VEC {
        v.push(p.spectral_features.mel_frequency_cepstral_coefficients.get(j).copied().unwrap_or(0.0));
    }
    v
}

/// Z-score standardize each feature column to mean 0, standard deviation 1, so no
/// dimension dominates a Euclidean distance (k-means) or a covariance (PCA) just
/// because its raw units are larger — an 8 kHz centroid must not outweigh a 0..1
/// harmonicity. Columns with ~zero variance collapse to 0. Population std (÷n),
/// the convention PCA's correlation-structure decomposition expects.
///
/// This is the single normalization both clustering paths share, so the k-means
/// grouping and the PCA map are laid out on the same geometry. It replaces the
/// old min-max scaling k-means used, which a single outlier could crush.
pub fn standardize(feats: &[Vec<f64>]) -> Vec<Vec<f64>> {
    if feats.is_empty() {
        return Vec::new();
    }
    let n = feats.len() as f64;
    let d = feats[0].len();
    let mean: Vec<f64> = (0..d).map(|j| feats.iter().map(|f| f[j]).sum::<f64>() / n).collect();
    let std: Vec<f64> = (0..d)
        .map(|j| (feats.iter().map(|f| (f[j] - mean[j]).powi(2)).sum::<f64>() / n).sqrt())
        .collect();
    feats
        .iter()
        .map(|f| (0..d).map(|j| if std[j] > 1e-12 { (f[j] - mean[j]) / std[j] } else { 0.0 }).collect())
        .collect()
}
