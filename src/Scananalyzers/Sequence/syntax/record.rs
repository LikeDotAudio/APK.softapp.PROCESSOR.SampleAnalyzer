//! The serialized shape of the syntax analysis — what lands in the `.PEAK`.
//!
//! Data only: every field is filled by `super::bioacoustic_syntax`, and the
//! doc comment on each says what it means and, where the distinction matters,
//! what its absence means.

use serde::{Deserialize, Serialize};

/// One slice, placed. The node of the grammar map: which type it belongs to and
/// where it sits in the file's own acoustic embedding.
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct SliceNode {
    pub region_index: usize,
    pub type_label: String,
    pub start_seconds: f64,
    pub duration_seconds: f64,
    /// Position in the file's 2-D acoustic projection — the layout coordinates
    /// for drawing the map. Arbitrary units, centered near zero.
    pub embedding_x: f64,
    pub embedding_y: f64,
}

/// One entry in the vocabulary — a cluster of acoustically similar slices.
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct SliceType {
    /// "A", "B", "C"… assigned by descending frequency, so "A" is always the
    /// most-used syllable and sequences read consistently across files.
    pub label: String,
    /// A plain-language read of the medoid: length, brightness, tonality.
    pub descriptor: String,
    pub occurrences: usize,
    /// Share of all slices, 0..1.
    pub share: f64,
    /// The region index of the medoid — the most typical member, the one to
    /// audition or draw as this type's exemplar.
    pub exemplar_region_index: usize,
    pub mean_duration_seconds: f64,
    pub mean_spectral_centroid_hz: f64,
    pub mean_harmonicity: f64,
}

/// One edge of the grammar map: how likely `from` is followed by `to`.
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Transition {
    pub from_label: String,
    pub to_label: String,
    pub count: usize,
    /// P(to | from) — row-normalized, so the edges leaving one node sum to 1.
    pub probability: f64,
    pub mean_gap_seconds: f64,
    pub mean_inter_onset_seconds: f64,
}

/// The measured, classified space BETWEEN two consecutive slices.
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Junction {
    pub index: usize,
    pub from_region_index: usize,
    pub to_region_index: usize,
    pub from_label: String,
    pub to_label: String,
    /// End of the previous slice to the start of the next.
    pub gap_seconds: f64,
    /// Onset to onset — the interval bioacoustics actually reports, because it
    /// is the one that survives a disagreement about where a syllable ends.
    pub inter_onset_seconds: f64,
    pub gap_root_mean_square_level: f64,
    /// How far the gap sits below the preceding slice's peak. Small means the
    /// sound never really stopped.
    pub gap_level_below_previous_peak_decibels: f64,
    /// How far the gap sits above the file's own quietest moment. `None` when
    /// the file has been edited to digital silence and so HAS no noise floor —
    /// dividing by that zero produced readings like "+119 dB above the floor",
    /// which is not a measurement of anything.
    pub gap_level_above_noise_floor_decibels: Option<f64>,
    /// Negative = falling (a tail ringing out), positive = rising (a breath or
    /// wind-up into the next onset), ~0 = a steady bed.
    pub gap_slope_decibels_per_second: f64,
    pub gap_spectral_centroid_hz: f64,
    pub gap_spectral_flatness: f64,
    /// Silence / Noise Bed / Resonant Tail / Breath / Continuous.
    pub junction_class: String,
    /// P(to | from) for the transition this junction spans.
    pub transition_probability: f64,
    /// −log2(probability): how surprising this particular move was, in bits. A
    /// rare transition in an otherwise rigid song is where the interesting
    /// behavior lives.
    pub surprisal_bits: f64,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct JunctionCount {
    pub junction_class: String,
    pub count: usize,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct BioacousticSyntax {
    pub slice_count: usize,
    pub type_count: usize,
    /// The whole file as one string, one letter per slice: "ABABACAB".
    pub sequence: String,
    pub slices: Vec<SliceNode>,
    pub slice_types: Vec<SliceType>,
    /// How far apart the two closest types sit, in just-noticeable differences.
    /// The confidence behind `type_count`: a big number means the vocabulary is
    /// obvious, a number near the cut-off means the split is arguable. 0 when
    /// the file has a single type.
    pub type_separation: f64,
    pub transitions: Vec<Transition>,
    pub junctions: Vec<Junction>,
    pub junction_profile: Vec<JunctionCount>,
    pub dominant_junction_class: String,

    /// Shannon entropy of the type distribution, in bits — how evenly the
    /// repertoire is used. 0 = one type only.
    pub repertoire_entropy_bits: f64,
    /// The same, over its own maximum (log2 of the type count), 0..1.
    pub repertoire_entropy_normalized: f64,
    /// H(next | current): the average uncertainty remaining about the next
    /// slice once the current one is known.
    pub transition_entropy_bits: f64,
    /// H(next) − H(next | current): the mutual information between consecutive
    /// slices. THE number. Zero bits means the order carries no information —
    /// the sequence is a bag of sounds, not a grammar.
    pub syntactic_information_bits: f64,
    /// Syntactic information over H(next), 0..1. 0 = order is random,
    /// 1 = each slice fully determines the next.
    pub determinism: f64,
    /// Fraction of transitions that stay on the same type (A→A).
    pub repeat_ratio: f64,
    pub distinct_bigrams: usize,
    /// Distinct bigrams over the type_count² possible ones. A low coverage with
    /// plenty of transitions is a strict grammar: most moves are never made.
    pub bigram_coverage: f64,
    /// The longest slice phrase that occurs more than once, and how often.
    pub dominant_motif: String,
    pub dominant_motif_occurrences: usize,

    pub median_gap_seconds: f64,
    /// 1 − coefficient of variation of the inter-onset intervals, 0..1. Near 1
    /// is metronomic delivery (a trill, a loop, a machine); near 0 is free.
    pub gap_regularity: f64,
    /// Fraction of junctions where the two slices are acoustically bound
    /// (a tail or no real break) rather than genuinely separated.
    pub bound_ratio: f64,

    /// The verdict: Isolated / Repetition / Isochronous Repetition /
    /// Alternation / Structured / Variable / Stochastic.
    pub syntax_class: String,
    /// Transitions per possible type-pair — the sample adequacy behind the
    /// entropy figures. Below ~2 the table is sparse and the numbers are
    /// suggestive rather than measured.
    pub transitions_per_type_pair: f64,
    pub reason: Vec<String>,
}
