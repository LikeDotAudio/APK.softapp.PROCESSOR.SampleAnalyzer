//! ADSR-style envelope analysis — how the sound behaves over time.
//!
//! A recorded one-shot has no note-off, so the classic synthesizer ADSR is
//! estimated from the amplitude envelope itself:
//!   Attack  — rise time from 10 % to 90 % of the peak level
//!   Decay   — time from the peak down to the sustain plateau
//!   Sustain — the plateau level after the decay, as a fraction of peak
//!   Release — the final fade from the plateau down to silence
//! plus the temporal centroid (where the energy sits in time: ~0 = front-
//! loaded hit, ~0.5 = held evenly), the envelope's statistical skewness and
//! kurtosis (a strongly positive-skewed envelope IS a percussive sound), and
//! a categorical envelope `shape`.
//!
//! ## Why this is measured per SLICE, not per file
//!
//! Every measurement above is defined against *the* peak. On a file with more
//! than one transient there is no such thing: a 3 s recording of typing whose
//! loudest keypress lands at 2.65 s reports a 2.5 s "attack", because the
//! envelope first crosses 10 % at the first keypress and does not reach 90 %
//! until the loudest one. That number describes where the uploader happened to
//! hit hardest — the edit — not the physics of the sound.
//!
//! So the file is cut at its transient onsets and the ADSR is measured once per
//! slice, each against its OWN peak. `EnvelopeAnalysis::slices` is that array.
//! The file-level scalars are only meaningful when there is exactly one slice;
//! `single_event` says whether they are, and the caller emits null when not.
use crate::moments::moments;

pub struct Envelope {
    pub attack: f64,   // 10 % → 90 % rise time (s)
    pub decay: f64,    // peak → sustain plateau (s)
    pub sustain: f64,  // plateau level, fraction of peak (0..1)
    pub release: f64,  // final fade (plateau → 5 % of peak) (s)
    pub centroid: f64, // temporal energy centroid, 0..1 of window length
    pub skew: f64,     // 3rd moment: high positive ⇒ front-loaded energy
    pub kurt: f64,     // 4th moment (excess): high ⇒ sharp isolated bursts
    pub shape: &'static str,
    /// Ring time: seconds for the post-peak level to fall 60 dB, extrapolated
    /// from the initial decay slope. None when the sound never decays (a held
    /// tone, a steady bed) — which is itself the diagnostic, so it must not be
    /// confused with a decay time of zero. This is the material-impedance axis:
    /// METAL-IMPACT rings, WOOD-IMPACT thuds.
    pub decay_time_60db: Option<f64>,
}

impl Envelope {
    fn silent() -> Envelope {
        Envelope {
            attack: 0.0, decay: 0.0, sustain: 0.0, release: 0.0,
            centroid: 0.0, skew: 0.0, kurt: 0.0, shape: "Silent",
            decay_time_60db: None,
        }
    }
}

/// One transient's worth of the file: an ADSR measured against that slice's own
/// peak, plus where the slice sits. A one-shot produces exactly one of these
/// spanning the whole file.
pub struct EnvelopeSlice {
    pub index: usize,
    pub start_seconds: f64,
    pub end_seconds: f64,
    /// The slice's own peak, as a fraction of the file's loudest frame (0..1).
    /// 1.0 marks the slice the file-level "representative" reading comes from.
    pub relative_level: f64,
    pub envelope: Envelope,
}

/// The whole envelope picture for one file.
pub struct EnvelopeAnalysis {
    /// One entry per transient, in time order. Never empty for audible audio.
    pub slices: Vec<EnvelopeSlice>,
    /// The loudest slice — the best single-event evidence the file contains.
    /// This is what the timbre/family classifiers should read, since they are
    /// asking "what object is vibrating", which is a per-event question.
    pub representative: usize,
    /// True when the file holds exactly one event, i.e. when a file-level ADSR
    /// is a meaningful measurement at all.
    pub single_event: bool,
    /// The file-level categorical shape. Always meaningful: "Multi" is the
    /// honest answer for a multi-event file, where the numbers are not.
    pub shape: &'static str,
}

impl EnvelopeAnalysis {
    /// The loudest slice's measurement.
    pub fn representative(&self) -> &Envelope {
        &self.slices[self.representative].envelope
    }

