//! What sits BETWEEN two slices, measured and classified.
//!
//! The gap is not nothing. It has a level, a spectrum and a slope, and its shape
//! says whether the two slices are separate utterances or one gesture: a decaying
//! tail means the first sound is still ringing into the second, a rising gap is a
//! breath or a wind-up, a flat gap well above the noise floor is a bed that never
//! stops. TweetyNet (Cohen 2022) makes the same move by giving its network an
//! explicit "background" class rather than treating the quiet as absence.

use super::mathutil::ratio_decibels;
use super::record::Junction;
use super::tunables::{
    AUDIBLE_AT_ONSET_DECIBELS_FULL_SCALE, BOUND_WITHIN_DECIBELS, DIGITAL_SILENCE,
    JUNCTION_CHANGE_DECIBELS, ONSET_APPROACH_FRACTION,
};

pub const SILENCE: &str = "Silence";
pub const NOISE_BED: &str = "Noise Bed";
pub const RESONANT_TAIL: &str = "Resonant Tail";
pub const BREATH: &str = "Breath";
pub const CONTINUOUS: &str = "Continuous";

#[allow(clippy::too_many_arguments)]
pub(super) fn measure_junction(
    index: usize,
    previous: &crate::peak::Region,
    next: &crate::peak::Region,
    from_label: &str,
    to_label: &str,
    transition_probability: f64,
    noise_floor: f64,
    frames: &[Vec<f32>],
    sr_f: f64,
    n_fft: usize,
    hop: usize,
    envelope: &[f64],
    envelope_rate_hz: f64,
) -> Junction {
    let gap_start = previous.end_seconds;
    let gap_end = next.start_seconds;
    let gap_seconds = (gap_end - gap_start).max(0.0);
    let inter_onset_seconds = (next.start_seconds - previous.start_seconds).max(0.0);

    // Level and slope come off the RMS envelope (~200 fps) — fine enough to see
    // a tail fall inside a 150 ms gap, which the 86 fps STFT is not.
    let first = (gap_start * envelope_rate_hz).round().max(0.0) as usize;
    let last = ((gap_end * envelope_rate_hz).round().max(0.0) as usize).min(envelope.len());
    let window = if last > first { &envelope[first..last] } else { &[][..] };

    let level = if window.is_empty() {
        0.0
    } else {
        (window.iter().map(|v| v * v).sum::<f64>() / window.len() as f64).sqrt()
    };
    let slope = decibel_slope(window, envelope_rate_hz);
    let below_previous_peak = ratio_decibels(level, previous.peak_amplitude);
    let above_noise_floor = if noise_floor > DIGITAL_SILENCE {
        Some(ratio_decibels(level, noise_floor))
    } else {
        None
    };

    // Spectrum of the residue: is the quiet bright hiss, low rumble, or a tone?
    let (centroid, flatness) = gap_spectrum(frames, gap_start, gap_end, sr_f, n_fft, hop);

    // What is still sounding as the next slice arrives. This, not the gap's
    // average, is what decides whether the two slices are joined by anything: a
    // release that has fallen to nothing long before the next onset binds them
    // no more tightly than a hard edit does, and treating it as a tail would
    // stamp "Resonant Tail" on every ordinary one-shot library.
    let approach_level = if window.is_empty() {
        0.0
    } else {
        let take = ((window.len() as f64 * ONSET_APPROACH_FRACTION).ceil() as usize).max(1);
        let approach = &window[window.len() - take.min(window.len())..];
        (approach.iter().map(|v| v * v).sum::<f64>() / approach.len() as f64).sqrt()
    };
    let sounding_at_onset =
        20.0 * approach_level.max(1e-9).log10() >= AUDIBLE_AT_ONSET_DECIBELS_FULL_SCALE;

    // Classify. Order matters: "the sound never stopped" outranks everything,
    // then "nothing reaches the next onset", and only what is left gets read for
    // its shape.
    let change_decibels = slope * gap_seconds;
    let junction_class = if window.is_empty() {
        SILENCE
    } else if below_previous_peak > -BOUND_WITHIN_DECIBELS {
        CONTINUOUS
    } else if !sounding_at_onset {
        SILENCE
    } else if change_decibels <= -JUNCTION_CHANGE_DECIBELS {
        RESONANT_TAIL
    } else if change_decibels >= JUNCTION_CHANGE_DECIBELS {
        BREATH
    } else {
        NOISE_BED
    };

    Junction {
        index,
        from_region_index: previous.index,
        to_region_index: next.index,
        from_label: from_label.to_string(),
        to_label: to_label.to_string(),
        gap_seconds,
        inter_onset_seconds,
        gap_root_mean_square_level: level,
        gap_level_below_previous_peak_decibels: below_previous_peak,
        gap_level_above_noise_floor_decibels: above_noise_floor,
        gap_slope_decibels_per_second: slope,
        gap_spectral_centroid_hz: centroid,
        gap_spectral_flatness: flatness,
        junction_class: junction_class.to_string(),
        transition_probability,
        // `max(0.0)` rather than plain negation: a certain transition has
        // probability 1, and −log2(1) is −0.0, which serializes as "-0.0".
        surprisal_bits: if transition_probability > 0.0 {
            (-transition_probability.log2()).max(0.0)
        } else {
            0.0
        },
    }
}

