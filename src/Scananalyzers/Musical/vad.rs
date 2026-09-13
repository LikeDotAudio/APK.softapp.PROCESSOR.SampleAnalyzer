//! Voicing: how much of a file is *periodic in the human-voice range*, and
//! whether that periodicity actually behaves like speech.
//!
//! This replaces the WebRTC VAD, for two reasons.
//!
//! 1. **It was wrong.** WebRTC's VAD is telephony-tuned and fires on any harmonic
//!    energy. Measured against FSD50K's labels it rated AMBIENCE-TRAFFIC **0.96
//!    voiced** and CROWDS-APPLAUSE 0.79 — while speech scored 0.94. A feature that
//!    returns ~0.8 for everything carries almost no information, and the 148 UCS
//!    priors resting on `voicing_ratio` were resting on noise.
//! 2. **It could not be built for the web.** `webrtc-vad` compiles C that needs a
//!    libc, and `wasm32-unknown-unknown` has none, so the WASM engine could not be
//!    rebuilt at all. A pure-Rust detector keeps the native and web engines
//!    byte-identical, which is what lets the web front absorb a `.PEAK` sidecar
//!    written by the desktop.
//!
//! What `voicing_ratio` now means, precisely: **the fraction of non-silent frames
//! whose waveform is strongly periodic at a fundamental between 60 and 400 Hz.**
//! It is a *periodicity* measure, not a speech classifier — a held cello note is
//! periodic and will score high. That is fine and intended: the UCS priors are
//! calibrated per subcategory, so what the feature must do is *discriminate*, and
//! periodicity separates the things the spec actually asks of it (CHEERING is
//! voiced, APPLAUSE is not; VOICES are voiced, RAIN is not).
//!
//! Telling *speech* from a sustained instrument needs a second fact, and the
//! syllable rate is it: a voice pulses 3–8 times a second, an instrument holds.
//! `has_voice` requires both, and that is what keeps the god-category from
//! calling every guitar a vocal.

/// Everything one pass over the signal can tell us about voicing.
pub struct Voicing {
    /// Fraction of non-silent frames that are periodic in the voice range, 0..1.
    /// None when the file is silent or too short to hold a single frame.
    pub ratio: Option<f64>,
    /// Periodic *and* pulsing at the syllable rate — a voice, not a held note.
    pub is_speech: bool,
}

// Analysis geometry. 8 kHz is plenty: a voice fundamental lives at 60–400 Hz and
// we only need the waveform's periodicity, not its brightness. Decimating first
// is what keeps the autocorrelation cheap enough to run on every file.
const TARGET_RATE: usize = 8_000;
const FRAME: usize = 240; // 30 ms
const HOP: usize = 80; //    10 ms
const F0_MIN: f64 = 60.0;
const F0_MAX: f64 = 400.0;

/// How periodic a frame must be before we call it voiced. Normalized
/// autocorrelation: 1.0 is a perfect repeat, ~0 is noise.
const VOICED_THRESHOLD: f64 = 0.45;

/// A frame this quiet relative to the file's loudest is silence, and silence has
/// no voicing — counting it would let a long tail of room tone drag the ratio to 0.
const SILENCE_FLOOR: f64 = 0.02;

/// Speech is voiced most of the time it is sounding.
const SPEECH_RATIO_MIN: f64 = 0.65;

/// ...and it is *rhythmic at the syllable rate*. This second test is what tells a
/// voice from an instrument, and it is not optional: a guitar and a piano are
/// even more periodic than a vowel (measured on FSD50K: music 0.99 voiced, speech
/// 0.88), so periodicity alone calls every guitar a vocal.
///
/// The first attempt here used pitch *wobble* instead — speech glides, a held note
/// does not. It failed on real music: a piano playing a melody changes note, and
/// that reads as more pitch movement than a spoken syllable. Syllabic modulation
/// does separate them, because it asks a different question — not "does the pitch
/// move" but "does the loudness pulse 3–8 times a second". Measured on FSD50K:
/// speech 0.31, music 0.08.
const SPEECH_SYLLABIC_MIN: f64 = 0.15;

/// The frame rate of the RMS track below — one frame per HOP at TARGET_RATE.
const FRAME_RATE_HZ: f64 = TARGET_RATE as f64 / HOP as f64; // 100 Hz

/// Measure voicing in one pass.
pub fn voice_activity(data: &[f32], sr: u32) -> Voicing {
    let x = decimate(data, sr);
    if x.len() < FRAME + 2 {
        return Voicing { ratio: None, is_speech: false };
    }

    let lag_min = (TARGET_RATE as f64 / F0_MAX).floor() as usize; // 20
    let lag_max = (TARGET_RATE as f64 / F0_MIN).ceil() as usize; //  134
    if lag_max >= FRAME {
        return Voicing { ratio: None, is_speech: false };
    }

    // The loudest frame sets the silence floor: voicing is a property of the
    // sound, and the gaps between sounds must not vote.
    let rms: Vec<f64> = x
        .chunks(HOP)
        .map(|c| (c.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / c.len() as f64).sqrt())
        .collect();
    let peak = rms.iter().cloned().fold(0.0f64, f64::max);
    if peak <= 1e-9 {
        return Voicing { ratio: None, is_speech: false }; // digital silence
    }
    let floor = peak * SILENCE_FLOOR;

    let mut active = 0usize;
    let mut voiced = 0usize;

    let mut start = 0usize;
    while start + FRAME <= x.len() {
        let frame = &x[start..start + FRAME];
        let energy: f64 = frame.iter().map(|&v| (v as f64) * (v as f64)).sum();
        if (energy / FRAME as f64).sqrt() < floor {
            start += HOP;
            continue; // silence: not a vote either way
        }
        active += 1;

        if let Some((_lag, r)) = best_period(frame, lag_min, lag_max) {
            if r >= VOICED_THRESHOLD {
                voiced += 1;
            }
        }
        start += HOP;
    }

    if active == 0 {
        return Voicing { ratio: None, is_speech: false };
    }
    let ratio = voiced as f64 / active as f64;

    // Voiced *and* pulsing at the syllable rate. The RMS track above is already an
    // amplitude envelope at 100 Hz, which is exactly what the modulation spectrum
    // wants — so this costs one small FFT and no second pass over the audio.
    let syllabic = crate::morphology::syllabic_modulation(&rms, FRAME_RATE_HZ).unwrap_or(0.0);
    let is_speech = ratio >= SPEECH_RATIO_MIN && syllabic >= SPEECH_SYLLABIC_MIN;

    Voicing { ratio: Some(ratio), is_speech }
}