    /// The file-level ADSR — Some only for a genuine one-shot. On a multi-event
    /// file this is None by construction, which is the whole point: a caller
    /// cannot accidentally write a global-peak-relative number to the record.
    pub fn file_level(&self) -> Option<&Envelope> {
        self.single_event.then(|| self.representative())
    }
}

/// The RMS amplitude track and its sample rate in frames per second.
///
/// ~5 ms frames: fine enough to resolve fast attacks. Shared by the ADSR
/// measurement and by `morphology::syllabic_modulation`, which needs the same
/// track at a known rate — computing it once here keeps the two definitions
/// from drifting apart.
pub fn amplitude_envelope(data: &[f32], sr: u32) -> (Vec<f64>, f64) {
    let hop = (sr as usize / 200).max(1);
    let rate_hz = sr as f64 / hop as f64;
    let mut env: Vec<f64> = Vec::with_capacity(data.len() / hop + 1);
    let mut i = 0;
    while i < data.len() {
        let end = (i + hop).min(data.len());
        let mut s = 0.0f64;
        for &x in &data[i..end] {
            s += x as f64 * x as f64;
        }
        env.push((s / (end - i) as f64).sqrt());
        i += hop;
    }
    (env, rate_hz)
}

/// A slice shorter than this cannot hold an attack and a decay, so an onset
/// this close behind another is folded into the slice already running.
const MIN_SLICE_SECONDS: f64 = 0.03;

