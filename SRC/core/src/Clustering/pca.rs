//! Principal Component Analysis — compresses the ~19-D acoustic feature space
//! down to the 3 axes of greatest variance, so the whole library can be laid
//! out on a plain X/Y(/Z) map where statistically similar samples land close
//! together. Deterministic (fixed-seed power iteration with deflation).
use crate::feature_vec::{feature_vec, standardize};
use crate::peak::Peak;

pub const N_COMPONENTS: usize = 3;

/// Compute the top principal components of the full result set and write each
/// sample's projected coordinates into `Peak::pca`.
pub fn pca_assign(results: &mut [Peak]) {
    let n = results.len();
    if n == 0 {
        return;
    }
    let feats: Vec<Vec<f64>> = results.iter().map(feature_vec).collect();
    if n < 2 {
        results[0].unsupervised.principal_components = vec![0.0; N_COMPONENTS];
        return;
    }

    // Z-score standardize each column (PCA on the correlation structure, so
    // Hz-scaled features don't drown the 0..1 ones) — shared with k-means so both
    // views share one geometry.
    let x = standardize(&feats);
    let coords = project(&x, N_COMPONENTS);
    for (i, row) in coords.into_iter().enumerate() {
        results[i].unsupervised.principal_components = row;
    }
}

/// Project already-standardized rows onto their own top `n_components`
/// principal axes. Split out of [`pca_assign`] so the same deterministic
/// projection lays out the library map AND the per-file slice map the
/// bioacoustic-syntax extractor draws — one geometry, one implementation.
///
/// Returns one coordinate vector per input row. Fewer than two rows have no
/// covariance to decompose, so they land at the origin.
pub fn project(x: &[Vec<f64>], n_components: usize) -> Vec<Vec<f64>> {
    let n = x.len();
    if n == 0 {
        return Vec::new();
    }
    let d = x[0].len();
    if n < 2 || d == 0 {
        return vec![vec![0.0; n_components]; n];
    }
    let nf = n as f64;

    // Covariance matrix C = XᵀX / (n-1).
    let mut cov = vec![vec![0.0f64; d]; d];
    for row in x {
        for j in 0..d {
            if row[j] == 0.0 {
                continue;
            }
            for l in j..d {
                cov[j][l] += row[j] * row[l];
            }
        }
    }
    for j in 0..d {
        for l in j..d {
            cov[j][l] /= nf - 1.0;
            cov[l][j] = cov[j][l];
        }
    }

    // Top components by power iteration + deflation (deterministic seed).
    let mut seed = 0xD1B54A32D192ED03u64;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / ((1u64 << 53) as f64) - 0.5
    };
    let mut components: Vec<Vec<f64>> = Vec::with_capacity(n_components);
    for _ in 0..n_components.min(d) {
        let mut v: Vec<f64> = (0..d).map(|_| rnd()).collect();
        normalize(&mut v);
        let mut lambda = 0.0f64;
        for _ in 0..200 {
            let mut w = matvec(&cov, &v);
            let norm = normalize(&mut w);
            let delta: f64 = w.iter().zip(&v).map(|(a, b)| (a - b).abs()).sum();
            v = w;
            lambda = norm;
            if delta < 1e-10 {
                break;
            }
        }
        // Deflate: C -= λ v vᵀ, so the next iteration finds the next component.
        for j in 0..d {
            for l in 0..d {
                cov[j][l] -= lambda * v[j] * v[l];
            }
        }
        components.push(v);
    }

    x.iter()
        .map(|row| {
            components
                .iter()
                .map(|c| c.iter().zip(row).map(|(a, b)| a * b).sum())
                .collect()
        })
        .collect()
}

fn matvec(m: &[Vec<f64>], v: &[f64]) -> Vec<f64> {
    m.iter().map(|row| row.iter().zip(v).map(|(a, b)| a * b).sum()).collect()
}

/// Normalize in place; returns the pre-normalization L2 norm.
fn normalize(v: &mut [f64]) -> f64 {
    let n: f64 = v.iter().map(|x| x * x).sum::<f64>().sqrt();
    if n > 1e-12 {
        for x in v.iter_mut() {
            *x /= n;
        }
    }
    n
}
