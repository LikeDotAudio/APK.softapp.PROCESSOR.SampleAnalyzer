//! Partial (overtone) analysis — how well the sound's spectral peaks line up
//! with integer multiples of the fundamental. Harmonic sounds (voice, flute,
//! guitar) sit near 0; metallic / bell-like sounds (cymbals, gongs, chimes)
//! whose partials fall between the harmonics score high.
use rustfft::{num_complex::Complex, FftPlanner};

pub struct Partials {
    pub count: usize,        // detected spectral peaks (0 = none / no fundamental)
    pub inharmonicity: f64,  // 0 = exact integer harmonics … 1 = maximally detuned
}

impl Partials {
    fn none() -> Partials {
        Partials { count: 0, inharmonicity: 0.0 }
    }
}

/// Detect the strongest spectral peaks above the fundamental `f0` and measure
/// their magnitude-weighted deviation from the nearest integer multiple of
/// `f0`. A deviation of half a harmonic spacing (the farthest a partial can be
/// from any integer multiple) maps to 1.0.
pub fn partial_analysis(data: &[f32], sr_f: f64, f0: f64) -> Partials {
    if f0 <= 0.0 {
        return Partials::none();
    }
    let n = data.len().min(65_536);
    if n < 2048 {
        return Partials::none();
    }
    // Hann-windowed FFT over the middle of the signal.
    let start = (data.len().saturating_sub(n)) / 2;
    let mut buf: Vec<Complex<f32>> = (0..n)
        .map(|i| {
            let w = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (n as f32 - 1.0)).cos();
            Complex { re: data[start + i] * w, im: 0.0 }
        })
        .collect();
    FftPlanner::<f32>::new().plan_fft_forward(n).process(&mut buf);
    let half = n / 2;
    let mags: Vec<f64> = buf[..half].iter().map(|c| c.norm() as f64).collect();
    let bin_hz = sr_f / n as f64;

    // Peak-pick from just above f0 up to the 16th harmonic (or Nyquist):
    // local maxima at least 5% of the global max, separated by ≥ f0/2.
    let max_mag = mags.iter().cloned().fold(0.0f64, f64::max);
    if max_mag <= 0.0 {
        return Partials::none();
    }
    let thresh = max_mag * 0.05;
    let lo = ((f0 * 1.5 / bin_hz) as usize).max(2);
    let hi = ((f0 * 16.5 / bin_hz) as usize).min(half - 2);
    if hi <= lo {
        return Partials::none();
    }
    let min_sep = ((f0 * 0.5 / bin_hz) as usize).max(2);

    // (frequency, magnitude) of each accepted peak, strongest kept on clashes.
    let mut peaks: Vec<(f64, f64)> = Vec::new();
    for k in lo..hi {
        let m = mags[k];
        if m < thresh || m <= mags[k - 1] || m < mags[k + 1] {
            continue;
        }
        // Parabolic interpolation for a sub-bin frequency estimate.
        let (a, b, c) = (mags[k - 1], mags[k], mags[k + 1]);
        let denom = a - 2.0 * b + c;
        let delta = if denom.abs() > 1e-12 { (0.5 * (a - c) / denom).clamp(-0.5, 0.5) } else { 0.0 };
        let f = (k as f64 + delta) * bin_hz;
        match peaks.last() {
            Some(&(pf, pm)) if (f - pf) / bin_hz < min_sep as f64 => {
                if m > pm {
                    *peaks.last_mut().unwrap() = (f, m);
                }
            }
            _ => peaks.push((f, m)),
        }
    }
    if peaks.is_empty() {
        return Partials::none();
    }
    // Keep the 12 strongest.
    peaks.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap_or(std::cmp::Ordering::Equal));
    peaks.truncate(12);

    let mut wsum = 0.0f64;
    let mut dsum = 0.0f64;
    for &(f, m) in &peaks {
        let ratio = f / f0;
        let dev = (ratio - ratio.round()).abs(); // 0 … 0.5 harmonic spacings off
        dsum += dev * m;
        wsum += m;
    }
    let inharmonicity = if wsum > 0.0 { (dsum / wsum * 2.0).clamp(0.0, 1.0) } else { 0.0 };
    Partials { count: peaks.len(), inharmonicity }
}

#[cfg(test)]
mod tests {
    use super::partial_analysis;

    fn tone(partial_ratios: &[f32], f0: f32, secs: f32) -> Vec<f32> {
        let sr = 44_100.0f32;
        (0..(sr * secs) as usize)
            .map(|i| {
                let t = i as f32 / sr;
                partial_ratios
                    .iter()
                    .enumerate()
                    .map(|(k, &r)| (2.0 * std::f32::consts::PI * f0 * r * t).sin() / (k + 1) as f32)
                    .sum::<f32>()
            })
            .collect()
    }

    #[test]
    fn harmonic_series_scores_low_bell_scores_high() {
        let f0 = 220.0;
        let harmonic = tone(&[1.0, 2.0, 3.0, 4.0, 5.0], f0, 1.0);
        // Bell-like partial ratios (roughly a church bell's).
        let bell = tone(&[1.0, 2.14, 3.38, 4.72, 5.61], f0, 1.0);
        let h = partial_analysis(&harmonic, 44_100.0, f0 as f64);
        let b = partial_analysis(&bell, 44_100.0, f0 as f64);
        assert!(h.count >= 3);
        assert!(h.inharmonicity < 0.1, "harmonic inharm = {}", h.inharmonicity);
        assert!(b.inharmonicity > 0.25, "bell inharm = {}", b.inharmonicity);
    }
}
