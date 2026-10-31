use ebur128::{EbuR128, Mode};

pub fn analyze_stereo_width(raw_interleaved: &[f32], channels: u16) -> (f64, f64) {
    if raw_interleaved.is_empty() { return (0.0, 0.0); }
    let mut mid_energy = 0.0;
    let mut side_energy = 0.0;
    
    let ch = channels as usize;
    let mut frames = 0;

    for frame in raw_interleaved.chunks(ch) {
        if frame.is_empty() { continue; }
        
        let l = frame[0];
        let r = if ch > 1 && frame.len() > 1 { frame[1] } else { l };
        
        let mid = (l + r) / 2.0;
        let side = (l - r) / 2.0;
        
        mid_energy += (mid * mid) as f64;
        side_energy += (side * side) as f64;
        frames += 1;
    }
    
    if frames == 0 { return (0.0, 0.0); }
    
    let mid_rms = (mid_energy / frames as f64).sqrt();
    let side_rms = (side_energy / frames as f64).sqrt();
    
    (mid_rms, side_rms)
}

pub fn get_integrated_lufs(raw_interleaved: &[f32], channels: u16, sample_rate: u32) -> f64 {
    if raw_interleaved.is_empty() { return -70.0; }
    let ch = channels as u32;
    let ebu_ch = if ch > 2 { 2 } else { ch };
    let mut analyzer = match EbuR128::new(ebu_ch, sample_rate, Mode::I | Mode::TRUE_PEAK) {
        Ok(a) => a,
        Err(_) => return -70.0,
    };
    
    let frames = if ch > 2 {
        let mut two_ch = Vec::with_capacity(raw_interleaved.len() / ch as usize * 2);
        for frame in raw_interleaved.chunks(ch as usize) {
            if frame.len() >= 2 {
                two_ch.push(frame[0]);
                two_ch.push(frame[1]);
            }
        }
        two_ch
    } else {
        raw_interleaved.to_vec()
    };
    
    if analyzer.add_frames_f32(&frames).is_err() {
        return -70.0;
    }
    
    // A silent file measures as −∞, which serde_json writes as `null` — and a null
    // is not an f64 on the way back in. Fold every non-finite result onto the −70
    // "unmeasurable" sentinel so the record we write is a record we can read.
    let lufs = analyzer.loudness_global().unwrap_or(-70.0);
    if lufs.is_finite() { lufs } else { -70.0 }
}

/// The onset envelope: rising energy per 512-sample frame, i.e. where the
/// attacks are. This is an intermediate signal, not a field — it is thousands of
/// floats per file and the record keeps only what it *asks* of it, which is the
/// single number below.
pub fn detect_transient_onsets(audio: &[f32], frame_size: usize) -> Vec<f64> {
    let mut onset_envelope = Vec::new();
    let mut previous_energy = 0.0;

    for frame in audio.chunks(frame_size) {
        let current_energy: f64 = frame.iter().map(|&s| (s as f64) * (s as f64)).sum();
        let flux = (current_energy - previous_energy).max(0.0);
        onset_envelope.push(flux);
        previous_energy = current_energy;
    }

    onset_envelope
}

/// Peak of the normalized autocorrelation of the onset envelope: 1 = a perfectly
/// periodic onset train (a clock, a gallop, a footstep loop), 0 = stochastic
/// (rain, applause). This is the whole of what the record wants from the
/// envelope, so we reduce here and store the scalar.
///
/// None when the envelope is too short to hold a period, or is perfectly flat —
/// and None must reach the UCS scorer as "no evidence", never as 0.0, which
/// would be the *positive* claim that the onsets are stochastic.
pub fn onset_periodicity(envelope: &[f64]) -> Option<f64> {
    if envelope.len() < 8 {
        return None;
    }
    let mean = envelope.iter().sum::<f64>() / envelope.len() as f64;
    let x: Vec<f64> = envelope.iter().map(|v| v - mean).collect();
    let denom: f64 = x.iter().map(|v| v * v).sum();
    if denom <= 1e-12 {
        return None;
    }
    let max_lag = envelope.len() / 2;
    let mut best: f64 = 0.0;
    for lag in 2..max_lag {
        let r: f64 = x[..x.len() - lag]
            .iter()
            .zip(&x[lag..])
            .map(|(a, b)| a * b)
            .sum::<f64>()
            / denom;
        if r > best {
            best = r;
        }
    }
    Some(best.clamp(0.0, 1.0))
}

