//! The first four statistical moments of a series — the standard way to
//! summarize how a frame-wise feature behaves over time:
//!   mean (average level), variance (fluctuation / modulation depth),
//!   skewness (asymmetry: a strongly positive-skewed amplitude envelope IS a
//!   percussive sound — all the energy up front, a long near-zero tail),
//!   kurtosis (tailedness: sharp isolated bursts within otherwise smooth data).

pub struct Moments {
    pub mean: f64,
    pub var: f64,
    pub skew: f64, // 0 for symmetric data; guarded to 0 when variance ≈ 0
    pub kurt: f64, // EXCESS kurtosis (normal distribution = 0)
}

pub fn moments(x: &[f64]) -> Moments {
    let n = x.len();
    if n == 0 {
        return Moments { mean: 0.0, var: 0.0, skew: 0.0, kurt: 0.0 };
    }
    let nf = n as f64;
    let mean = x.iter().sum::<f64>() / nf;
    let (mut m2, mut m3, mut m4) = (0.0f64, 0.0f64, 0.0f64);
    for &v in x {
        let d = v - mean;
        let d2 = d * d;
        m2 += d2;
        m3 += d2 * d;
        m4 += d2 * d2;
    }
    m2 /= nf;
    m3 /= nf;
    m4 /= nf;
    let (skew, kurt) = if m2 > 1e-18 {
        (m3 / m2.powf(1.5), m4 / (m2 * m2) - 3.0)
    } else {
        (0.0, 0.0)
    };
    Moments { mean, var: m2, skew, kurt }
}

#[cfg(test)]
mod tests {
    use super::moments;

    #[test]
    fn percussive_envelope_is_positively_skewed() {
        // Front-loaded decay (percussive): big values first, long tail of ~0.
        let perc: Vec<f64> = (0..1000).map(|i| (-(i as f64) / 50.0).exp()).collect();
        // Flat sustain: symmetric, no skew.
        let flat = vec![0.8f64; 1000];
        let p = moments(&perc);
        let f = moments(&flat);
        assert!(p.skew > 2.0, "percussive skew = {}", p.skew);
        assert!(f.skew.abs() < 1e-9 && f.var < 1e-12);
        assert!(p.kurt > f.kurt);
    }
}
