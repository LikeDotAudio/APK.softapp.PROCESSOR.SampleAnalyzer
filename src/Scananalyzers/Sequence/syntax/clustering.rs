//! Discovering the vocabulary: which slices are the same word.
//!
//! Agglomerative merging builds the whole dendrogram, every level of it is
//! scored, and the best-supported level wins — but only if it clears two
//! independent bars. "This file has one word" has to stay reachable, because it
//! is usually the truth.

use std::collections::HashMap;

use super::record::SliceType;
use super::tunables::{MAX_SLICE_TYPES, MINIMUM_SILHOUETTE, MINIMUM_TYPE_SEPARATION};
use crate::peak::Regions;

/// Complete-linkage agglomerative clustering, cut where the split is best
/// SUPPORTED rather than merely largest.
///
/// K-means is the wrong tool here: it needs `k` up front, and the repertoire
/// size is precisely the unknown. Agglomerative merging builds the whole
/// dendrogram instead, and every level of it is then scored by mean silhouette —
/// how much tighter each cluster is than its distance to the nearest other one.
/// The best-scoring level wins, and only if it clears `MINIMUM_SILHOUETTE`;
/// otherwise the slices are one continuum and the file has a single type. That
/// outcome is a real answer, not a failure, and it has to be reachable — a
/// repeated call sung six times is one word, not six.
///
/// Complete linkage (cluster distance = the FARTHEST pair) rather than single
/// linkage, because single linkage chains: one intermediate slice would fuse two
/// genuinely distinct syllable types into a smear.
///
/// Returns a cluster index per input row.
pub(super) fn cluster_slices(x: &[Vec<f64>]) -> Vec<usize> {
    let n = x.len();
    if n < 2 {
        return vec![0; n];
    }
    // Pairwise distances. `d` is consumed by the merge loop (complete linkage
    // rewrites it in place); `d0` is kept pristine for scoring the cuts.
    let mut d = vec![vec![0.0f64; n]; n];
    for i in 0..n {
        for j in (i + 1)..n {
            let dist = x[i]
                .iter()
                .zip(&x[j])
                .map(|(a, b)| (a - b) * (a - b))
                .sum::<f64>()
                .sqrt();
            d[i][j] = dist;
            d[j][i] = dist;
        }
    }
    let d0 = d.clone();

    let mut active: Vec<bool> = vec![true; n];
    let mut members: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    // One entry per level of the dendrogram: the partition standing BEFORE that
    // step's merge collapsed it. `levels[m]` therefore holds `n - m` clusters.
    let mut levels: Vec<Vec<usize>> = Vec::with_capacity(n - 1);
    let mut assignment: Vec<usize> = (0..n).collect();

    for _ in 0..(n - 1) {
        let (mut nearest, mut pair) = (f64::INFINITY, (0usize, 0usize));
        for i in 0..n {
            if !active[i] {
                continue;
            }
            for j in (i + 1)..n {
                if active[j] && d[i][j] < nearest {
                    nearest = d[i][j];
                    pair = (i, j);
                }
            }
        }
        levels.push(assignment.clone());

        let (i, j) = pair;
        let absorbed = std::mem::take(&mut members[j]);
        for &m in &absorbed {
            assignment[m] = i;
        }
        members[i].extend(absorbed);
        active[j] = false;
        for k in 0..n {
            if active[k] && k != i {
                let far = d[i][k].max(d[j][k]);
                d[i][k] = far;
                d[k][i] = far;
            }
        }
    }

    // Score every level and keep the best-supported one, provided it clears the
    // bar. Nothing clearing it means the slices are one continuum: one type.
    let mut best_score = f64::NEG_INFINITY;
    let mut cut: Option<usize> = None;
    for (m, partition) in levels.iter().enumerate() {
        let clusters = n - m;
        if !(2..=MAX_SLICE_TYPES).contains(&clusters) {
            continue;
        }
        let score = silhouette(&compact(partition), &d0);
        if score > best_score {
            best_score = score;
            cut = Some(m);
        }
    }

    match cut {
        // Both bars, because each catches what the other misses: silhouette
        // rejects a split into overlapping smears, separation rejects a split
        // into two crisp blobs that are nonetheless a hair apart.
        Some(m)
            if best_score >= MINIMUM_SILHOUETTE
                && closest_type_separation(&compact(&levels[m]), x)
                    >= MINIMUM_TYPE_SEPARATION =>
        {
            compact(&levels[m])
        }
        _ => vec![0; n],
    }
}

