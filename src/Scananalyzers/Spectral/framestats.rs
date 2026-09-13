//! Frame-wise spectral statistics: how a spectral feature *moves* over the
//! file, summarized by its statistical moments. A single whole-file FFT hides
//! modulation — a wobbling filter sweep and a static tone can share the same
//! average centroid; the frame-wise variance separates them.
use crate::moments::moments;

/// The per-frame spectral centroid as a `(time_seconds, centroid_hz)` track.
///
/// Silent frames have no centroid and are dropped rather than reported as 0 —
/// which is why the track carries its own timestamps instead of being a bare
/// series: the caller cannot assume frame *i* sits at *i · hop*.
pub fn centroid_track(frames: &[Vec<f32>], sr_f: f64, n_fft: usize, hop: usize) -> Vec<(f64, f64)> {
    if frames.is_empty() || n_fft < 2 || sr_f <= 0.0 {
        return Vec::new();
    }
    let bin_hz = sr_f / n_fft as f64;
    let frame_seconds = hop as f64 / sr_f;
    frames
        .iter()
        .enumerate()
        .filter_map(|(fi, frame)| {
            let mut sum_m = 0.0f64;
            let mut sum_fm = 0.0f64;
            for (k, &m) in frame.iter().enumerate() {
                let m = m as f64;
                sum_m += m;
                sum_fm += k as f64 * bin_hz * m;
            }
            if sum_m > 1e-9 {
                Some((fi as f64 * frame_seconds, sum_fm / sum_m))
            } else {
                None // silent frame — no centroid
            }
        })
        .collect()
}

/// Mean and standard deviation of the per-frame spectral centroid (Hz).
/// (0, 0) when there are no usable frames.
pub fn centroid_stats(frames: &[Vec<f32>], sr_f: f64, n_fft: usize) -> (f64, f64) {
    // Hop affects only the timestamps, which this summary discards.
    let per_frame: Vec<f64> = centroid_track(frames, sr_f, n_fft, 1)
        .into_iter()
        .map(|(_, centroid)| centroid)
        .collect();
    let m = moments(&per_frame);
    (m.mean, m.var.sqrt())
}

#[cfg(test)]
mod tests {
    use super::centroid_stats;
    use crate::stft::stft_mags;

    #[test]
    fn static_tone_has_stable_centroid_sweep_does_not() {
        let sr = 44_100.0f32;
        let tone: Vec<f32> = (0..44_100)
            .map(|i| (2.0 * std::f32::consts::PI * 880.0 * i as f32 / sr).sin())
            .collect();
        // 200 Hz → 8 kHz linear chirp: the centroid climbs the whole file.
        let chirp: Vec<f32> = (0..44_100)
            .map(|i| {
                let t = i as f32 / sr;
                let f = 200.0 + (8000.0 - 200.0) * t / 1.0;
                (2.0 * std::f32::consts::PI * f * t).sin()
            })
            .collect();
        let (tm, ts) = centroid_stats(&stft_mags(&tone, 2048, 512), 44_100.0, 2048);
        let (_, cs) = centroid_stats(&stft_mags(&chirp, 2048, 512), 44_100.0, 2048);
        assert!((tm - 880.0).abs() < 200.0, "tone centroid mean = {}", tm);
        assert!(cs > ts * 10.0, "chirp std {} vs tone std {}", cs, ts);
    }
}
