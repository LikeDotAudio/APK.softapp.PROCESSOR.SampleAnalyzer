/// Squared Euclidean distance between two equal-length feature vectors.
pub fn sqdist(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum()
}
