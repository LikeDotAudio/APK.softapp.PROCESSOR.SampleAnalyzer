//! Every threshold the syntax analysis turns on, in one place with its reason.
//!
//! These are gathered rather than scattered because most of them were set by a
//! specific failure — the comments record which one, so a future change knows
//! what it is trading away.

/// Ceiling on the repertoire. A "vocabulary" where nearly every slice is its own
/// word is not a vocabulary, it is a failure to cluster; and a grammar map with
/// more than a dozen node colors stops being readable. Birdsong repertoires in
/// the literature sit comfortably inside this.
pub(super) const MAX_SLICE_TYPES: usize = 12;

/// Mean silhouette a candidate split must reach before the file is credited with
/// more than one syllable type. 0.5 is Kaufman & Rousseeuw's "reasonable
/// structure" line, and the bar exists because the alternative — cutting the
/// dendrogram at its biggest jump and taking whatever falls out — invents a
/// repertoire out of nothing: standardizing within one file rescales even
/// microscopic differences to unit variance, so six recordings of the SAME
/// syllable produce a tidy-looking five-way split. Requiring the clusters to be
/// genuinely tighter than the gaps between them is what makes "this file has one
/// word" an available answer.
pub(super) const MINIMUM_SILHOUETTE: f64 = 0.5;

/// How far apart two type centroids must sit, in just-noticeable differences
/// (see `SLICE_FEATURE_FLOORS`), before they count as different words. Two
/// syllables that differ by less than two JNDs summed across twenty features
/// are the same syllable sung twice.
pub(super) const MINIMUM_TYPE_SEPARATION: f64 = 2.0;

/// A gap whose level sits within this many dB of the preceding slice's peak was
/// never really a silence — the gate dipped, the sound did not stop.
pub(super) const BOUND_WITHIN_DECIBELS: f64 = 12.0;

/// Total level change ACROSS the gap (slope × duration) that counts as a real
/// decay or a real rise. Expressed as a total rather than a rate because a
/// −3 dB/s slope over a 150 ms gap is 0.45 dB — indistinguishable from noise,
/// while the same slope over two seconds is a plain audible fade.
pub(super) const JUNCTION_CHANGE_DECIBELS: f64 = 3.0;

/// Below this level, measured at the END of the gap, nothing has survived to
/// meet the next slice and the junction is a silence whatever shape the residue
/// had on the way down. Absolute (dBFS), not relative to the file's own floor,
/// because in a digitally-silent edit that floor is zero and *everything* would
/// read as "above" it. −60 dBFS puts editing dither and quantization residue on
/// the silent side and a real room tone on the sounding side.
pub(super) const AUDIBLE_AT_ONSET_DECIBELS_FULL_SCALE: f64 = -60.0;

/// The share of the gap, at its end, read as "what is still present when the
/// next slice begins".
pub(super) const ONSET_APPROACH_FRACTION: f64 = 0.25;

/// An envelope level at or below this (≈−120 dBFS) is a hard-edited zero, not a
/// quiet noise floor. Anything measured *relative* to it is undefined.
pub(super) const DIGITAL_SILENCE: f64 = 1e-6;

/// Longest phrase the motif search will look for, in slices.
pub(super) const MAXIMUM_MOTIF_LENGTH: usize = 16;

/// Below this many transitions per possible type-pair, the transition table is
/// too sparse for its entropy to mean much, and the record says so.
pub(super) const ADEQUATE_TRANSITIONS_PER_PAIR: f64 = 2.0;
