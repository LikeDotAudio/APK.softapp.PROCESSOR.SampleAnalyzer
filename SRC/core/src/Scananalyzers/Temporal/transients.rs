//! Transient (attack) detection by prominence peak-picking on the amplitude
//! envelope. A hit is a rise to a local peak that stands at least `PROM` above
//! the valley preceding it — so each re-attack in a loop counts, while a steady
//! sustain or low-frequency envelope ripple (no real dip-then-rise) does not.
//!
//! Two entry points over the same pass: `count_transients` for the scalar the
//! record has always carried, and `transient_onsets` for WHERE each attack is.
//! The positions are what lets `envelope::envelope_analysis` cut a multi-event
//! file into one slice per hit instead of measuring a single ADSR against the
//! file's loudest peak — which on a multi-event file describes the edit, not
//! the sound.

/// ~16 ms frames — averages out sub-100 Hz ripple without smearing an attack.
fn frame_hop(sr: u32) -> usize {
    (sr as usize / 60).max(1)
}

const PROM: f32 = 0.18; // peak must rise this far above the preceding valley
const MIN_LEVEL: f32 = 0.12; // and reach at least this loudness
const EPS: f32 = 1e-4;

/// The smoothed, peak-normalized amplitude track the peak-picker runs on.
/// None when there is no audible signal at all.
fn onset_track(data: &[f32], sr: u32) -> Option<Vec<f32>> {
    if data.is_empty() {
        return None;
    }
    let hop = frame_hop(sr);
    let mut env: Vec<f32> = Vec::with_capacity(data.len() / hop + 1);
    let mut i = 0;
    while i < data.len() {
        let end = (i + hop).min(data.len());
        let mut s = 0.0f32;
        for &x in &data[i..end] {
            s += x * x;
        }
        env.push((s / (end - i) as f32).sqrt());
        i += hop;
    }
    let n = env.len();
    let emax = env.iter().cloned().fold(0.0f32, f32::max);
    if emax <= 0.0 {
        return None;
    }
    // Normalize + 3-tap smoothing.
    Some(
        (0..n)
            .map(|k| {
                let a = env[k.saturating_sub(1)];
                let b = env[k];
                let c = env[(k + 1).min(n - 1)];
                (a + b + c) / (3.0 * emax)
            })
            .collect(),
    )
}

/// The frame index at which each detected attack STARTS — the valley the rise
/// began from, not the peak it reached. That is the correct cut point: slicing
/// at the peak would put the attack of hit N at the end of slice N−1.
fn onset_frames(sm: &[f32]) -> Vec<usize> {
    let n = sm.len();
    let mut onsets = Vec::new();
    if n < 2 {
        return onsets;
    }
    let mut rising = false;
    let mut valley = sm[0];
    let mut valley_idx = 0usize;
    let mut peak = sm[0];
    for k in 1..n {
        if sm[k] > sm[k - 1] + EPS {
            if !rising {
                valley = sm[k - 1];
                valley_idx = k - 1;
                rising = true;
            }
            peak = sm[k];
        } else if sm[k] < sm[k - 1] - EPS && rising {
            if peak - valley >= PROM && peak >= MIN_LEVEL {
                onsets.push(valley_idx);
            }
            rising = false;
        }
    }
    onsets
}

/// Count transients (attacks). A clean one-shot yields 1; a loop yields many.
pub fn count_transients(data: &[f32], sr: u32) -> usize {
    if data.is_empty() {
        return 0;
    }
    let Some(sm) = onset_track(data, sr) else {
        return 0;
    };
    if sm.len() < 3 {
        return 1; // audible but too short to peak-pick — one event by definition
    }
    onset_frames(&sm).len().max(1) // audible signal ⇒ at least one attack
}

/// Where each attack begins, in seconds from the start of the file.
///
/// Empty means "no distinct attack was found" — a held tone, a fade-in, a bed.
/// That is not the same as one attack at t=0: the caller decides what a sound
/// with no detectable onset should be sliced into (`envelope_analysis` treats
/// it as a single window over the whole file). The count returned by
/// `count_transients` floors at 1, so it can exceed `transient_onsets().len()`
/// by one for exactly this case; the two agree whenever any onset is found.
pub fn transient_onsets(data: &[f32], sr: u32) -> Vec<f64> {
    let Some(sm) = onset_track(data, sr) else {
        return Vec::new();
    };
    let secs_per_frame = frame_hop(sr) as f64 / sr as f64;
    onset_frames(&sm)
        .into_iter()
        .map(|k| k as f64 * secs_per_frame)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 44_100;

    /// `hits` = (start seconds, duration seconds) decaying bursts of 440 Hz.
    fn hits(total: f64, hits: &[(f64, f64)]) -> Vec<f32> {
        let n = (total * SR as f64) as usize;
        (0..n)
            .map(|i| {
                let t = i as f64 / SR as f64;
                let mut lvl = 0.0f64;
                for &(start, dur) in hits {
                    if t >= start && t < start + dur {
                        lvl += (-(t - start) / (dur / 4.0)).exp();
                    }
                }
                (lvl * (2.0 * std::f64::consts::PI * 440.0 * t).sin()) as f32
            })
            .collect()
    }

    #[test]
    fn onsets_land_on_each_hit() {
        // Four hits, 0.5 s apart — the positions must come back, not just "4".
        let audio = hits(2.2, &[(0.1, 0.3), (0.6, 0.3), (1.1, 0.3), (1.6, 0.3)]);
        let onsets = transient_onsets(&audio, SR);
        assert_eq!(onsets.len(), 4, "onsets = {onsets:?}");
        for (found, expected) in onsets.iter().zip([0.1, 0.6, 1.1, 1.6]) {
            assert!(
                (found - expected).abs() < 0.05,
                "onset at {found} s, expected ~{expected} s (all: {onsets:?})"
            );
        }
    }

    #[test]
    fn onset_count_agrees_with_the_scalar() {
        let audio = hits(2.2, &[(0.1, 0.3), (0.6, 0.3), (1.1, 0.3), (1.6, 0.3)]);
        assert_eq!(transient_onsets(&audio, SR).len(), count_transients(&audio, SR));
    }

    #[test]
    fn a_held_tone_has_no_onsets_but_still_counts_as_one_event() {
        let drone: Vec<f32> = (0..SR * 2)
            .map(|i| {
                let t = i as f64 / SR as f64;
                (0.5 * (2.0 * std::f64::consts::PI * 220.0 * t).sin()) as f32
            })
            .collect();
        assert!(transient_onsets(&drone, SR).is_empty());
        assert_eq!(count_transients(&drone, SR), 1);
    }

    #[test]
    fn silence_has_neither() {
        let quiet = vec![0.0f32; SR as usize];
        assert!(transient_onsets(&quiet, SR).is_empty());
        assert_eq!(count_transients(&quiet, SR), 0);
        assert_eq!(count_transients(&[], SR), 0);
    }
}
