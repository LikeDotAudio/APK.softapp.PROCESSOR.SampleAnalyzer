//! oa_sample_analyzer — fast, parallel audio-sample analyzer.
//!
//! Walks a directory for audio files and, across a pool of worker threads (30 by
//! default), computes for each: length, a pitch estimate (autocorrelation), and
//! a spectral "complexity" (centroid + spread). Streams one JSON line per file
//! to stdout so a GUI can graph progress live, then writes the aggregate
//! `sample_cloud_data.PEAK` (each record includes the file NAME and FOLDER).
//!
//! Usage: oa_sample_analyzer <dir> [--out <path>] [--workers <n>] [--max-len <s>]
//!
//! The code is split one-purpose-per-module:
//!   peak         — the streamed/serialized record struct
//!   normalize    — file-name tokenizer for keyword matching
//!   categorize   — name/path → group + subgroup taxonomy (+ tests)
//!   decode / wav — audio decode to mono f32 (WAV via hound; MP3/FLAC/AIFF/
//!                  OGG/M4A via symphonia), lossy sources flagged
//!   acid         — ACID loop metadata (BPM + root) from the WAV chunk
//!   amplitude / pitch / spectrum / transients / sustain — feature extractors
//!   stft / flux / mfcc / framestats — frame-based spectral features
//!   envelope     — measured ADSR (attack/decay/sustain/release) + moments + shape
//!   morphology   — the UCS morphology axis (spec §4b): stationarity, spectral
//!                  entropy/tilt/band-limit, centroid & pitch sweep slopes,
//!                  syllabic modulation
//!   syntax       — bioacoustic syntax: cluster the file's regions into syllable
//!                  types, map the slice-to-slice transition probabilities, and
//!                  measure + classify the junctions between them
//!   partials     — overtone peak-picking → inharmonicity (harmonic vs metallic)
//!   distortion   — THD + clipping density → Clean / Dirty / Clipped
//!   moments      — mean / variance / skewness / kurtosis of a series
//!   timbre / label — feature+name → timbre and taxonomy labels
//!   tags / family / music_prod — multi-label acoustic types, sound-design roles,
//!                  Hornbostel-Sachs family, music-production role
//!   analyze      — orchestrates the extractors into one `Peak`
//!   feature_vec / sqdist / kmeans / cluster / pca — blind grouping + embedding
//!   args / discover / sidecar / stream / emit / run — CLI + orchestration

#[path = "Scananalyzers/Musical/acid.rs"]
pub mod acid;
#[path = "Scananalyzers/Musical/advanced_stats.rs"]
pub mod advanced_stats;
#[path = "Scananalyzers/Temporal/amplitude.rs"]
pub mod amplitude;
#[path = "Pipeline/analyze.rs"]
pub mod analyze;
#[path = "Core/args.rs"]
pub mod args;
#[path = "NameSorting/categorize.rs"]
pub mod categorize;
#[path = "Clustering/cluster.rs"]
pub mod cluster;
#[path = "Decoders/decode.rs"]
pub mod decode;
#[path = "Pipeline/discover.rs"]
pub mod discover;
#[path = "Scananalyzers/Spectral/distortion.rs"]
pub mod distortion;
#[cfg(not(target_arch = "wasm32"))]
#[path = "Encoders/db.rs"]
pub mod db;
#[path = "Encoders/emit.rs"]
pub mod emit;
#[path = "Encoders/manifest.rs"]
pub mod manifest;
#[path = "Scananalyzers/Temporal/envelope.rs"]
pub mod envelope;
#[path = "NameSorting/family.rs"]
pub mod family;
#[path = "Clustering/feature_vec.rs"]
pub mod feature_vec;
#[path = "Scananalyzers/Spectral/flux.rs"]
pub mod flux;
#[path = "Scananalyzers/Spectral/framestats.rs"]
pub mod framestats;
#[path = "Clustering/kmeans.rs"]
pub mod kmeans;
#[path = "NameSorting/label.rs"]
pub mod label;
#[path = "Scananalyzers/Spectral/mfcc.rs"]
pub mod mfcc;
#[path = "Scananalyzers/Musical/moments.rs"]
pub mod moments;
#[path = "Scananalyzers/Musical/morphology.rs"]
pub mod morphology;
#[path = "NameSorting/normalize.rs"]
pub mod normalize;
#[path = "Scananalyzers/Spectral/partials.rs"]
pub mod partials;
#[path = "Clustering/pca.rs"]
pub mod pca;
#[path = "Core/peak.rs"]
pub mod peak;
#[path = "Scananalyzers/Musical/pitch.rs"]
pub mod pitch;
#[path = "Scananalyzers/Musical/note_map.rs"]
pub mod note_map;
#[path = "Pipeline/preview.rs"]
pub mod preview;
#[path = "Scananalyzers/Musical/root.rs"]
pub mod root;
#[path = "Scananalyzers/Temporal/regions.rs"]
pub mod regions;
#[path = "Pipeline/run.rs"]
pub mod run;
#[path = "Encoders/sidecar.rs"]
pub mod sidecar;
#[path = "Scananalyzers/Spectral/spectrum.rs"]
pub mod spectrum;
#[path = "Clustering/sqdist.rs"]
pub mod sqdist;
#[path = "Scananalyzers/Spectral/stft.rs"]
pub mod stft;
#[path = "Encoders/stream.rs"]
pub mod stream;
#[path = "Scananalyzers/Temporal/sustain.rs"]
pub mod sustain;
#[path = "Scananalyzers/Sequence/syntax/mod.rs"]
pub mod syntax;
#[path = "Tagging/tags.rs"]
pub mod tags;
#[path = "Scananalyzers/Temporal/tempo.rs"]
pub mod tempo;
#[path = "Scananalyzers/Musical/timbre.rs"]
pub mod timbre;
#[path = "Scananalyzers/Temporal/transients.rs"]
pub mod transients;
#[path = "Core/version.rs"]
pub mod version;
#[path = "Scananalyzers/Musical/vad.rs"]
pub mod vad;
#[path = "Decoders/wav.rs"]
pub mod wav;
#[path = "NameSorting/ucs.rs"]
pub mod ucs;
