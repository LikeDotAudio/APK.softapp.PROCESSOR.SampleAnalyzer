//! Small numeric helpers shared across the syntax analysis.
//!
//! Each is a plain function over a slice of numbers with no knowledge of the
//! record — kept together because they are read as a group when checking the
//! information-theory figures, not because they are related to each other.

/// Shannon entropy of a count vector, in bits.
pub(super) fn entropy(counts: &[usize]) -> f64 {
    let total: usize = counts.iter().sum();
    if total == 0 {
        return 0.0;
    }
    let total = total as f64;
    -counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / total;
            p * p.log2()
        })
        .sum::<f64>()
}

/// `1 − coefficient of variation`, clamped to 0..1 — 1 is perfectly even.
pub(super) fn regularity(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    if mean <= 1e-9 {
        return 0.0;
    }
    let variance = values.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / n;
    (1.0 - variance.sqrt() / mean).clamp(0.0, 1.0)
}

/// `q`-quantile of an unsorted slice; the file's noise floor is read at q=0.05.
pub(super) fn percentile(values: &[f64], q: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v: Vec<f64> = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let i = ((v.len() - 1) as f64 * q.clamp(0.0, 1.0)).round() as usize;
    v[i]
}

/// 20·log10(a/b), floored so a silent numerator or denominator stays finite.
pub(super) fn ratio_decibels(a: f64, b: f64) -> f64 {
    20.0 * (a.max(1e-9) / b.max(1e-9)).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entropy_of_a_fair_coin_is_one_bit() {
        assert!((entropy(&[4, 4]) - 1.0).abs() < 1e-12);
        assert_eq!(entropy(&[7]), 0.0);
        assert_eq!(entropy(&[]), 0.0);
    }

    #[test]
    fn regularity_is_one_for_an_even_series() {
        assert!((regularity(&[0.5, 0.5, 0.5]) - 1.0).abs() < 1e-12);
        assert!(regularity(&[0.1, 2.0, 0.1]) < 0.5);
    }
}
