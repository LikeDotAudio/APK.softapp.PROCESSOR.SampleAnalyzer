//! Distortion analysis — non-linear alteration of the signal, measured three
//! ways (the third, crest factor, is computed in `amplitude.rs`):
//!
//! * **THD** (Total Harmonic Distortion): the power of everything at integer
//!   multiples of the fundamental vs the fundamental itself,
//!   `sqrt(V2² + … + Vn²) / V1`. Clipping a tone generates artificial
//!   overtones, so squashed/fuzzed tonal material scores high. (A clean saw
//!   wave is also harmonically dense — THD is really *harmonic density*, which
//!   is exactly what separates a clean sine bass from a fuzz bass.)
//! * **Clipping density**: the brute-force count — the fraction of samples
//!   pinned at the waveform ceiling in flat-top runs. A clean waveform only
//!   *touches* its peak for a sample or two per cycle; hard-clipped audio sits
//!   there for runs at a time.
//! * A categorical label: Clean / Dirty / Clipped.
use rustfft::{num_complex::Complex, FftPlanner};

pub struct Distortion {
    pub thd: f64,      // total harmonic distortion ratio (0 = pure tone; >1 = harmonics outweigh the fundamental)
    pub clipping: f64, // fraction of samples pinned at the ceiling in flat-top runs
    pub label: &'static str,
}

/// Measure THD (needs a fundamental `f0`; 0.0 when unpitched) and clipping
/// density, then label the file Clean / Dirty / Clipped.
pub fn distortion_analysis(data: &[f32], sr_f: f64, f0: f64, crest: f64) -> Distortion {
    let thd = measure_thd(data, sr_f, f0);
    // A THD beyond ~3 means the fundamental estimate itself is wrong (swept or
    // unpitched material) — real distorted signals measure well under that.
    // Report 0 (unmeasurable) rather than a junk number that would poison the
    // Dirty label and the clustering features.
    let thd = if thd > 3.0 { 0.0 } else { thd };
    let clipping = clipping_density(data);
    // Reference points on THIS meter (harmonics 2–10, Hann window): a square
    // wave measures ≈0.40, a saw ≈0.7, a pure sine ~0 — so heavy waveshaping
    // lands at ≥0.35, and combined with a squashed crest factor that is the
    // "fuzz" signature.
    let label = if clipping >= 0.01 {
        "Clipped" // ≥1 % of all samples are sitting on the ceiling
    } else if clipping >= 0.001 || (thd >= 0.35 && crest < 3.0) {
        "Dirty" // saturated: pinned runs, or harmonic-dense AND dynamically squashed
    } else {
        "Clean"
    };
    Distortion { thd, clipping, label }
}

/// THD via one Hann-windowed FFT over the middle of the signal: V1 is the
/// strongest bin within ±4 % of `f0`, V2…V10 within ±4 % of each multiple.
fn measure_thd(data: &[f32], sr_f: f64, f0: f64) -> f64 {
    if f0 <= 0.0 {
        return 0.0;
    }
    let n = data.len().min(65_536);
    if n < 2048 || f0 * 2.0 >= sr_f / 2.0 {
        return 0.0;
    }
    let start = (data.len().saturating_sub(n)) / 2;
    let mut buf: Vec<Complex<f32>> = (0..n)
        .map(|i| {
            let w = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (n as f32 - 1.0)).cos();
            Complex { re: data[start + i] * w, im: 0.0 }
        })
        .collect();
    FftPlanner::<f32>::new().plan_fft_forward(n).process(&mut buf);
    let half = n / 2;
    let bin_hz = sr_f / n as f64;

    // Strongest magnitude within ±4 % of a target frequency.
    let peak_near = |f: f64| -> f64 {
        let lo = ((f * 0.96 / bin_hz) as usize).max(1);
        let hi = (((f * 1.04 / bin_hz) as usize).max(lo + 1)).min(half - 1);
        buf[lo..=hi].iter().map(|c| c.norm() as f64).fold(0.0, f64::max)
    };

    let v1 = peak_near(f0);
    if v1 <= 1e-9 {
        return 0.0;
    }
    let mut sum_sq = 0.0f64;
    for k in 2..=10 {
        let fk = f0 * k as f64;
        if fk >= sr_f / 2.0 {
            break;
        }
        let vk = peak_near(fk);
        sum_sq += vk * vk;
    }
    (sum_sq.sqrt() / v1).min(10.0)
}

/// Fraction of samples pinned at the waveform ceiling: runs of ≥4 consecutive
/// samples within ~2 LSB (16-bit) of the file's own peak. A clean sine only
/// grazes its peak for a couple of samples per cycle; flat tops sit there.
fn clipping_density(data: &[f32]) -> f64 {
    let peak = data.iter().map(|x| x.abs()).fold(0.0f32, f32::max);
    if peak <= 0.0 || data.len() < 8 {
        return 0.0;
    }
    let thr = peak - 2.0 / 32768.0;
    const MIN_RUN: usize = 4;
    let mut pinned = 0usize;
    let mut run = 0usize;
    for &x in data {
        if x.abs() >= thr {
            run += 1;
        } else {
            if run >= MIN_RUN {
                pinned += run;
            }
            run = 0;
        }
    }
    if run >= MIN_RUN {
        pinned += run;
    }
    pinned as f64 / data.len() as f64
}

#[cfg(test)]
mod tests {
    use super::distortion_analysis;

    const SR: f64 = 44_100.0;
    const F0: f64 = 110.0;

    fn shaped(gain: f32, clip: bool, soft: bool, fade: bool) -> Vec<f32> {
        (0..44_100)
            .map(|i| {
                let s = gain * (2.0 * std::f32::consts::PI * F0 as f32 * i as f32 / SR as f32).sin();
                let s = if clip { s.clamp(-0.9, 0.9) } else if soft { s.tanh() } else { s };
                // A slight fade keeps later cycles off the file's global peak,
                // so nothing sits in pinned runs (saturated but not clipped).
                if fade {
                    s * (1.0 - 0.1 * i as f32 / 44_100.0)
                } else {
                    s
                }
            })
            .collect()
    }

    #[test]
    fn clean_vs_saturated_vs_clipped() {
        // Pure sine: near-zero THD, no pinned runs.
        let clean = distortion_analysis(&shaped(0.9, false, false, false), SR, F0, 1.414);
        assert_eq!(clean.label, "Clean");
        assert!(clean.thd < 0.05, "clean thd = {}", clean.thd);
        assert!(clean.clipping < 1e-4, "clean clipping = {}", clean.clipping);

        // Hard-clipped at 3× gain: flat tops + strong odd harmonics.
        let clipped = distortion_analysis(&shaped(3.0, true, false, false), SR, F0, 1.1);
        assert_eq!(clipped.label, "Clipped");
        assert!(clipped.clipping > 0.05, "clipped density = {}", clipped.clipping);
        assert!(clipped.thd > 0.2, "clipped thd = {}", clipped.thd);

        // Soft-saturated (tanh at 10× gain, slight fade): near-square wave —
        // harmonic-dense and squashed, but nothing pinned at the ceiling.
        let dirty = distortion_analysis(&shaped(10.0, false, true, true), SR, F0, 1.1);
        assert_eq!(dirty.label, "Dirty");
        assert!(dirty.thd > 0.35, "dirty thd = {}", dirty.thd);
        assert!(dirty.clipping < 0.01, "dirty density = {}", dirty.clipping);
    }
}