/// Least-squares slope of the envelope in dB against time in seconds. The dB
/// floor is set well under any real noise floor so a digitally silent frame
/// contributes a finite value instead of −∞ dragging the fit to nonsense.
fn decibel_slope(window: &[f64], rate_hz: f64) -> f64 {
    if window.len() < 3 || rate_hz <= 0.0 {
        return 0.0;
    }
    let decibels: Vec<f64> = window.iter().map(|&v| 20.0 * (v.max(1e-9)).log10()).collect();
    let n = decibels.len() as f64;
    let mean_t = (n - 1.0) / 2.0 / rate_hz;
    let mean_d = decibels.iter().sum::<f64>() / n;
    let mut numerator = 0.0;
    let mut denominator = 0.0;
    for (i, &d) in decibels.iter().enumerate() {
        let t = i as f64 / rate_hz - mean_t;
        numerator += t * (d - mean_d);
        denominator += t * t;
    }
    if denominator <= 1e-12 {
        0.0
    } else {
        numerator / denominator
    }
}

/// Centroid and flatness of whatever is sounding inside the gap, averaged over
/// the STFT frames whose centers fall in it. Reuses the pipeline's own STFT —
/// no second transform.
fn gap_spectrum(
    frames: &[Vec<f32>],
    gap_start: f64,
    gap_end: f64,
    sr_f: f64,
    n_fft: usize,
    hop: usize,
) -> (f64, f64) {
    if frames.is_empty() || hop == 0 || sr_f <= 0.0 || n_fft < 2 {
        return (0.0, 0.0);
    }
    let bin_hz = sr_f / n_fft as f64;

    // Prefer frames whose whole ANALYSIS WINDOW lies inside the gap. An STFT
    // window is 46 ms at 44.1 kHz, so a frame merely *centered* just inside the
    // gap still has the previous slice's tail in it, and the "spectrum of the
    // silence" would really be a smeared copy of the sound before it. Gaps are
    // at least 150 ms by construction, so containment normally holds; the
    // center-based pass is the fallback for the rare gap too short to fit one.
    let contained = |f: usize| {
        let start = (f * hop) as f64 / sr_f;
        let end = ((f * hop + n_fft) as f64) / sr_f;
        start >= gap_start && end <= gap_end
    };
    let centered = |f: usize| {
        let t = ((f * hop) as f64 + n_fft as f64 / 2.0) / sr_f;
        t >= gap_start && t <= gap_end
    };

    let mut sum = vec![0.0f64; frames[0].len()];
    let mut used = 0usize;
    for (f, frame) in frames.iter().enumerate() {
        if !contained(f) {
            continue;
        }
        for (b, &m) in frame.iter().enumerate() {
            sum[b] += m as f64;
        }
        used += 1;
    }
    if used == 0 {
        for (f, frame) in frames.iter().enumerate() {
            if !centered(f) {
                continue;
            }
            for (b, &m) in frame.iter().enumerate() {
                sum[b] += m as f64;
            }
            used += 1;
        }
    }
    if used == 0 {
        return (0.0, 0.0);
    }
    let magnitudes: Vec<f64> = sum.iter().map(|v| v / used as f64).collect();
    let total: f64 = magnitudes.iter().sum();
    if total <= 1e-12 {
        return (0.0, 0.0);
    }
    let centroid = magnitudes
        .iter()
        .enumerate()
        .map(|(b, m)| b as f64 * bin_hz * m)
        .sum::<f64>()
        / total;
    // Flatness = geometric mean / arithmetic mean. 1 is white noise, →0 a tone.
    let n = magnitudes.len() as f64;
    let log_sum: f64 = magnitudes.iter().map(|m| m.max(1e-12).ln()).sum();
    let flatness = (log_sum / n).exp() / (total / n);
    (centroid, flatness.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::super::bioacoustic_syntax;
    use super::super::fixtures::{analyze, sequence_of};
    use super::*;

    #[test]
    fn every_gap_becomes_one_classified_junction() {
        let r = sequence_of(&[0, 1, 0, 1], 0.2, 0.3);
        let s = analyze(&r, 2.5);
        assert_eq!(s.junctions.len(), 3, "one junction per gap");
        for j in &s.junctions {
            assert!((j.gap_seconds - 0.3).abs() < 0.02, "gap {}", j.gap_seconds);
            assert!((j.inter_onset_seconds - 0.5).abs() < 0.02);
            assert!(!j.junction_class.is_empty());
        }
        assert_eq!(s.junction_profile.iter().map(|c| c.count).sum::<usize>(), 3);
    }

    #[test]
    fn a_sounding_flat_gap_reads_as_a_noise_bed() {
        let r = sequence_of(&[0, 0], 0.2, 0.4);
        // The two slices sit at 0..0.2 s and 0.6..0.8 s (frames 0..40 and
        // 120..160 at 200 fps). Between them, a steady −50 dBFS bed: audible
        // when the next slice arrives, and going neither up nor down.
        let env: Vec<f64> = (0..200)
            .map(|i| if i < 40 || (120..160).contains(&i) { 0.8 } else { 0.003 })
            .collect();
        let s = bioacoustic_syntax(&r, &[], 44100.0, 2048, 512, &env, 200.0).expect("syntax");
        assert_eq!(s.junctions[0].junction_class, NOISE_BED);
        // A bed is not a bond — the two slices are still separate events.
        assert_eq!(s.bound_ratio, 0.0);
    }

    #[test]
    fn a_quiet_flat_gap_reads_as_silence_not_as_a_bed() {
        let r = sequence_of(&[0, 0], 0.2, 0.4);
        let s = analyze(&r, 1.0);
        assert_eq!(s.junctions[0].junction_class, SILENCE);
        assert!(s.junctions[0].gap_level_below_previous_peak_decibels < -50.0);
    }

    #[test]
    fn a_falling_gap_reads_as_a_resonant_tail() {
        let mut r = sequence_of(&[0, 0], 0.2, 0.6);
        r.regions[1].start_seconds = 0.8;
        r.regions[1].end_seconds = 1.0;
        let rate = 200.0;
        let mut env = vec![0.0005; 220];
        for x in env.iter_mut().take(40) {
            *x = 0.8;
        }
        // The gap starts loud and fades — the first sound ringing out.
        for (i, x) in env.iter_mut().enumerate().take(160).skip(40) {
            *x = 0.4 * (0.02f64).powf((i - 40) as f64 / 120.0);
        }
        for x in env.iter_mut().take(200).skip(160) {
            *x = 0.8;
        }
        let s = bioacoustic_syntax(&r, &[], 44100.0, 2048, 512, &env, rate).expect("syntax");
        assert_eq!(s.junctions[0].junction_class, RESONANT_TAIL);
        assert!(s.junctions[0].gap_slope_decibels_per_second < 0.0);
        assert!(s.bound_ratio > 0.0);
    }

    #[test]
    fn a_rising_gap_reads_as_a_breath() {
        let mut r = sequence_of(&[0, 0], 0.2, 0.6);
        r.regions[1].start_seconds = 0.8;
        r.regions[1].end_seconds = 1.0;
        let rate = 200.0;
        let mut env = vec![0.0005; 220];
        for x in env.iter_mut().take(40) {
            *x = 0.8;
        }
        // Level climbs through the gap — an inhale, or a wind-up into the onset.
        for (i, x) in env.iter_mut().enumerate().take(160).skip(40) {
            *x = 0.0002 * (50.0f64).powf((i - 40) as f64 / 120.0);
        }
        for x in env.iter_mut().take(200).skip(160) {
            *x = 0.8;
        }
        let s = bioacoustic_syntax(&r, &[], 44100.0, 2048, 512, &env, rate).expect("syntax");
        assert_eq!(s.junctions[0].junction_class, BREATH);
        assert!(s.junctions[0].gap_slope_decibels_per_second > 0.0);
    }

    #[test]
    fn a_gap_that_never_drops_reads_as_continuous() {
        let r = sequence_of(&[0, 0], 0.2, 0.4);
        let rate = 200.0;
        // The gate dipped, the sound did not stop: the "gap" is 6 dB down.
        let env = vec![0.4; 200];
        let s = bioacoustic_syntax(&r, &[], 44100.0, 2048, 512, &env, rate).expect("syntax");
        assert_eq!(s.junctions[0].junction_class, CONTINUOUS);
        assert_eq!(s.bound_ratio, 1.0);
    }
}
