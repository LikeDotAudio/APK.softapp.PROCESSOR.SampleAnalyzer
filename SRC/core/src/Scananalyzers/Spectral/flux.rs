//! Spectral flux — how fast the spectrum changes frame-to-frame. Low for a
//! steady oscillator/drone, high for evolving or noisy material. A big part of
//! how the ear identifies an instrument's onset character.
use crate::stft::Stft;

/// Mean normalized positive spectral flux over consecutive STFT frames.
/// Each frame is L1-normalized first (so loudness changes don't count, only
/// changes in spectral *shape*), then the positive bin-wise increases are
/// summed. Result is 0 (static spectrum) … ~1 (completely new spectrum every
/// frame). 0 when there are fewer than two frames.
pub fn spectral_flux(frames: &[Vec<f32>]) -> f64 {
    if frames.len() < 2 {
        return 0.0;
    }
    flux_over_pairs(frames.iter().zip(&frames[1..]))
}

/// Spectral flux of a whole-file transform. On a long file whose frames were
/// sampled across its length, each kept frame is paired with its neighbour one
/// analysis hop later (`Stft::next`), so the measure stays frame-to-next-frame
/// and comparable with a short file's.
pub fn spectral_flux_of(stft: &Stft) -> f64 {
    if stft.next.is_empty() {
        spectral_flux(&stft.frames)
    } else {
        flux_over_pairs(stft.frames.iter().zip(&stft.next))
    }
}

/// Mean positive flux over `(earlier, later)` frame pairs; a pair with a
/// silent frame is skipped rather than divided by ~0.
fn flux_over_pairs<'a>(pairs: impl Iterator<Item = (&'a Vec<f32>, &'a Vec<f32>)>) -> f64 {
    let norm = |f: &[f32]| -> Option<Vec<f64>> {
        let s: f64 = f.iter().map(|&v| v as f64).sum();
        (s > 1e-9).then(|| f.iter().map(|&v| v as f64 / s).collect())
    };
    let mut total = 0.0f64;
    let mut counted = 0usize;
    for (a, b) in pairs {
        if let (Some(a), Some(b)) = (norm(a), norm(b)) {
            total += a.iter().zip(&b).map(|(x, y)| (y - x).max(0.0)).sum::<f64>();
            counted += 1;
        }
    }
    if counted > 0 {
        total / counted as f64
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::{spectral_flux, spectral_flux_of};
    use crate::stft::{stft, stft_mags};

    fn noise(n: usize) -> Vec<f32> {
        let mut s = 0x12345678u32;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s as f32 / u32::MAX as f32) * 2.0 - 1.0
            })
            .collect()
    }

    #[test]
    fn a_strided_long_file_measures_the_same_frame_to_frame_flux() {
        // A 90 s file is sampled across its length; its flux still compares
        // neighbouring analysis frames, so it matches a 10 s file of the same noise.
        let long = stft(&noise(44_100 * 90), 2048, 512);
        assert!(long.hop > 512 && !long.next.is_empty());
        let f_long = spectral_flux_of(&long);
        let f_short = spectral_flux_of(&stft(&noise(44_100 * 10), 2048, 512));
        assert!((f_long - f_short).abs() < 0.05 * f_short, "long {f_long} vs short {f_short}");
    }

    #[test]
    fn steady_sine_has_low_flux_noise_has_more() {
        let sr = 44_100.0f32;
        let sine: Vec<f32> = (0..44_100)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr).sin())
            .collect();
        // Deterministic pseudo-noise (xorshift).
        let mut s = 0x12345678u32;
        let noise: Vec<f32> = (0..44_100)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s as f32 / u32::MAX as f32) * 2.0 - 1.0
            })
            .collect();
        let f_sine = spectral_flux(&stft_mags(&sine, 2048, 512));
        let f_noise = spectral_flux(&stft_mags(&noise, 2048, 512));
        assert!(f_sine < 0.1, "steady sine flux = {}", f_sine);
        assert!(f_noise > f_sine * 3.0, "noise {} vs sine {}", f_noise, f_sine);
    }
}
