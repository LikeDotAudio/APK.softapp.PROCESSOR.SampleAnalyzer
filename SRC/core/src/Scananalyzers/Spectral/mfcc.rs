//! Mel-Frequency Cepstral Coefficients — the standard compact "timbral
//! fingerprint". The power spectrum of each frame is pooled through a bank of
//! triangular filters spaced on the Mel scale (a human-hearing frequency
//! warp), log-compressed, and decorrelated with a DCT-II. The per-frame
//! coefficient vectors are averaged into one fingerprint per file.

pub const N_MEL: usize = 26; // triangular mel filters
pub const N_COEF: usize = 13; // cepstral coefficients kept

fn hz_to_mel(f: f64) -> f64 {
    2595.0 * (1.0 + f / 700.0).log10()
}

fn mel_to_hz(m: f64) -> f64 {
    700.0 * (10f64.powf(m / 2595.0) - 1.0)
}

/// Mean MFCC vector (`N_COEF` values) over Hann-windowed STFT magnitude
/// frames. Empty when there are no frames. `c0` (overall log-energy) is
/// included as the first coefficient.
pub fn mfcc_mean(frames: &[Vec<f32>], sr_f: f64, n_fft: usize) -> Vec<f64> {
    if frames.is_empty() || n_fft < 4 {
        return Vec::new();
    }
    let half = n_fft / 2;
    let bin_hz = sr_f / n_fft as f64;

    // Filter-bank edges: N_MEL + 2 points evenly spaced in mel between 0 Hz
    // and Nyquist, converted back to bin indices.
    let mel_max = hz_to_mel(sr_f / 2.0);
    let edges: Vec<f64> = (0..N_MEL + 2)
        .map(|i| mel_to_hz(mel_max * i as f64 / (N_MEL + 1) as f64) / bin_hz)
        .collect();

    // Accumulate log mel energies over all frames.
    let mut mean_log = [0.0f64; N_MEL];
    for frame in frames {
        for m in 0..N_MEL {
            let (lo, ctr, hi) = (edges[m], edges[m + 1], edges[m + 2]);
            let mut e = 0.0f64;
            let k0 = lo.floor().max(0.0) as usize;
            let k1 = (hi.ceil() as usize).min(half.saturating_sub(1));
            for k in k0..=k1 {
                let kf = k as f64;
                // Triangular weight rising lo→ctr then falling ctr→hi.
                let w = if kf < ctr {
                    (kf - lo) / (ctr - lo).max(1e-9)
                } else {
                    (hi - kf) / (hi - ctr).max(1e-9)
                };
                if w > 0.0 {
                    let p = frame[k] as f64;
                    e += w * p * p; // power spectrum
                }
            }
            mean_log[m] += (e + 1e-10).ln();
        }
    }
    let nf = frames.len() as f64;
    for v in mean_log.iter_mut() {
        *v /= nf;
    }

    // DCT-II of the mean log-energies → cepstral coefficients.
    let scale = (2.0 / N_MEL as f64).sqrt();
    (0..N_COEF)
        .map(|k| {
            let s: f64 = mean_log
                .iter()
                .enumerate()
                .map(|(m, &e)| e * (std::f64::consts::PI * k as f64 * (m as f64 + 0.5) / N_MEL as f64).cos())
                .sum();
            s * scale
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{mfcc_mean, N_COEF};
    use crate::stft::stft_mags;

    fn sine(freq: f32, secs: f32) -> Vec<f32> {
        let sr = 44_100.0f32;
        (0..(sr * secs) as usize)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sr).sin())
            .collect()
    }

    #[test]
    fn fingerprint_shape_and_discrimination() {
        let low = mfcc_mean(&stft_mags(&sine(110.0, 0.5), 2048, 512), 44_100.0, 2048);
        let high = mfcc_mean(&stft_mags(&sine(3520.0, 0.5), 2048, 512), 44_100.0, 2048);
        assert_eq!(low.len(), N_COEF);
        assert!(low.iter().all(|v| v.is_finite()));
        // Very different spectra must give clearly different fingerprints.
        let dist: f64 = low.iter().zip(&high).map(|(a, b)| (a - b).powi(2)).sum::<f64>().sqrt();
        assert!(dist > 1.0, "MFCC distance = {}", dist);
    }
}
