//! Morphology features — the "proposed" set of `ucs_signature_spec.md` §4b.
//!
//! These exist because the UCS signatures ask for them: 329 priors reference
//! `stationarity`, 202 `decay_time_seconds_60db`, 148 `voicing_ratio`. Until now
//! the scorer had no field to read them from, so `ucs::feature()` returned None
//! and every one of those terms was dropped from the likelihood — the signatures
//! were being scored on a fraction of the evidence they were written against.
//!
//! Every value is an `Option`: "we could not measure this" is a different
//! statement from "this is zero", and the scorer must be able to tell them
//! apart (spec §6 — a missing feature is skipped and its weight renormalized
//! away, never treated as 0).
//!
//! All of these ride on the STFT and amplitude envelope the analyzer already
//! computes, so the cost is arithmetic, not another transform.
use rustfft::{num_complex::Complex, FftPlanner};

pub struct Morphology {
    /// 0 = eventful, 1 = an unchanging bed. Texture-vs-event.
    pub stationarity: Option<f64>,
    /// Shannon entropy of the power spectrum, normalized 0..1. Tonal vs noise,
    /// more robust than flatness on sparse spectra.
    pub spectral_entropy: Option<f64>,
    /// LTAS tilt. Pink noise is −3 dB/oct exactly.
    pub spectral_slope_db_per_octave: Option<f64>,
    /// Highest frequency still above the noise floor — finds the encoder/telephone
    /// lowpass that defines VOICES-FUTZED, WATER-UNDERWATER, VEHICLES-INTERIOR.
    pub band_limit_high_hz: Option<f64>,
    /// Brightness trajectory: the `sweep` morphology (risers, whooshes, dives).
    pub spectral_centroid_slope_hz_per_second: Option<f64>,
    /// The same trajectory in the tonal domain (LASERS, CARTOON-ZIP).
    pub pitch_slope_semitones_per_second: Option<f64>,
    /// Fraction of envelope-modulation energy in the 3–8 Hz syllabic band —
    /// speech-likeness. Separates WALLA from a stationary crowd bed.
    pub syllabic_modulation_energy: Option<f64>,
}

impl Morphology {
    fn empty() -> Morphology {
        Morphology {
            stationarity: None,
            spectral_entropy: None,
            spectral_slope_db_per_octave: None,
            band_limit_high_hz: None,
            spectral_centroid_slope_hz_per_second: None,
            pitch_slope_semitones_per_second: None,
            syllabic_modulation_energy: None,
        }
    }
}

/// The floor, relative to the loudest band, below which we call it silence.
/// Used by both the band limit and the entropy's frame gate.
const FLOOR_DB: f64 = -50.0;

/// Compute every §4b feature from the already-extracted STFT frames and
/// amplitude envelope.
///
/// `frames` are the Hann magnitude spectra from `stft::stft_mags`, `hop` their
/// stride in samples. `envelope` is the RMS amplitude track from
/// `envelope::amplitude_envelope`, sampled at `envelope_rate_hz`.
pub fn morphology(
    frames: &[Vec<f32>],
    sr_f: f64,
    n_fft: usize,
    hop: usize,
    envelope: &[f64],
    envelope_rate_hz: f64,
) -> Morphology {
    if frames.is_empty() || n_fft < 2 || sr_f <= 0.0 {
        return Morphology::empty();
    }
    let bin_hz = sr_f / n_fft as f64;
    let ltas = long_term_average_spectrum(frames);

    Morphology {
        stationarity: stationarity(frames),
        spectral_entropy: spectral_entropy(frames),
        spectral_slope_db_per_octave: spectral_slope(&ltas, bin_hz, sr_f),
        band_limit_high_hz: band_limit(&ltas, bin_hz),
        spectral_centroid_slope_hz_per_second: centroid_slope(frames, sr_f, n_fft, hop),
        pitch_slope_semitones_per_second: pitch_slope(frames, bin_hz, hop, sr_f),
        syllabic_modulation_energy: syllabic_modulation(envelope, envelope_rate_hz),
    }
}

// ------------------------------------------------------------------ stationarity