/// Distance between the two closest type centroids, in just-noticeable
/// differences (the units `scale_slices` leaves behind).
///
/// Silhouette is scale-invariant — it asks only whether the clusters are tight
/// RELATIVE to the space between them, so two blobs of pure quantization noise
/// score as well as a snare and a flute. This is the absolute companion: how far
/// apart the types actually are. Zero for a single type.
pub(super) fn closest_type_separation(assignment: &[usize], rows: &[Vec<f64>]) -> f64 {
    let k = assignment.iter().copied().max().map_or(0, |m| m + 1);
    if k < 2 || rows.is_empty() {
        return 0.0;
    }
    let d = rows[0].len();
    let mut sums = vec![vec![0.0f64; d]; k];
    let mut sizes = vec![0usize; k];
    for (i, row) in rows.iter().enumerate() {
        for j in 0..d {
            sums[assignment[i]][j] += row[j];
        }
        sizes[assignment[i]] += 1;
    }
    let centroids: Vec<Vec<f64>> = (0..k)
        .map(|c| sums[c].iter().map(|v| v / sizes[c].max(1) as f64).collect())
        .collect();
    let mut closest = f64::INFINITY;
    for a in 0..k {
        for b in (a + 1)..k {
            let dist = centroids[a]
                .iter()
                .zip(&centroids[b])
                .map(|(p, q)| (p - q) * (p - q))
                .sum::<f64>()
                .sqrt();
            closest = closest.min(dist);
        }
    }
    if closest.is_finite() {
        closest
    } else {
        0.0
    }
}

/// Mean silhouette of a partition: for each slice, how much closer it sits to
/// its own cluster than to the nearest other one, on a −1..1 scale. Singleton
/// clusters score 0 by convention — a cluster of one has no internal scatter to
/// compare against, and scoring it 1 would reward shattering the set into
/// one-word-per-slice, which is the exact failure this bar exists to catch.
fn silhouette(assignment: &[usize], d: &[Vec<f64>]) -> f64 {
    let n = assignment.len();
    let k = assignment.iter().copied().max().map_or(0, |m| m + 1);
    if k < 2 || n < 2 {
        return -1.0;
    }
    let mut sizes = vec![0usize; k];
    for &c in assignment {
        sizes[c] += 1;
    }

    let mut total = 0.0;
    for i in 0..n {
        // Total distance from i to each cluster, its own included.
        let mut sums = vec![0.0f64; k];
        for j in 0..n {
            if i != j {
                sums[assignment[j]] += d[i][j];
            }
        }
        let own = assignment[i];
        if sizes[own] < 2 {
            continue; // singleton: scores 0
        }
        let a = sums[own] / (sizes[own] - 1) as f64;
        let b = (0..k)
            .filter(|&c| c != own && sizes[c] > 0)
            .map(|c| sums[c] / sizes[c] as f64)
            .fold(f64::INFINITY, f64::min);
        let scale = a.max(b);
        if b.is_finite() && scale > 1e-12 {
            total += (b - a) / scale;
        }
    }
    total / n as f64
}

/// Renumber arbitrary group ids to a dense 0..k range.
fn compact(assignment: &[usize]) -> Vec<usize> {
    let mut map: HashMap<usize, usize> = HashMap::new();
    assignment
        .iter()
        .map(|&a| {
            let next = map.len();
            *map.entry(a).or_insert(next)
        })
        .collect()
}

/// "A".."Z" then "a".."z"; beyond that, a number. `MAX_SLICE_TYPES` keeps us in
/// the first stretch, but the fallback means no input can produce a collision.
pub(super) fn type_label(index: usize) -> String {
    if index < 26 {
        ((b'A' + index as u8) as char).to_string()
    } else if index < 52 {
        ((b'a' + (index - 26) as u8) as char).to_string()
    } else {
        format!("#{}", index)
    }
}

pub(super) fn build_slice_types(
    regions: &Regions,
    order: &[usize],
    types: &[usize],
    type_count: usize,
    standardized: &[Option<Vec<f64>>],
) -> Vec<SliceType> {
    (0..type_count)
        .map(|t| {
            let slots: Vec<usize> = (0..order.len()).filter(|&k| types[k] == t).collect();
            let mut duration = 0.0;
            let mut centroid = 0.0;
            let mut harmonicity = 0.0;
            let mut measured = 0.0;
            for &k in &slots {
                let region = &regions.regions[order[k]];
                duration += region.duration_seconds;
                if let Some(p) = region.analysis.as_ref() {
                    centroid += p.spectral_features.spectral_centroid_hz;
                    harmonicity += p.spectral_features.harmonicity;
                    measured += 1.0;
                }
            }
            let n = slots.len().max(1) as f64;
            let exemplar = medoid(&slots, standardized).unwrap_or(slots[0]);
            let mean_centroid = if measured > 0.0 { centroid / measured } else { 0.0 };
            let mean_harmonicity = if measured > 0.0 { harmonicity / measured } else { 0.0 };
            SliceType {
                label: type_label(t),
                descriptor: descriptor(duration / n, mean_centroid, mean_harmonicity),
                occurrences: slots.len(),
                share: slots.len() as f64 / order.len() as f64,
                exemplar_region_index: regions.regions[order[exemplar]].index,
                mean_duration_seconds: duration / n,
                mean_spectral_centroid_hz: mean_centroid,
                mean_harmonicity,
            }
        })
        .collect()
}

