//! Spectral flux — how fast the spectrum changes frame-to-frame. Low for a
//! steady oscillator/drone, high for evolving or noisy material. A big part of
//! how the ear identifies an instrument's onset character.

/// Mean normalized positive spectral flux over consecutive STFT frames.
/// Each frame is L1-normalized first (so loudness changes don't count, only
/// changes in spectral *shape*), then the positive bin-wise increases are
/// summed. Result is 0 (static spectrum) … ~1 (completely new spectrum every
/// frame). 0 when there are fewer than two frames.
pub fn spectral_flux(frames: &[Vec<f32>]) -> f64 {
    if frames.len() < 2 {
        return 0.0;
    }
    let norm = |f: &[f32]| -> Option<Vec<f64>> {
        let s: f64 = f.iter().map(|&v| v as f64).sum();
        if s > 1e-9 {
            Some(f.iter().map(|&v| v as f64 / s).collect())
        } else {
            None // silent frame — skip the pair rather than divide by ~0
        }
    };
    let mut total = 0.0f64;
    let mut pairs = 0usize;
    let mut prev = norm(&frames[0]);
    for f in &frames[1..] {
        let cur = norm(f);
        if let (Some(a), Some(b)) = (&prev, &cur) {
            total += a.iter().zip(b).map(|(x, y)| (y - x).max(0.0)).sum::<f64>();
            pairs += 1;
        }
        prev = cur;
    }
    if pairs > 0 {
        total / pairs as f64
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::spectral_flux;
    use crate::stft::stft_mags;

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