/// `1 − (std / mean)` of the per-frame RMS *amplitude*, clamped to 0..1.
///
/// A steady bed (surf, room tone, an air hiss) barely varies frame to frame, so
/// its coefficient of variation is small and this lands high. A single impact is
/// nearly all its energy in one frame, so the CV is large and this saturates
/// at 0.
///
/// The spec wrote this over frame *energy*. Measured against FSD50K it had to be
/// amplitude instead: energy is quadratic, so its CV runs past 1 on anything with
/// dynamics, and the clamp then flattens it. On labelled clips the energy form
/// scored a median of 0.000 for steady beds (rain, wind, surf, frying) *and*
/// 0.000 for impacts (gunshots, knocks, slams) — no discriminative power at all.
/// The amplitude form separates the same two groups 0.47 to 0.00. Taking the
/// square root is the whole difference.
fn stationarity(frames: &[Vec<f32>]) -> Option<f64> {
    if frames.len() < 4 {
        return None; // too few frames to speak of variation over time
    }
    let amplitude: Vec<f64> = frames
        .iter()
        .map(|f| f.iter().map(|&m| (m as f64) * (m as f64)).sum::<f64>().sqrt())
        .collect();
    let mean = amplitude.iter().sum::<f64>() / amplitude.len() as f64;
    if mean <= 1e-12 {
        return None; // digital silence — stationarity is undefined, not 1
    }
    let var = amplitude.iter().map(|a| (a - mean) * (a - mean)).sum::<f64>() / amplitude.len() as f64;
    let coefficient_of_variation = var.sqrt() / mean;
    Some((1.0 - coefficient_of_variation).clamp(0.0, 1.0))
}

// -------------------------------------------------------------- spectral entropy

/// Mean over frames of the Shannon entropy of the normalized power spectrum,
/// divided by ln(bins) so a perfectly flat spectrum scores 1 and a pure sine ~0.
fn spectral_entropy(frames: &[Vec<f32>]) -> Option<f64> {
    let bins = frames.first()?.len();
    if bins < 2 {
        return None;
    }
    let norm = (bins as f64).ln();
    let mut acc = 0.0;
    let mut counted = 0usize;
    for frame in frames {
        let total: f64 = frame.iter().map(|&m| (m as f64) * (m as f64)).sum();
        if total <= 1e-12 {
            continue; // silent frame carries no timbre
        }
        let mut h = 0.0;
        for &m in frame {
            let p = (m as f64) * (m as f64) / total;
            if p > 1e-12 {
                h -= p * p.ln();
            }
        }
        acc += h / norm;
        counted += 1;
    }
    if counted == 0 {
        return None;
    }
    Some((acc / counted as f64).clamp(0.0, 1.0))
}

// ------------------------------------------------------------------------ LTAS

/// Long-term average spectrum: the mean magnitude in each bin across all frames.
fn long_term_average_spectrum(frames: &[Vec<f32>]) -> Vec<f64> {
    let bins = frames.first().map(|f| f.len()).unwrap_or(0);
    let mut ltas = vec![0.0f64; bins];
    for frame in frames {
        for (k, &m) in frame.iter().enumerate() {
            ltas[k] += m as f64;
        }
    }
    let n = frames.len() as f64;
    for v in &mut ltas {
        *v /= n;
    }
    ltas
}

/// Spectral tilt in dB per octave, from a third-octave-band reduction of the LTAS.
///
/// The banding matters. Regressing raw FFT bins against log-frequency would let
/// the top octave — which holds half the bins — dictate the fit. Third-octave
/// bands are equal-width on the log axis the slope is defined over, which is why
/// pink noise comes out at −3.0 here and would not otherwise.
fn spectral_slope(ltas: &[f64], bin_hz: f64, sr_f: f64) -> Option<f64> {
    let nyquist = sr_f / 2.0;
    let f_lo = 50.0f64;
    let f_hi = (nyquist * 0.9).min(16_000.0);
    if f_hi <= f_lo * 2.0 {
        return None; // less than an octave of usable band
    }
    // Third-octave band centers from f_lo up to f_hi.
    let ratio = 2.0f64.powf(1.0 / 3.0);
    let mut points: Vec<(f64, f64)> = Vec::new(); // (log2 f, dB)
    let mut center = f_lo * ratio; // first full band sits above f_lo
    while center < f_hi {
        let lo = center / ratio.sqrt();
        let hi = center * ratio.sqrt();
        let k0 = (lo / bin_hz).floor() as usize;
        let k1 = ((hi / bin_hz).ceil() as usize).min(ltas.len());
        if k0 < k1 {
            let power: f64 = ltas[k0..k1].iter().map(|m| m * m).sum::<f64>() / (k1 - k0) as f64;
            if power > 1e-20 {
                points.push((center.log2(), 10.0 * power.log10()));
            }
        }
        center *= ratio;
    }
    if points.len() < 4 {
        return None;
    }
    least_squares_slope(&points)
}