/// The member with the smallest total distance to its siblings — the most
/// typical example of the type, and a better exemplar than the centroid because
/// it is an actual slice you can audition.
fn medoid(slots: &[usize], standardized: &[Option<Vec<f64>>]) -> Option<usize> {
    let usable: Vec<usize> = slots.iter().copied().filter(|&k| standardized[k].is_some()).collect();
    if usable.is_empty() {
        return None;
    }
    usable
        .iter()
        .map(|&k| {
            let a = standardized[k].as_ref().unwrap();
            let total: f64 = usable
                .iter()
                .map(|&m| {
                    let b = standardized[m].as_ref().unwrap();
                    a.iter().zip(b).map(|(p, q)| (p - q) * (p - q)).sum::<f64>().sqrt()
                })
                .sum();
            (k, total)
        })
        .min_by(|x, y| x.1.partial_cmp(&y.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(k, _)| k)
}

/// A plain-language read of a type, from its own measurements only.
fn descriptor(duration_seconds: f64, centroid_hz: f64, harmonicity: f64) -> String {
    let length = if duration_seconds < 0.15 {
        "click"
    } else if duration_seconds < 0.5 {
        "short"
    } else if duration_seconds < 1.5 {
        "medium"
    } else {
        "long"
    };
    let brightness = if centroid_hz <= 0.0 {
        "unmeasured"
    } else if centroid_hz < 500.0 {
        "dark"
    } else if centroid_hz < 2000.0 {
        "warm"
    } else if centroid_hz < 6000.0 {
        "bright"
    } else {
        "airy"
    };
    let tonality = if harmonicity > 0.7 {
        "tonal"
    } else if harmonicity > 0.4 {
        "mixed"
    } else {
        "noisy"
    };
    format!("{} {} {}", length, brightness, tonality)
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{analyze, region, sequence_of};
    use crate::peak::Regions;

    #[test]
    fn two_timbres_split_into_two_types() {
        let r = sequence_of(&[0, 1, 0, 1, 0, 1, 0, 1], 0.2, 0.3);
        let s = analyze(&r, 4.5);
        assert_eq!(s.type_count, 2, "two distinct timbres: {}", s.sequence);
        assert_eq!(s.slice_types.len(), 2);
        // Strict alternation: knowing the current slice fully determines the next.
        assert!(s.determinism > 0.99, "determinism {}", s.determinism);
        assert_eq!(s.syntax_class, "Alternation");
        assert_eq!(s.repeat_ratio, 0.0);
        assert_eq!(s.distinct_bigrams, 2);
    }

    /// The same syllable, measured six times with the small wobble a segmenter
    /// introduces: region edges snap to envelope frames, so durations quantize
    /// and the discrete features (inharmonicity above all) twitch. Standardizing
    /// by the within-file spread turns exactly this into a tidy fake vocabulary,
    /// so it has to stay one type.
    #[test]
    fn segmenter_wobble_does_not_invent_a_vocabulary() {
        let mut regions = Vec::new();
        for i in 0..6 {
            let mut r = region(i, i as f64 * 0.5, 0.2, 0);
            let p = r.analysis.as_mut().unwrap();
            // Bimodal jitter — the worst case, and the shape frame quantization
            // actually produces: two crisp little blobs a hair apart.
            let wobble = if i % 2 == 0 { 1.0 } else { -1.0 };
            p.metadata.length_seconds += 0.002 * wobble;
            p.spectral_features.inharmonicity += 0.05 * wobble;
            p.spectral_features.spectral_centroid_hz += 6.0 * wobble;
            p.spectral_features.mel_frequency_cepstral_coefficients[1] += 0.3 * wobble;
            regions.push(r);
        }
        let regions = Regions { count: regions.len(), regions, ..Default::default() };
        let s = analyze(&regions, 3.2);
        assert_eq!(s.type_count, 1, "wobble became a vocabulary: {}", s.sequence);
        assert_eq!(s.type_separation, 0.0);
        assert_eq!(s.syntax_class, "Isochronous Repetition");
    }
}
