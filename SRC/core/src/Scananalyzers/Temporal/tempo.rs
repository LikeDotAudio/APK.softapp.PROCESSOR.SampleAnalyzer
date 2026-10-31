//! Heuristic tempo (BPM) for loops that carry no embedded ACID tempo tag.
//! Autocorrelation of an onset-strength envelope (positive spectral flux per
//! STFT frame): a loop repeats its rhythmic pattern, so the onset envelope is
//! periodic and the autocorrelation peaks at the beat period.

/// Estimate BPM from STFT magnitude frames. Returns 0.0 when it can't decide.
pub fn estimate_bpm(frames: &[Vec<f32>], sr: f64, hop: usize) -> f64 {
    let n = frames.len();
    if n < 16 || sr <= 0.0 || hop == 0 {
        return 0.0;
    }

    // Onset envelope: summed positive bin-wise increase between frames.
    let mut onset = vec![0.0f64; n];
    for i in 1..n {
        let (prev, cur) = (&frames[i - 1], &frames[i]);
        let m = prev.len().min(cur.len());
        let mut s = 0.0;
        for k in 0..m {
            let d = cur[k] as f64 - prev[k] as f64;
            if d > 0.0 {
                s += d;
            }
        }
        onset[i] = s;
    }

    // Remove the mean so the autocorrelation reflects periodicity, not energy.
    let mean = onset.iter().sum::<f64>() / n as f64;
    for v in onset.iter_mut() {
        *v -= mean;
    }

    let frame_period = hop as f64 / sr; // seconds per frame
    let bpm_to_lag = |bpm: f64| (60.0 / bpm / frame_period).round() as usize;
    let lag_min = bpm_to_lag(200.0).max(1);
    let lag_max = bpm_to_lag(60.0).min(n - 1);
    if lag_max <= lag_min {
        return 0.0;
    }

    // Autocorrelation across the plausible beat-period lags; pick the strongest.
    let mut best_lag = 0usize;
    let mut best = f64::MIN;
    for lag in lag_min..=lag_max {
        let mut acc = 0.0;
        for i in 0..(n - lag) {
            acc += onset[i] * onset[i + lag];
        }
        if acc > best {
            best = acc;
            best_lag = lag;
        }
    }
    if best_lag == 0 || best <= 0.0 {
        return 0.0;
    }

    let mut bpm = 60.0 / (best_lag as f64 * frame_period);
    // Fold octave errors into a musical range.
    while bpm < 70.0 {
        bpm *= 2.0;
    }
    while bpm > 180.0 {
        bpm /= 2.0;
    }
    (bpm * 10.0).round() / 10.0
}