/// Highest frequency whose LTAS is still within `FLOOR_DB` of the loudest band.
///
/// Scanned downward from the top so a lone stray bin cannot set the limit; the
/// 3-bin median smoothing is what makes that true.
fn band_limit(ltas: &[f64], bin_hz: f64) -> Option<f64> {
    if ltas.len() < 8 {
        return None;
    }
    let smoothed: Vec<f64> = (0..ltas.len())
        .map(|k| {
            let a = ltas[k.saturating_sub(1)];
            let b = ltas[k];
            let c = ltas[(k + 1).min(ltas.len() - 1)];
            let mut w = [a, b, c];
            w.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
            w[1]
        })
        .collect();
    // Ignore DC and the sub-50 Hz rumble when looking for the loudest band.
    let k_min = ((50.0 / bin_hz).floor() as usize).max(1);
    if k_min >= smoothed.len() {
        return None;
    }
    let peak = smoothed[k_min..].iter().cloned().fold(0.0f64, f64::max);
    if peak <= 1e-12 {
        return None; // silence has no band limit
    }
    let threshold = peak * 10.0f64.powf(FLOOR_DB / 20.0);
    let k = smoothed.iter().rposition(|&m| m >= threshold)?;
    Some(k as f64 * bin_hz)
}

// ------------------------------------------------------------- sweep trajectories

/// OLS slope of the per-frame spectral centroid against time (Hz/s).
fn centroid_slope(frames: &[Vec<f32>], sr_f: f64, n_fft: usize, hop: usize) -> Option<f64> {
    let track = crate::framestats::centroid_track(frames, sr_f, n_fft, hop);
    if track.len() < 4 {
        return None;
    }
    let span = track.last()?.0 - track.first()?.0;
    if span < 0.05 {
        return None; // too brief for a trajectory to mean anything
    }
    least_squares_slope(&track)
}