/// Measure the envelope of every transient in the file.
///
/// `onsets` are attack start times in seconds (`transients::transient_onsets`).
/// Empty — a held tone, a fade-in, a bed — means one slice over the whole file.
/// Each slice runs from its onset to the next onset: the tail of a hit that is
/// cut off by the next hit really is masked in the audio, so measuring past the
/// boundary would report a decay nobody can hear.
pub fn envelope_analysis(data: &[f32], sr: u32, onsets: &[f64]) -> EnvelopeAnalysis {
    let (env, rate_hz) = amplitude_envelope(data, sr);
    let dt = 1.0 / rate_hz;
    let n = env.len();
    if n == 0 {
        return silent_analysis();
    }
    // 3-tap smoothing to keep single-cycle ripple out of the crossings. Done
    // once over the whole track, then sliced — so a slice boundary does not
    // introduce a smoothing edge the measurement would read as an attack.
    let sm: Vec<f64> = (0..n)
        .map(|k| (env[k.saturating_sub(1)] + env[k] + env[(k + 1).min(n - 1)]) / 3.0)
        .collect();

    let file_peak = sm.iter().cloned().fold(0.0f64, f64::max);
    if file_peak <= 0.0 {
        return silent_analysis();
    }

    // Onset times → frame bounds, dropping onsets too close behind the previous
    // one to be their own event, and any that fall outside the track.
    let mut starts: Vec<usize> = Vec::new();
    for &t in onsets {
        if t < 0.0 {
            continue;
        }
        let f = (t * rate_hz).round() as usize;
        if f >= n {
            continue;
        }
        match starts.last() {
            Some(&prev) if f.saturating_sub(prev) as f64 * dt < MIN_SLICE_SECONDS => continue,
            _ => starts.push(f),
        }
    }
    match starts.first_mut() {
        // Whatever precedes the first attack is pre-roll — lead-in silence, a
        // room tone, the tail of a fade-up. It belongs to the first slice, so
        // pull that slice's start back to the file start rather than opening a
        // stub window that holds no event.
        Some(first) => *first = 0,
        // No detectable onset at all: a drone, a bed, a slow fade-in. One window
        // over the whole file is the honest reading.
        None => starts.push(0),
    }

    let mut slices: Vec<EnvelopeSlice> = Vec::with_capacity(starts.len());
    for (i, &s0) in starts.iter().enumerate() {
        let s1 = starts.get(i + 1).copied().unwrap_or(n);
        if s1 <= s0 {
            continue;
        }
        let win_sm = &sm[s0..s1];
        let win_env = &env[s0..s1];
        let win_peak = win_sm.iter().cloned().fold(0.0f64, f64::max);
        slices.push(EnvelopeSlice {
            index: slices.len(),
            start_seconds: s0 as f64 * dt,
            end_seconds: s1 as f64 * dt,
            relative_level: (win_peak / file_peak).clamp(0.0, 1.0),
            // Inside a slice there is exactly one event by construction, so the
            // single-note shape model applies — pass transients = 1.
            envelope: measure(win_sm, win_env, dt, 1),
        });
    }
    if slices.is_empty() {
        return silent_analysis();
    }

    let representative = slices
        .iter()
        .enumerate()
        .max_by(|a, b| {
            a.1.relative_level
                .partial_cmp(&b.1.relative_level)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
        .unwrap_or(0);

    let single_event = slices.len() == 1;
    // File-level shape: the honest categorical answer even when the numbers are
    // not. `classify_shape` returns "Multi" as soon as there is more than one.
    let shape = if single_event {
        slices[0].envelope.shape
    } else {
        "Multi"
    };

    EnvelopeAnalysis { slices, representative, single_event, shape }
}

fn silent_analysis() -> EnvelopeAnalysis {
    EnvelopeAnalysis {
        slices: vec![EnvelopeSlice {
            index: 0,
            start_seconds: 0.0,
            end_seconds: 0.0,
            relative_level: 0.0,
            envelope: Envelope::silent(),
        }],
        representative: 0,
        single_event: true,
        shape: "Silent",
    }
}

/// Measure one single-event window. Every threshold here is relative to
/// `sm`'s OWN peak — the window is the world, which is what makes the result a
/// property of the sound rather than of the file it was pasted into.
///
/// `sm` is the smoothed track, `raw` the unsmoothed one (the moments and the
/// centroid want the real distribution, not a filtered one), both already cut
/// to the window. `dt` is seconds per frame.
fn measure(sm: &[f64], raw: &[f64], dt: f64, transients: usize) -> Envelope {
    let n = sm.len();
    if n == 0 {
        return Envelope::silent();
    }
    let peak = sm.iter().cloned().fold(0.0f64, f64::max);
    if peak <= 0.0 {
        return Envelope::silent();
    }
    // Anchor the decay/sustain/release segmentation at the first arrival near
    // the peak: a held tone's literal argmax is just numeric ripple and can
    // land at the very end of the window, which would erase the plateau.
    let peak_idx = sm.iter().position(|&v| v >= 0.95 * peak).unwrap_or(0);

    // Attack: first 10 % crossing → first 90 % crossing.
    let onset = sm.iter().position(|&v| v >= 0.10 * peak).unwrap_or(0);
    let t90 = sm.iter().position(|&v| v >= 0.90 * peak).unwrap_or(onset);
    let attack = (t90.saturating_sub(onset)) as f64 * dt;

    // Sustain: median level over the middle of the post-peak region.
    let tail = n - peak_idx;
    let sustain = if tail > 4 {
        let a = peak_idx + (tail as f64 * 0.3) as usize;
        let b = peak_idx + (tail as f64 * 0.7) as usize;
        let mut w: Vec<f64> = sm[a..b.max(a + 1)].to_vec();
        w.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
        (w[w.len() / 2] / peak).clamp(0.0, 1.0)
    } else {
        // Too short after the peak to have a plateau at all.
        0.0
    };

    // Decay: peak → first arrival at the plateau (10 % above the sustain level).
    let dec_target = (sustain * 1.1).min(0.95) * peak;
    let decay = sm[peak_idx..]
        .iter()
        .position(|&v| v <= dec_target)
        .map(|k| k as f64 * dt)
        .unwrap_or((n - peak_idx) as f64 * dt);

    // Release: last time at/above the plateau (or ≥25 % of peak, whichever is
    // higher) → last audible frame (≥5 % of peak).
    let rel_thr = (sustain * 0.7).max(0.25) * peak;
    let i_rel = sm.iter().rposition(|&v| v >= rel_thr).unwrap_or(peak_idx);
    let i_end = sm.iter().rposition(|&v| v >= 0.05 * peak).unwrap_or(i_rel);
    let release = i_end.saturating_sub(i_rel) as f64 * dt;

    // Temporal centroid: energy-weighted mean time, normalized 0..1.
    let e_sum: f64 = raw.iter().map(|&v| v * v).sum();
    let centroid = if e_sum > 0.0 {
        raw.iter().enumerate().map(|(k, &v)| k as f64 * v * v).sum::<f64>()
            / (e_sum * raw.len().max(1) as f64)
    } else {
        0.0
    };

    // Envelope moments: skewness separates front-loaded percussive envelopes
    // from held ones; kurtosis flags isolated bursts in otherwise smooth audio.
    let m = moments(raw);

    let length = n as f64 * dt;
    let shape = classify_shape(attack, sustain, decay, length, transients);
    let decay_time_60db = decay_time(sm, peak_idx, peak, dt);
    Envelope {
        attack, decay, sustain, release, centroid,
        skew: m.skew, kurt: m.kurt, shape, decay_time_60db,
    }
}

/// Ring time (RT60), extrapolated from the initial post-peak decay slope.
///
/// Fitting the *whole* tail would be wrong: it runs into the noise floor, and
/// into whatever else happens later in the window. So the fit is confined to the
/// −5 dB → −25 dB window below the peak, the room-acoustics convention (a T20
/// measurement), and stops at the first frame that leaves it. The slope over
/// that window is extrapolated to a 60 dB fall.
///
/// Returns None when the sound does not decay at all — a held tone or a steady
/// bed never falls 25 dB, and reporting 0 s of ring for a drone would be a lie
/// that reads as "maximally damped".
fn decay_time(smoothed: &[f64], peak_idx: usize, peak: f64, dt: f64) -> Option<f64> {
    const START_DB: f64 = -5.0; // skip the peak itself: it is a transient, not the decay
    const END_DB: f64 = -25.0;
    const MAX_SECONDS: f64 = 60.0;

    if peak <= 0.0 || peak_idx >= smoothed.len() {
        return None;
    }
    let mut points: Vec<(f64, f64)> = Vec::new();
    for (k, &v) in smoothed.iter().enumerate().skip(peak_idx) {
        let db = 20.0 * (v / peak).max(1e-12).log10();
        if db > START_DB {
            continue; // still in the peak region
        }
        if db < END_DB {
            break; // left the fit window — everything past here is floor
        }
        points.push(((k - peak_idx) as f64 * dt, db));
    }
    if points.len() < 4 {
        return None; // never decayed through the window (or fell through it instantly)
    }
    // Insist on a real fall before extrapolating one. Fitting a slope to a
    // fragment that only slipped 2 dB and then multiplying it out to 60 dB is
    // how you get the 20-second "ring times" that came out of the first pass
    // over FSD50K — an extrapolation 30× longer than its evidence.
    const MIN_FALL_DB: f64 = 15.0;
    let observed_fall = points.first()?.1 - points.last()?.1;
    if observed_fall < MIN_FALL_DB {
        return None;
    }
    // Ordinary least squares of dB against time.
    let n = points.len() as f64;
    let mt = points.iter().map(|p| p.0).sum::<f64>() / n;
    let md = points.iter().map(|p| p.1).sum::<f64>() / n;
    let mut num = 0.0;
    let mut den = 0.0;
    for (t, db) in &points {
        num += (t - mt) * (db - md);
        den += (t - mt) * (t - mt);
    }
    if den < 1e-12 {
        return None;
    }
    let slope_db_per_second = num / den;
    if slope_db_per_second > -1.0 {
        return None; // flat or rising: not a decay
    }
    let rt60 = -60.0 / slope_db_per_second;
    (rt60.is_finite() && rt60 > 0.0).then(|| rt60.min(MAX_SECONDS))
}

/// Categorical envelope shape from the measured segments.
fn classify_shape(attack: f64, sustain: f64, decay: f64, length: f64, transients: usize) -> &'static str {
    if transients > 1 {
        return "Multi"; // repeated hits — single-note ADSR doesn't apply
    }
    if attack > (0.1 * length).max(0.15) {
        return "Swell"; // slow fade-in (pad / bowed / reversed)
    }
    if sustain > 0.5 {
        return "Sustained"; // held at level (drone / organ / lead)
    }
    if sustain < 0.15 && decay < 0.15 {
        return "Plucky"; // instant peak, dies immediately (pluck / click / dry hit)
    }
    "Decaying" // fast attack, gradual die-off (piano / cymbal / room hit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transients::transient_onsets;

    const SR: u32 = 44_100;

    /// Analyze exactly as the pipeline does: detect onsets, then slice on them.
    fn analyze(data: &[f32]) -> EnvelopeAnalysis {
        envelope_analysis(data, SR, &transient_onsets(data, SR))
    }

    /// Tone with a piecewise-linear amplitude envelope over (time s, level) points.
    fn shaped(points: &[(f32, f32)]) -> Vec<f32> {
        let total = points.last().unwrap().0;
        (0..(total * SR as f32) as usize)
            .map(|i| {
                let t = i as f32 / SR as f32;
                let mut lvl = 0.0;
                for w in points.windows(2) {
                    let ((t0, a0), (t1, a1)) = (w[0], w[1]);
                    if t >= t0 && t <= t1 {
                        lvl = a0 + (a1 - a0) * (t - t0) / (t1 - t0).max(1e-9);
                    }
                }
                lvl * (2.0 * std::f32::consts::PI * 220.0 * t).sin()
            })
            .collect()
    }

    #[test]
    fn pad_swell_vs_pluck() {
        // Slow 0.5 s fade-in, hold, slow fade-out ⇒ Swell, high sustain.
        let pad = analyze(&shaped(&[(0.0, 0.0), (0.5, 1.0), (1.5, 0.95), (2.0, 0.0)]));
        assert!(pad.single_event, "a swell is one event");
        let e = pad.representative();
        assert_eq!(e.shape, "Swell");
        assert!(e.attack > 0.3, "attack = {}", e.attack);
        assert!(e.sustain > 0.5, "sustain = {}", e.sustain);

        // Instant peak, dead in 80 ms ⇒ Plucky, near-zero sustain.
        let pluck = analyze(&shaped(&[(0.0, 0.0), (0.005, 1.0), (0.08, 0.0), (0.5, 0.0)]));
        assert!(pluck.single_event);
        let e = pluck.representative();
        assert_eq!(e.shape, "Plucky");
        assert!(e.attack < 0.05, "attack = {}", e.attack);
        assert!(e.sustain < 0.15, "sustain = {}", e.sustain);
        assert!(e.centroid < 0.2, "centroid = {}", e.centroid);
        // Front-loaded energy ⇒ strongly positive envelope skewness.
        assert!(e.skew > 1.0, "skew = {}", e.skew);
    }

    #[test]
    fn held_tone_is_sustained() {
        let lead = analyze(&shaped(&[(0.0, 0.0), (0.01, 1.0), (0.9, 0.9), (1.0, 0.0)]));
        assert!(lead.single_event);
        assert_eq!(lead.representative().shape, "Sustained");
        assert!(lead.representative().sustain > 0.5);
    }

    /// Exponentially decaying tone with a known RT60, to check we recover it.
    fn ringing(rt60: f32, secs: f32) -> Vec<f32> {
        let tau = rt60 / 6.908; // 60 dB fall = e^(-t/tau) with t/tau = ln(1000)
        (0..(secs * SR as f32) as usize)
            .map(|i| {
                let t = i as f32 / SR as f32;
                (-t / tau).exp() * (2.0 * std::f32::consts::PI * 440.0 * t).sin()
            })
            .collect()
    }

    #[test]
    fn ring_time_separates_a_bell_from_a_thud() {
        // A 2 s ring (metal) and a 0.15 s one (wood) — the material-impedance axis.
        let bell = analyze(&ringing(2.0, 3.0)).representative().decay_time_60db.unwrap();
        let thud = analyze(&ringing(0.15, 1.0)).representative().decay_time_60db.unwrap();
        assert!((bell - 2.0).abs() < 0.4, "bell RT60 = {bell} s");
        assert!((thud - 0.15).abs() < 0.06, "thud RT60 = {thud} s");
        assert!(bell > thud * 5.0);
    }

    #[test]
    fn a_sound_that_never_decays_reports_no_ring_time_rather_than_zero() {
        // A held tone at full level: a 0 s ring time would read as "maximally
        // damped", the exact opposite of the truth.
        let drone = analyze(&shaped(&[(0.0, 0.0), (0.01, 1.0), (2.0, 1.0)]));
        assert!(drone.representative().decay_time_60db.is_none());
    }

    // ---------------------------------------------------------------- slicing

    /// `hits` = (start s, duration s, level) exponentially decaying bursts.
    fn multi(total: f64, hits: &[(f64, f64, f64)]) -> Vec<f32> {
        let n = (total * SR as f64) as usize;
        (0..n)
            .map(|i| {
                let t = i as f64 / SR as f64;
                let mut lvl = 0.0f64;
                for &(start, dur, level) in hits {
                    if t >= start && t < start + dur {
                        lvl += level * (-(t - start) / (dur / 4.0)).exp();
                    }
                }
                (lvl * (2.0 * std::f64::consts::PI * 440.0 * t).sin()) as f32
            })
            .collect()
    }

    /// The exact failure this module was rewritten for: a 3 s file of quiet
    /// events whose loudest one lands at 2.65 s. The old whole-file measurement
    /// reported an attack of ~2.5 s — the distance to the loud hit, i.e. the
    /// user's edit. Every slice must now report its own short attack instead.
    #[test]
    fn a_late_loud_hit_does_not_stretch_the_attack_of_the_quiet_ones() {
        // The quiet hits sit at half the level of the loud one — enough of a
        // spread to trigger the bug, while staying above the onset detector's
        // prominence floor (its sensitivity is a separate axis from this).
        let mut events: Vec<(f64, f64, f64)> =
            (0..9).map(|k| (0.15 + k as f64 * 0.28, 0.16, 0.5)).collect();
        events.push((2.65, 0.25, 1.0)); // the loud one, late
        let typing = multi(3.0, &events);

        // What the old code did: one window over the whole file, every threshold
        // relative to the global peak. Passing no onsets reproduces it exactly.
        // The 10 % crossing lands on the first quiet hit and the 90 % crossing on
        // the loud one at 2.65 s, so the "attack" is the gap between them.
        let whole_file = envelope_analysis(&typing, SR, &[]);
        let bogus = whole_file.representative().attack;
        assert!(
            bogus > 1.5,
            "expected the whole-file measurement to still show the bug, got {bogus:.3} s"
        );

        let a = envelope_analysis(&typing, SR, &transient_onsets(&typing, SR));
        assert!(!a.single_event, "10 hits must not read as one event");
        assert!(a.file_level().is_none(), "a multi-event file has no file-level ADSR");
        assert_eq!(a.shape, "Multi");
        assert!(a.slices.len() >= 8, "got {} slices", a.slices.len());

        for s in &a.slices {
            assert!(
                s.envelope.attack < 0.15,
                "slice {} at {:.2}s reported a {:.3} s attack — the whole-file bug",
                s.index, s.start_seconds, s.envelope.attack
            );
            assert!(s.end_seconds > s.start_seconds, "slice {} is empty", s.index);
        }
        // The representative slice is the loud one, and it is where we said.
        let rep = &a.slices[a.representative];
        assert!(
            (rep.start_seconds - 2.65).abs() < 0.1,
            "representative slice starts at {:.2} s, expected ~2.65 s",
            rep.start_seconds
        );
        assert!((rep.relative_level - 1.0).abs() < 1e-9);
    }

    #[test]
    fn slices_are_contiguous_and_in_order() {
        let audio = multi(2.0, &[(0.1, 0.3, 1.0), (0.7, 0.3, 0.8), (1.3, 0.3, 0.9)]);
        let a = envelope_analysis(&audio, SR, &transient_onsets(&audio, SR));
        assert_eq!(a.slices.len(), 3, "got {} slices", a.slices.len());
        for (i, s) in a.slices.iter().enumerate() {
            assert_eq!(s.index, i);
            if let Some(next) = a.slices.get(i + 1) {
                assert!(
                    (s.end_seconds - next.start_seconds).abs() < 1e-9,
                    "gap between slice {i} and {}", i + 1
                );
            }
        }
    }

    #[test]
    fn a_one_shot_still_yields_exactly_one_slice_covering_the_file() {
        let pluck = shaped(&[(0.0, 0.0), (0.005, 1.0), (0.08, 0.0), (0.5, 0.0)]);
        let a = analyze(&pluck);
        assert_eq!(a.slices.len(), 1);
        assert!(a.single_event && a.file_level().is_some());
        assert_eq!(a.slices[0].start_seconds, 0.0);
        assert!(a.slices[0].end_seconds > 0.45, "{}", a.slices[0].end_seconds);
    }

    #[test]
    fn silence_yields_one_silent_slice() {
        let a = envelope_analysis(&vec![0.0f32; SR as usize], SR, &[]);
        assert_eq!(a.slices.len(), 1);
        assert_eq!(a.shape, "Silent");
        assert!(a.single_event);
    }
}