/// The fraction of non-silent frames that are periodic in the voice range.
pub fn voicing_ratio(data: &[f32], sr: u32) -> Option<f64> {
    voice_activity(data, sr).ratio
}

/// True when the file carries something that behaves like a voice.
pub fn has_voice(data: &[f32], sr: u32) -> bool {
    voice_activity(data, sr).is_speech
}

/// Peak of the normalized autocorrelation over the voice-f0 lag range, with the
/// lag that produced it. None when the frame has no energy.
///
/// Normalizing by *both* windows' energy (rather than by the frame's total) is
/// what keeps a decaying frame from looking aperiodic just because it is quieter
/// at the end than the start.
fn best_period(frame: &[f32], lag_min: usize, lag_max: usize) -> Option<(usize, f64)> {
    let mut best = (0usize, 0.0f64);
    for lag in lag_min..=lag_max.min(frame.len() - 1) {
        let n = frame.len() - lag;
        let mut num = 0.0f64;
        let mut e0 = 0.0f64;
        let mut e1 = 0.0f64;
        for i in 0..n {
            let a = frame[i] as f64;
            let b = frame[i + lag] as f64;
            num += a * b;
            e0 += a * a;
            e1 += b * b;
        }
        let denom = (e0 * e1).sqrt();
        if denom <= 1e-12 {
            continue;
        }
        let r = num / denom;
        if r > best.1 {
            best = (lag, r);
        }
    }
    (best.0 > 0).then_some(best)
}

/// Decimate to ~8 kHz by averaging blocks — the average is a crude anti-alias
/// filter, and without one the high frequencies would fold down into exactly the
/// band we are about to measure periodicity in.
fn decimate(data: &[f32], sr: u32) -> Vec<f32> {
    if data.is_empty() || sr == 0 {
        return Vec::new();
    }
    let factor = (sr as usize / TARGET_RATE).max(1);
    if factor == 1 {
        return data.to_vec();
    }
    data.chunks(factor)
        .map(|c| c.iter().sum::<f32>() / c.len() as f32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 44_100;

    fn samples(n: usize, f: impl Fn(f64) -> f64) -> Vec<f32> {
        (0..n).map(|i| f(i as f64 / SR as f64) as f32).collect()
    }

    fn noise(n: usize) -> Vec<f32> {
        let mut s = 987_654u32;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                ((s >> 8) as f32 / 8_388_608.0) - 1.0
            })
            .collect()
    }

    #[test]
    fn a_periodic_tone_is_voiced_and_noise_is_not() {
        // 120 Hz buzz, squarely in the voice range.
        let buzz = samples(SR as usize, |t| (2.0 * std::f64::consts::PI * 120.0 * t).sin());
        let v = voicing_ratio(&buzz, SR).unwrap();
        assert!(v > 0.9, "periodic tone voicing = {v}");

        let n = voicing_ratio(&noise(SR as usize), SR).unwrap();
        assert!(n < 0.2, "white noise voicing = {n}");
    }

    #[test]
    fn silence_has_no_voicing_rather_than_zero_voicing() {
        assert!(voicing_ratio(&vec![0.0f32; SR as usize], SR).is_none());
        assert!(voicing_ratio(&[], SR).is_none());
    }

    #[test]
    fn a_held_note_is_periodic_but_is_not_speech() {
        // This is the distinction WebRTC could not make. A cello holding a note is
        // as periodic as a vowel; only its *stillness* gives it away.
        let held = samples(SR as usize * 2, |t| {
            (2.0 * std::f64::consts::PI * 220.0 * t).sin()
                + 0.4 * (2.0 * std::f64::consts::PI * 440.0 * t).sin()
        });
        let v = voice_activity(&held, SR);
        assert!(v.ratio.unwrap() > 0.8, "a held note must still read as periodic");
        assert!(!v.is_speech, "a held note must not be called speech");
    }

    #[test]
    fn a_voice_pulsing_at_the_syllable_rate_is_speech() {
        // A 130 Hz buzz amplitude-modulated at 5 Hz: periodic in the voice range,
        // and pulsing at the syllable rate. That is the whole definition.
        let n = SR as usize * 3;
        let speechy: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f64 / SR as f64;
                let carrier = (2.0 * std::f64::consts::PI * 130.0 * t).sin()
                    + 0.5 * (2.0 * std::f64::consts::PI * 260.0 * t).sin();
                let syllable = 0.55 + 0.45 * (2.0 * std::f64::consts::PI * 5.0 * t).sin();
                (carrier * syllable) as f32
            })
            .collect();
        let v = voice_activity(&speechy, SR);
        assert!(v.ratio.unwrap() > 0.7, "voiced ratio = {:?}", v.ratio);
        assert!(v.is_speech, "voiced + syllabic must read as speech");
    }
}