/// OLS slope of a frame-wise pitch track, in semitones per second.
///
/// The track follows the *dominant partial* in the 50–2000 Hz band, not the
/// fundamental. That sounds like a compromise and is in fact the robust choice:
/// a slope in semitones is invariant to which harmonic you follow, because every
/// partial of a glide moves by the same ratio. Picking H2 instead of f0 shifts
/// the track by a constant octave and leaves the slope untouched — and unlike a
/// harmonic product spectrum, an argmax does not collapse on a pure sine, which
/// has no harmonics for the product to find.
///
/// Bin resolution (≈21 Hz at 2048/44.1 kHz) is coarse at the bottom of the band,
/// so the peak is parabolically interpolated; even then, treat a slope drawn
/// from a very low partial as approximate.
fn pitch_slope(frames: &[Vec<f32>], bin_hz: f64, hop: usize, sr_f: f64) -> Option<f64> {
    const F_LO: f64 = 50.0;
    const F_HI: f64 = 2000.0;
    /// How far the dominant partial must stand above the band's mean magnitude
    /// before we will call the frame pitched. Without this the "pitch track" of
    /// a hiss is a random walk, and its slope is noise dressed as a trajectory.
    const CLEARANCE: f64 = 8.0;

    let bins = frames.first()?.len();
    let k_lo = ((F_LO / bin_hz).floor() as usize).max(1);
    let k_hi = ((F_HI / bin_hz).ceil() as usize).min(bins - 1);
    if k_lo + 2 >= k_hi {
        return None;
    }
    let frame_seconds = hop as f64 / sr_f;

    let mut track: Vec<(f64, f64)> = Vec::new(); // (time_s, semitones re A440)
    for (fi, frame) in frames.iter().enumerate() {
        let mut best_k = 0usize;
        let mut best_v = 0.0f64;
        let mut sum = 0.0f64;
        for k in k_lo..k_hi {
            let m = frame[k] as f64;
            sum += m;
            if m > best_v {
                best_v = m;
                best_k = k;
            }
        }
        let mean = sum / (k_hi - k_lo) as f64;
        if best_k == 0 || mean <= 1e-12 || best_v < CLEARANCE * mean {
            continue; // unpitched frame
        }
        let f = (best_k as f64 + parabolic_offset(frame, best_k)) * bin_hz;
        if !(F_LO..=F_HI).contains(&f) {
            continue;
        }
        track.push((fi as f64 * frame_seconds, 12.0 * (f / 440.0).log2()));
    }
    if track.len() < 6 {
        return None; // not enough pitched frames to call it a trajectory
    }
    let span = track.last()?.0 - track.first()?.0;
    if span < 0.05 {
        return None;
    }
    // Only report a trajectory the track actually supports. Most real material
    // is neither a glide nor a held note: the dominant partial wanders, and
    // regressing that wander yields a slope near zero — a confident-looking "no
    // sweep" that is really "no idea". Measured on 300 FSD50K clips, the ungated
    // version reported a slope for 99.7 % of files, nearly all spurious.
    //
    // The band spans 50–2000 Hz, i.e. ~64 semitones, so a random walk scatters
    // by tens of semitones; a real pitch trajectory — sweeping or flat — sits
    // within a couple. Abstaining is the correct answer for a sound that is
    // simply not pitched.
    const MAX_SCATTER_SEMITONES: f64 = 2.0;
    let (slope, intercept) = least_squares_line(&track)?;
    let scatter = residual_scatter(&track, slope, intercept);
    (scatter <= MAX_SCATTER_SEMITONES).then_some(slope)
}

/// Sub-bin peak position by fitting a parabola through the log magnitudes at
/// `k−1, k, k+1`. Returns an offset in bins, in (−0.5, +0.5).
fn parabolic_offset(frame: &[f32], k: usize) -> f64 {
    if k == 0 || k + 1 >= frame.len() {
        return 0.0;
    }
    let lg = |v: f32| ((v as f64).max(1e-12)).ln();
    let (a, b, c) = (lg(frame[k - 1]), lg(frame[k]), lg(frame[k + 1]));
    let denom = a - 2.0 * b + c;
    if denom.abs() < 1e-12 {
        return 0.0;
    }
    (0.5 * (a - c) / denom).clamp(-0.5, 0.5)
}

// ------------------------------------------------------- syllabic modulation band

/// Fraction of the envelope's modulation spectrum that sits in the 3–8 Hz
/// syllabic band, measured against the whole 0.5–30 Hz modulation range.
///
/// Speech and walla modulate at the syllable rate; a stationary crowd bed or a
/// machine hum does not. The envelope's DC is removed first, otherwise the mean
/// level would swamp every modulation band.
pub(crate) fn syllabic_modulation(envelope: &[f64], rate_hz: f64) -> Option<f64> {
    const SYLLABIC_LO: f64 = 3.0;
    const SYLLABIC_HI: f64 = 8.0;
    const BAND_LO: f64 = 0.5;
    const BAND_HI: f64 = 30.0;
    /// 8192 bins at the 200 Hz envelope rate is ~41 s — past that the extra
    /// modulation detail changes nothing and the FFT stops being free.
    const MAX_N: usize = 8192;

    if rate_hz <= 2.0 * BAND_HI || envelope.len() < 128 {
        return None; // under ~0.64 s there is no such thing as a 3 Hz modulation
    }
    let take = envelope.len().min(MAX_N);
    let mean = envelope[..take].iter().sum::<f64>() / take as f64;
    if mean <= 1e-9 {
        return None;
    }
    let mut n = 1usize;
    while n < take {
        n <<= 1;
    }
    let mut buf = vec![Complex { re: 0.0f64, im: 0.0f64 }; n];
    for i in 0..take {
        // Hann window over the used span, DC removed.
        let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (take as f64 - 1.0)).cos();
        buf[i] = Complex { re: (envelope[i] - mean) * w, im: 0.0 };
    }
    FftPlanner::<f64>::new().plan_fft_forward(n).process(&mut buf);

    let df = rate_hz / n as f64;
    let mut total = 0.0f64;
    let mut syllabic = 0.0f64;
    for (k, c) in buf.iter().enumerate().take(n / 2) {
        let f = k as f64 * df;
        if !(BAND_LO..=BAND_HI).contains(&f) {
            continue;
        }
        let power = c.norm_sqr();
        total += power;
        if (SYLLABIC_LO..=SYLLABIC_HI).contains(&f) {
            syllabic += power;
        }
    }
    if total <= 1e-20 {
        return None;
    }
    Some((syllabic / total).clamp(0.0, 1.0))
}

