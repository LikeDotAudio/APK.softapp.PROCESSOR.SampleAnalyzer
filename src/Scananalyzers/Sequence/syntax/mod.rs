//! Bioacoustic syntax — the grammar of a multi-slice file.
//!
//! Every other extractor answers "what does this sound like". This one answers
//! "in what ORDER, and what sits in between". A field recording of a bird, a
//! spoken take, a drum loop, a machine cycling — none of them are one sound.
//! They are a sequence of sounds drawn from a small vocabulary, and the
//! information is as much in the sequence as in any single event.
//!
//! The pipeline already does the hard part twice over:
//!   * `regions::detect_regions` segments the file into sounding stretches
//!     (the "slices"/syllables), and
//!   * `analyze_core` re-runs the WHOLE analysis on each slice, so every slice
//!     arrives here with its own MFCCs, centroid, harmonicity and envelope.
//!
//! What was missing is everything downstream of that. This module adds it:
//!
//! 1. **Vocabulary.** Cluster the slices by their own acoustic features into
//!    *types* (A, B, C…) — the discrete neighborhoods of a syllable map. Deep
//!    bioacoustics work (Best 2023; Morales 2022) learns this embedding with a
//!    convolutional auto-encoder; here the embedding is the MFCC + envelope +
//!    spectral vector the analyzer already computes, standardized within the
//!    file. The clustering is agglomerative with an automatic cut, because the
//!    repertoire size is exactly the thing we do not know in advance.
//!
//! 2. **A 2-D layout.** The same vectors are projected through the analyzer's
//!    own PCA so the record carries an (x, y) per slice. That is the node
//!    layout of a grammar map — the analog of the UMAP scatter, computed with
//!    the deterministic linear projection this codebase already trusts.
//!
//! 3. **Syntax.** First-order transition probabilities between types, and the
//!    information-theoretic verdict on whether the ordering carries any
//!    information at all: mutual information between consecutive slices, in
//!    bits. Zero means the order is random and there is no grammar to find.
//!
//! 4. **The junctions — what is BETWEEN the slices.** The gap between two
//!    slices is not nothing. It has a level, a spectrum, and a slope, and its
//!    shape says whether the two slices are separate utterances or one gesture:
//!    a decaying tail means the first sound is still ringing into the second, a
//!    rising gap is a breath or a wind-up before the next onset, a flat gap well
//!    above the noise floor is a continuous bed that never stops. TweetyNet
//!    (Cohen 2022) makes the same move by giving its network an explicit
//!    "background" class rather than treating the quiet as absence. Every
//!    junction is measured and classified here, and carries the surprisal of
//!    the transition it spans.
//!
//! Cost: arithmetic over the STFT frames, the RMS envelope and the per-region
//! analyses that already exist. No new transform, and it only runs for files
//! that actually have more than one slice.
//!
//! ## Layout
//!
//! This file is the entry point and the assembly: it orders the slices, calls
//! each stage, and folds the results into the record. The stages live beside it:
//!
//!   * `record`     — the serialized shape (data only)
//!   * `tunables`   — every threshold, with the failure that set it
//!   * `embedding`  — the per-slice acoustic vector and its scaling
//!   * `clustering` — vocabulary discovery: which slices are the same word
//!   * `junctions`  — measuring and classifying the space between slices
//!   * `motifs`     — repeated phrases in the sequence
//!   * `mathutil`   — entropy, regularity, percentile, dB ratio

use std::collections::HashMap;

use crate::peak::Regions;

mod clustering;
mod embedding;
mod junctions;
mod mathutil;
mod motifs;
mod record;
mod tunables;

#[cfg(test)]
mod fixtures;

pub use junctions::{BREATH, CONTINUOUS, NOISE_BED, RESONANT_TAIL, SILENCE};
pub use record::{
    BioacousticSyntax, Junction, JunctionCount, SliceNode, SliceType, Transition,
};

use clustering::{build_slice_types, closest_type_separation, cluster_slices, type_label};
use embedding::{scale_slices, slice_vector};
use junctions::measure_junction;
use mathutil::{entropy, percentile, regularity};
use motifs::{longest_repeated_motif, truncate_sequence};
use tunables::ADEQUATE_TRANSITIONS_PER_PAIR;