pub fn calculate_qa_metrics(samples: &[f32], sample_rate: u32) -> (f64, f64) {
    if samples.is_empty() { return (0.0, 0.0); }
    let sum: f64 = samples.iter().map(|&s| s as f64).sum();
    let dc_offset = sum / samples.len() as f64;
    
    let noise_floor = 0.0001;
    let trailing_samples = samples.iter()
        .rev()
        .take_while(|&&s| s.abs() < noise_floor)
        .count();
        
    let trailing_silence_ms = (trailing_samples as f64 / sample_rate as f64) * 1000.0;

    (dc_offset, trailing_silence_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A silent buffer measures as −∞ LUFS, and serde_json writes a non-finite float as
    // `null` — which is not an f64 coming back in. That made every silent sample a
    // sidecar the analyzer could not re-read, so it re-analyzed them on every scan.
    #[test]
    fn silence_measures_as_the_sentinel_not_negative_infinity() {
        let silence = vec![0.0f32; 48_000];
        let lufs = get_integrated_lufs(&silence, 1, 48_000);
        assert!(lufs.is_finite(), "silent file gave a non-finite LUFS: {lufs}");
        assert!(lufs <= -69.0, "silence should read as unmeasurable, got {lufs}");

        // And the record it produces must survive a round trip through JSON.
        let json = serde_json::to_string(&lufs).unwrap();
        assert_ne!(json, "null", "a non-finite LUFS serializes to null");
        assert!(serde_json::from_str::<f64>(&json).is_ok());
    }

    const FRAME: usize = 512;

    fn periodicity_of(audio: &[f32]) -> Option<f64> {
        onset_periodicity(&detect_transient_onsets(audio, FRAME))
    }

    /// The distinction the feature exists to make, and the whole reason the
    /// record keeps this number instead of the envelope it came from: a clock is
    /// periodic, rain is not.
    #[test]
    fn a_regular_onset_train_is_periodic_and_noise_is_not() {
        // A click every 8 frames — a metronome.
        let mut clicks = vec![0.0f32; FRAME * 200];
        for (i, s) in clicks.iter_mut().enumerate() {
            if i % (FRAME * 8) < 32 {
                *s = 1.0;
            }
        }
        let clock = periodicity_of(&clicks).unwrap();
        assert!(clock > 0.8, "a metronome must read as periodic, got {clock}");

        // Stochastic energy: no lag repeats.
        let mut seed = 12_345u32;
        let rain: Vec<f32> = (0..FRAME * 200)
            .map(|_| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                ((seed >> 8) as f32 / 8_388_608.0) - 1.0
            })
            .collect();
        let stochastic = periodicity_of(&rain).unwrap();
        assert!(stochastic < 0.5, "noise must not read as periodic, got {stochastic}");
        assert!(clock > stochastic, "the clock must outrank the noise");
    }

    /// None, not 0.0. A file with no onsets has *no evidence* about periodicity;
    /// reporting 0.0 would be the positive claim that its onsets are stochastic,
    /// and the UCS scorer would weigh that claim against a real prior.
    #[test]
    fn silence_has_no_periodicity_rather_than_zero_periodicity() {
        assert!(periodicity_of(&vec![0.0f32; FRAME * 100]).is_none());
        assert!(periodicity_of(&[]).is_none());
        assert!(periodicity_of(&[0.5f32; 3]).is_none(), "too short to hold a period");
    }
}