// ---------------------------------------------------------------------- helpers

/// Ordinary-least-squares slope of y on x. None when x has no spread.
fn least_squares_slope(points: &[(f64, f64)]) -> Option<f64> {
    least_squares_line(points).map(|(slope, _)| slope)
}

/// OLS fit of y on x, as `(slope, intercept)`.
fn least_squares_line(points: &[(f64, f64)]) -> Option<(f64, f64)> {
    let n = points.len() as f64;
    if n < 2.0 {
        return None;
    }
    let mx = points.iter().map(|p| p.0).sum::<f64>() / n;
    let my = points.iter().map(|p| p.1).sum::<f64>() / n;
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    for (x, y) in points {
        sxy += (x - mx) * (y - my);
        sxx += (x - mx) * (x - mx);
    }
    if sxx.abs() < 1e-12 {
        return None;
    }
    let slope = sxy / sxx;
    if !slope.is_finite() {
        return None;
    }
    Some((slope, my - slope * mx))
}

/// Root-mean-square distance of the points from the fitted line, in y's units.
///
/// This — not R² — is what tells a trajectory from a random walk. R² would
/// reject a *steady* tone, whose track is flat and therefore has no variance for
/// a slope to explain, even though "flat" is a perfectly good measurement of
/// zero sweep. Residual scatter separates the two honestly: a glide and a held
/// tone both hug their line, and only a wandering track strays from it.
fn residual_scatter(points: &[(f64, f64)], slope: f64, intercept: f64) -> f64 {
    let n = points.len() as f64;
    let ss: f64 = points
        .iter()
        .map(|(x, y)| {
            let e = y - (slope * x + intercept);
            e * e
        })
        .sum();
    (ss / n).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stft::stft_mags;

    const SR: f64 = 44_100.0;
    const N_FFT: usize = 2048;
    const HOP: usize = 512;

    fn frames_of(x: &[f32]) -> Vec<Vec<f32>> {
        stft_mags(x, N_FFT, HOP)
    }

    /// Deterministic white noise — no rand dependency, and a fixed seed means a
    /// fixed assertion.
    fn white(n: usize) -> Vec<f32> {
        let mut s = 12_345u32;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                ((s >> 8) as f32 / 8_388_608.0) - 1.0
            })
            .collect()
    }

    fn tone(f: f32, secs: f32) -> Vec<f32> {
        (0..(SR as f32 * secs) as usize)
            .map(|i| (2.0 * std::f32::consts::PI * f * i as f32 / SR as f32).sin())
            .collect()
    }

    /// A linear chirp from `f0` to `f1` over `secs`.
    ///
    /// The phase must be the *integral* of the instantaneous frequency:
    /// φ(t) = 2π(f₀t + (f₁−f₀)t²/2T). Writing the tempting `sin(2π·f(t)·t)`
    /// instead doubles the sweep rate, and a downward chirp written that way
    /// runs straight through 0 Hz into negative frequency and folds back up —
    /// which is a completely different sound from the one you asked for.
    fn chirp(f0: f32, f1: f32, secs: f32) -> Vec<f32> {
        let n = (SR as f32 * secs) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / SR as f32;
                let phase = 2.0 * std::f32::consts::PI
                    * (f0 * t + (f1 - f0) * t * t / (2.0 * secs));
                phase.sin()
            })
            .collect()
    }

    /// White noise brickwall-lowpassed at `cutoff` by zeroing every bin above it.
    ///
    /// A cascade of one-pole IIRs will not do: a one-pole only reaches −12.5 dB
    /// at Nyquist, so even four of them bottom out around −50 dB and never
    /// establish a band limit at all. A real telephone or codec lowpass is steep,
    /// and this is what steep means.
    fn band_limited(cutoff: f64, n: usize) -> Vec<f32> {
        let x = white(n);
        let mut buf: Vec<Complex<f64>> = x
            .iter()
            .map(|&v| Complex { re: v as f64, im: 0.0 })
            .collect();
        let mut planner = FftPlanner::<f64>::new();
        planner.plan_fft_forward(n).process(&mut buf);
        let bin_hz = SR / n as f64;
        for (k, c) in buf.iter_mut().enumerate() {
            // Zero the band above the cutoff, mirrored across Nyquist.
            let f = (k.min(n - k)) as f64 * bin_hz;
            if f > cutoff {
                *c = Complex { re: 0.0, im: 0.0 };
            }
        }
        planner.plan_fft_inverse(n).process(&mut buf);
        buf.iter().map(|c| (c.re / n as f64) as f32).collect()
    }

    #[test]
    fn a_steady_bed_is_stationary_and_an_impact_is_not() {
        let bed = stationarity(&frames_of(&white(SR as usize))).unwrap();
        // One click at the front, silence after: nearly all energy in one frame.
        let mut impact = vec![0.0f32; SR as usize];
        for (i, s) in impact.iter_mut().take(200).enumerate() {
            *s = (1.0 - i as f32 / 200.0) * 0.9;
        }
        let hit = stationarity(&frames_of(&impact)).unwrap();
        assert!(bed > 0.6, "steady noise stationarity = {bed}");
        assert!(hit < 0.2, "impact stationarity = {hit}");
    }

    #[test]
    fn entropy_separates_a_sine_from_noise() {
        let sine = spectral_entropy(&frames_of(&tone(440.0, 1.0))).unwrap();
        let noise = spectral_entropy(&frames_of(&white(SR as usize))).unwrap();
        assert!(sine < 0.5, "sine entropy = {sine}");
        assert!(noise > 0.8, "noise entropy = {noise}");
        assert!(noise > sine);
    }

    #[test]
    fn pink_noise_slopes_at_minus_three_db_per_octave() {
        // Shape white noise into pink with a one-pole −3 dB/oct filter bank
        // (Voss/McCartney weights), then confirm we recover the tilt.
        let w = white(SR as usize * 2);
        let (mut b0, mut b1, mut b2) = (0.0f32, 0.0f32, 0.0f32);
        let pink: Vec<f32> = w
            .iter()
            .map(|&x| {
                b0 = 0.99765 * b0 + x * 0.0990460;
                b1 = 0.96300 * b1 + x * 0.2965164;
                b2 = 0.57000 * b2 + x * 1.0526913;
                (b0 + b1 + b2 + x * 0.1848) * 0.2
            })
            .collect();
        let ltas = long_term_average_spectrum(&frames_of(&pink));
        let slope = spectral_slope(&ltas, SR / N_FFT as f64, SR).unwrap();
        assert!((slope + 3.0).abs() < 1.2, "pink noise slope = {slope} dB/oct");

        // White noise is flat by definition.
        let ltas = long_term_average_spectrum(&frames_of(&w));
        let flat = spectral_slope(&ltas, SR / N_FFT as f64, SR).unwrap();
        assert!(flat.abs() < 1.0, "white noise slope = {flat} dB/oct");
    }

    #[test]
    fn a_lowpassed_source_reports_its_band_limit() {
        let bin_hz = SR / N_FFT as f64;
        // A telephone futz: nothing above 3.4 kHz.
        let futzed = band_limited(3400.0, SR as usize);
        let limit = band_limit(&long_term_average_spectrum(&frames_of(&futzed)), bin_hz).unwrap();
        let full = band_limit(
            &long_term_average_spectrum(&frames_of(&white(SR as usize))),
            bin_hz,
        )
        .unwrap();
        assert!(
            (limit - 3400.0).abs() < 400.0,
            "telephone band limit = {limit} Hz, expected ~3400"
        );
        assert!(full > 15_000.0, "full-band noise limit = {full} Hz");
    }

    #[test]
    fn a_riser_has_a_positive_centroid_slope_and_a_dive_a_negative_one() {
        // 200 Hz → 6 kHz over 2 s: the centroid should climb ~2.9 kHz/s.
        let up = centroid_slope(&frames_of(&chirp(200.0, 6000.0, 2.0)), SR, N_FFT, HOP).unwrap();
        let down = centroid_slope(&frames_of(&chirp(6000.0, 200.0, 2.0)), SR, N_FFT, HOP).unwrap();
        let flat = centroid_slope(&frames_of(&tone(1000.0, 2.0)), SR, N_FFT, HOP).unwrap();
        assert!(up > 1000.0, "riser centroid slope = {up} Hz/s");
        assert!(down < -1000.0, "dive centroid slope = {down} Hz/s");
        assert!(flat.abs() < 200.0, "steady tone centroid slope = {flat} Hz/s");
    }

    #[test]
    fn an_upward_glide_reads_as_positive_semitones_per_second() {
        // 220 Hz → 440 Hz over 2 s: one octave, so +6 semitones/s.
        let bin_hz = SR / N_FFT as f64;
        let slope = pitch_slope(&frames_of(&chirp(220.0, 440.0, 2.0)), bin_hz, HOP, SR).unwrap();
        assert!((slope - 6.0).abs() < 1.5, "glide pitch slope = {slope} st/s, expected ~+6");

        // The same glide downward is the same magnitude, negated.
        let down = pitch_slope(&frames_of(&chirp(440.0, 220.0, 2.0)), bin_hz, HOP, SR).unwrap();
        assert!((down + 6.0).abs() < 1.5, "dive pitch slope = {down} st/s, expected ~−6");

        // A steady tone must not drift.
        let steady = pitch_slope(&frames_of(&tone(440.0, 2.0)), bin_hz, HOP, SR).unwrap();
        assert!(steady.abs() < 0.5, "steady pitch slope = {steady} st/s");

        // Noise is not pitched: refuse to report a trajectory rather than
        // regress a random walk and call the result a sweep.
        assert!(pitch_slope(&frames_of(&white(SR as usize)), bin_hz, HOP, SR).is_none());
    }

    #[test]
    fn a_syllable_rate_envelope_lands_in_the_speech_band() {
        // 200 Hz envelope rate, 5 Hz amplitude modulation = squarely syllabic.
        let rate = 200.0;
        let n = (rate * 4.0) as usize;
        let speechy: Vec<f64> = (0..n)
            .map(|i| {
                let t = i as f64 / rate;
                0.5 + 0.45 * (2.0 * std::f64::consts::PI * 5.0 * t).sin()
            })
            .collect();
        // A 0.2 Hz drift: slow, not syllabic.
        let bed: Vec<f64> = (0..n)
            .map(|i| {
                let t = i as f64 / rate;
                0.5 + 0.05 * (2.0 * std::f64::consts::PI * 0.2 * t).sin()
            })
            .collect();
        let s = syllabic_modulation(&speechy, rate).unwrap();
        let b = syllabic_modulation(&bed, rate).unwrap();
        assert!(s > 0.8, "5 Hz modulation syllabic energy = {s}");
        assert!(b < 0.2, "slow bed syllabic energy = {b}");
    }

    #[test]
    fn silence_measures_nothing_rather_than_measuring_zero() {
        let quiet = vec![0.0f32; SR as usize];
        let f = frames_of(&quiet);
        assert!(stationarity(&f).is_none(), "digital silence must not report a stationarity");
        assert!(spectral_entropy(&f).is_none());
        assert!(band_limit(&long_term_average_spectrum(&f), SR / N_FFT as f64).is_none());
    }
}