/// Build the syntax record for one file.
///
/// `regions` must be the top-level regions with their per-region `analysis`
/// filled in (that is what carries each slice's own features). `frames` is the
/// shared STFT the rest of the pipeline uses, `envelope` the RMS amplitude
/// track at `envelope_rate_hz`. Returns `None` for anything with fewer than two
/// slices — one slice is a sound, not a sequence.
#[allow(clippy::too_many_arguments)]
pub fn bioacoustic_syntax(
    regions: &Regions,
    frames: &[Vec<f32>],
    sr_f: f64,
    n_fft: usize,
    hop: usize,
    envelope: &[f64],
    envelope_rate_hz: f64,
) -> Option<BioacousticSyntax> {
    // Chronological order is the whole premise — sort defensively rather than
    // trusting the detector's ordering.
    let mut order: Vec<usize> = (0..regions.regions.len()).collect();
    order.sort_by(|&a, &b| {
        regions.regions[a]
            .start_seconds
            .partial_cmp(&regions.regions[b].start_seconds)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if order.len() < 2 {
        return None;
    }

    // ---- vocabulary: embed, cluster, label.
    let vectors: Vec<Option<Vec<f64>>> =
        order.iter().map(|&i| slice_vector(&regions.regions[i])).collect();
    let measured: Vec<usize> = (0..order.len()).filter(|&k| vectors[k].is_some()).collect();

    // One scaled matrix, used by the clustering, the medoid search AND the 2-D
    // layout, so the map the UI draws is the same space the types were decided
    // in. `scaled[k]` is None for a slice with no analysis to read.
    let mut scaled: Vec<Option<Vec<f64>>> = vec![None; order.len()];
    let (assignment, embedding, type_separation) = if measured.len() >= 2 {
        let feats: Vec<Vec<f64>> =
            measured.iter().map(|&k| vectors[k].clone().unwrap()).collect();
        let rows = scale_slices(&feats);
        let clusters = cluster_slices(&rows);
        let separation = closest_type_separation(&clusters, &rows);
        let projected = crate::pca::project(&rows, 2);
        for (slot, &k) in measured.iter().enumerate() {
            scaled[k] = Some(rows[slot].clone());
        }

        // Lift the per-measured-slice results back onto every slice. A slice
        // without an analysis (an older record, or one too short to measure)
        // still belongs to the sequence — it just cannot be typed, so it gets
        // its own catch-all cluster rather than being silently dropped.
        let unmeasured_cluster = clusters.iter().copied().max().map_or(0, |m| m + 1);
        let mut assignment = vec![unmeasured_cluster; order.len()];
        let mut embedding = vec![(0.0, 0.0); order.len()];
        for (slot, &k) in measured.iter().enumerate() {
            assignment[k] = clusters[slot];
            embedding[k] = (
                projected[slot].first().copied().unwrap_or(0.0),
                projected[slot].get(1).copied().unwrap_or(0.0),
            );
        }
        (assignment, embedding, separation)
    } else {
        (vec![0usize; order.len()], vec![(0.0, 0.0); order.len()], 0.0)
    };

    // Relabel by descending frequency so "A" is always the commonest syllable.
    let cluster_count = assignment.iter().copied().max().unwrap_or(0) + 1;
    let mut counts = vec![0usize; cluster_count];
    for &c in &assignment {
        counts[c] += 1;
    }
    let mut ranking: Vec<usize> = (0..cluster_count).filter(|&c| counts[c] > 0).collect();
    // Ties broken by first appearance, so the labelling is deterministic.
    let first_seen: Vec<usize> = (0..cluster_count)
        .map(|c| assignment.iter().position(|&a| a == c).unwrap_or(usize::MAX))
        .collect();
    ranking.sort_by(|&a, &b| counts[b].cmp(&counts[a]).then(first_seen[a].cmp(&first_seen[b])));
    let mut rank_of = vec![0usize; cluster_count];
    for (rank, &c) in ranking.iter().enumerate() {
        rank_of[c] = rank;
    }
    let type_count = ranking.len();
    let types: Vec<usize> = assignment.iter().map(|&c| rank_of[c]).collect();
    let labels: Vec<String> = types.iter().map(|&t| type_label(t)).collect();
    let sequence: String = labels.concat();

    // ---- the vocabulary entries themselves.
    let slice_types = build_slice_types(regions, &order, &types, type_count, &scaled);

    // ---- the nodes.
    let slices: Vec<SliceNode> = order
        .iter()
        .enumerate()
        .map(|(k, &i)| SliceNode {
            region_index: regions.regions[i].index,
            type_label: labels[k].clone(),
            start_seconds: regions.regions[i].start_seconds,
            duration_seconds: regions.regions[i].duration_seconds,
            embedding_x: embedding[k].0,
            embedding_y: embedding[k].1,
        })
        .collect();

    // ---- transitions.
    let mut bigrams: HashMap<(usize, usize), BigramStats> = HashMap::new();
    let mut from_totals = vec![0usize; type_count];
    let mut to_totals = vec![0usize; type_count];
    for k in 0..order.len() - 1 {
        let (a, b) = (types[k], types[k + 1]);
        let previous = &regions.regions[order[k]];
        let next = &regions.regions[order[k + 1]];
        let entry = bigrams.entry((a, b)).or_default();
        entry.count += 1;
        entry.gap_total += (next.start_seconds - previous.end_seconds).max(0.0);
        entry.inter_onset_total += (next.start_seconds - previous.start_seconds).max(0.0);
        from_totals[a] += 1;
        to_totals[b] += 1;
    }
    let transition_total = (order.len() - 1) as f64;

    let mut transitions: Vec<Transition> = bigrams
        .iter()
        .map(|(&(a, b), s)| Transition {
            from_label: type_label(a),
            to_label: type_label(b),
            count: s.count,
            probability: s.count as f64 / from_totals[a].max(1) as f64,
            mean_gap_seconds: s.gap_total / s.count as f64,
            mean_inter_onset_seconds: s.inter_onset_total / s.count as f64,
        })
        .collect();
    transitions.sort_by(|x, y| {
        y.count
            .cmp(&x.count)
            .then(x.from_label.cmp(&y.from_label))
            .then(x.to_label.cmp(&y.to_label))
    });

    // ---- information.
    let repertoire_entropy_bits = entropy(&counts.iter().copied().filter(|&c| c > 0).collect::<Vec<_>>());
    let repertoire_entropy_normalized = if type_count > 1 {
        repertoire_entropy_bits / (type_count as f64).log2()
    } else {
        0.0
    };
    // H(next | current), averaged over the from-states by how often each occurs.
    let mut transition_entropy_bits = 0.0;
    for a in 0..type_count {
        if from_totals[a] == 0 {
            continue;
        }
        let row: Vec<usize> = (0..type_count)
            .filter_map(|b| bigrams.get(&(a, b)).map(|s| s.count))
            .collect();
        transition_entropy_bits += (from_totals[a] as f64 / transition_total) * entropy(&row);
    }
    // Measured against H(next) — the destination marginal — so the difference is
    // a true mutual information and can never come out negative.
    let destination_entropy_bits = entropy(&to_totals);
    let syntactic_information_bits = (destination_entropy_bits - transition_entropy_bits).max(0.0);
    let determinism = if destination_entropy_bits > 1e-9 {
        (syntactic_information_bits / destination_entropy_bits).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let repeat_count = bigrams.iter().filter(|(&(a, b), _)| a == b).map(|(_, s)| s.count).sum::<usize>();
    let repeat_ratio = repeat_count as f64 / transition_total;
    let distinct_bigrams = bigrams.len();
    let bigram_coverage = distinct_bigrams as f64 / (type_count * type_count).max(1) as f64;
    let transitions_per_type_pair = transition_total / (type_count * type_count).max(1) as f64;
    let (dominant_motif, dominant_motif_occurrences) = longest_repeated_motif(&sequence);

    // ---- the junctions: measure and classify what sits between the slices.
    let noise_floor = percentile(envelope, 0.05);
    let junctions: Vec<Junction> = (0..order.len() - 1)
        .map(|k| {
            let previous = &regions.regions[order[k]];
            let next = &regions.regions[order[k + 1]];
            let probability = bigrams
                .get(&(types[k], types[k + 1]))
                .map(|s| s.count as f64 / from_totals[types[k]].max(1) as f64)
                .unwrap_or(0.0);
            measure_junction(
                k,
                previous,
                next,
                &labels[k],
                &labels[k + 1],
                probability,
                noise_floor,
                frames,
                sr_f,
                n_fft,
                hop,
                envelope,
                envelope_rate_hz,
            )
        })
        .collect();

    let mut profile: HashMap<&str, usize> = HashMap::new();
    for j in &junctions {
        *profile.entry(j.junction_class.as_str()).or_insert(0) += 1;
    }
    let mut junction_profile: Vec<JunctionCount> = profile
        .iter()
        .map(|(&class, &count)| JunctionCount { junction_class: class.to_string(), count })
        .collect();
    junction_profile.sort_by(|x, y| {
        y.count.cmp(&x.count).then(x.junction_class.cmp(&y.junction_class))
    });
    let dominant_junction_class = junction_profile
        .first()
        .map(|c| c.junction_class.clone())
        .unwrap_or_default();
    let bound_ratio = junctions
        .iter()
        .filter(|j| j.junction_class == CONTINUOUS || j.junction_class == RESONANT_TAIL)
        .count() as f64
        / junctions.len().max(1) as f64;

    let mut gaps: Vec<f64> = junctions.iter().map(|j| j.gap_seconds).collect();
    gaps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median_gap_seconds = gaps[gaps.len() / 2];
    let inter_onsets: Vec<f64> = junctions.iter().map(|j| j.inter_onset_seconds).collect();
    let gap_regularity = regularity(&inter_onsets);

    // ---- the verdict.
    let alternation_ratio = 1.0 - repeat_ratio;
    let syntax_class = if order.len() < 3 {
        "Isolated"
    } else if type_count == 1 {
        if gap_regularity >= 0.8 { "Isochronous Repetition" } else { "Repetition" }
    } else if type_count == 2 && alternation_ratio >= 0.8 {
        "Alternation"
    } else if determinism >= 0.35 {
        "Structured"
    } else if determinism < 0.15 {
        "Stochastic"
    } else {
        "Variable"
    }
    .to_string();

    let adequacy = if transitions_per_type_pair >= ADEQUATE_TRANSITIONS_PER_PAIR {
        String::new()
    } else {
        format!(
            " (sparse: {:.1} transitions per type-pair — suggestive, not measured)",
            transitions_per_type_pair
        )
    };
    let motif_part = if dominant_motif_occurrences > 1 {
        format!(" · motif \"{}\" ×{}", dominant_motif, dominant_motif_occurrences)
    } else {
        String::new()
    };
    let reason = vec![
        format!(
            "1) {} slices over {} type{} ({}) — sequence {}{}",
            order.len(),
            type_count,
            if type_count == 1 { "" } else { "s" },
            if type_count > 1 {
                format!("separated by {:.1} JND", type_separation)
            } else {
                "one continuum, no vocabulary".to_string()
            },
            truncate_sequence(&sequence),
            motif_part
        ),
        format!(
            "2) {:.2} bits of syntactic information (determinism {:.0}%, H(next|current) {:.2} bits){}",
            syntactic_information_bits,
            determinism * 100.0,
            transition_entropy_bits,
            adequacy
        ),
        format!(
            "3) junctions mostly {} · median gap {:.0} ms · regularity {:.0}% · {:.0}% bound",
            if dominant_junction_class.is_empty() { "unmeasured" } else { &dominant_junction_class },
            median_gap_seconds * 1000.0,
            gap_regularity * 100.0,
            bound_ratio * 100.0
        ),
    ];

    Some(BioacousticSyntax {
        slice_count: order.len(),
        type_count,
        sequence,
        slices,
        slice_types,
        type_separation,
        transitions,
        junctions,
        junction_profile,
        dominant_junction_class,
        repertoire_entropy_bits,
        repertoire_entropy_normalized,
        transition_entropy_bits,
        syntactic_information_bits,
        determinism,
        repeat_ratio,
        distinct_bigrams,
        bigram_coverage,
        dominant_motif,
        dominant_motif_occurrences,
        median_gap_seconds,
        gap_regularity,
        bound_ratio,
        syntax_class,
        transitions_per_type_pair,
        reason,
    })
}

#[derive(Default)]
struct BigramStats {
    count: usize,
    gap_total: f64,
    inter_onset_total: f64,
}

#[cfg(test)]
mod tests {
    use super::fixtures::{analyze, envelope_for, sequence_of};
    use super::*;

    #[test]
    fn one_slice_has_no_syntax() {
        let r = sequence_of(&[0], 0.2, 0.3);
        let (env, rate) = envelope_for(&r, 1.0);
        assert!(bioacoustic_syntax(&r, &[], 44100.0, 2048, 512, &env, rate).is_none());
    }

    #[test]
    fn a_repeated_syllable_is_one_type() {
        let r = sequence_of(&[0, 0, 0, 0, 0, 0], 0.2, 0.3);
        let s = analyze(&r, 3.5);
        assert_eq!(s.type_count, 1, "one timbre repeated is one word: {}", s.sequence);
        assert_eq!(s.sequence, "AAAAAA");
        // Even spacing, so the delivery reads as metronomic.
        assert!(s.gap_regularity > 0.9, "regularity {}", s.gap_regularity);
        assert_eq!(s.syntax_class, "Isochronous Repetition");
        // One word carries no choice, so no information rides on the order.
        assert!(s.syntactic_information_bits < 1e-9);
    }

    #[test]
    fn surprisal_marks_the_rare_move() {
        // Seven A→A repeats and one departure to B: the departure is the news.
        let r = sequence_of(&[0, 0, 0, 0, 0, 0, 0, 0, 1], 0.2, 0.3);
        let s = analyze(&r, 5.0);
        assert_eq!(s.type_count, 2);
        let rare = s.junctions.last().expect("a junction");
        let common = &s.junctions[0];
        assert!(
            rare.surprisal_bits > common.surprisal_bits,
            "rare {} vs common {}",
            rare.surprisal_bits,
            common.surprisal_bits
        );
    }

    #[test]
    fn labels_are_assigned_by_frequency() {
        // Kind 1 appears once, kind 0 five times — so kind 0 must be "A".
        let r = sequence_of(&[1, 0, 0, 0, 0, 0], 0.2, 0.3);
        let s = analyze(&r, 3.5);
        assert_eq!(s.type_count, 2);
        assert_eq!(s.sequence, "BAAAAA");
        assert_eq!(s.slice_types[0].label, "A");
        assert_eq!(s.slice_types[0].occurrences, 5);
    }
}
