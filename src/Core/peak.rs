//! The per-file analysis record: streamed to the GUI (one JSON line each) and
//! written to the aggregate `sample_cloud_data.PEAK` / per-file sidecars.
//!
//! Field names are deliberately spelled out in full English — the .PEAK file
//! is a data model others read, so nothing is abbreviated or implied.
use serde::{Deserialize, Deserializer, Serialize};

/// −70 LUFS is the agreed "unmeasurable" sentinel — `ucs::feature()` reads anything
/// at or below −69 as "no loudness here" rather than as a very quiet one.
fn lufs_unmeasurable() -> f64 {
    -70.0
}

/// A digitally silent file measures as −∞ LUFS, and serde_json writes a non-finite
/// float as `null`. Deserializing that back into an `f64` fails, so the analyzer could
/// not read the sidecars it had written itself — every silent sample became an
/// unreadable record. Read `null` (or any non-finite value) as the sentinel instead.
fn lufs_from_json<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let v = Option::<f64>::deserialize(d)?;
    Ok(v.filter(|x| x.is_finite()).unwrap_or_else(lufs_unmeasurable))
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Metadata {
    pub analyzer_version: String,
    pub name: String,
    pub folder: String,
    pub sub: String,
    pub path: String,
    pub length_seconds: f64,
    pub sample_rate: u32,
    pub bit_depth: u16,
    pub channels: u16,
    pub source_format: String,
    pub lossy_source: bool,
    pub dc_offset: f64,
    pub trailing_silence_ms: f64,
    // How deep the analysis went: "full" (the whole DSP pipeline) or "preview_only"
    // (a file over the full-analysis cap — metadata + waveform preview, no DSP), so a
    // future deep-analyze pass can find the shallow records. Old sidecars read as ""
    // and mean "full".
    #[serde(default)]
    pub analysis_depth: String,
}

// Read leniently. A .PEAK on disk may predate any field added since it was written —
// the browser has always migrated those, and the engine refusing to is why a whole
// SFX library was unreadable to its own analyzer. Both structs derive Default, so a
// missing field reads as empty rather than failing the entire record.
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Classification {
    pub group: String,
    pub reason: Vec<String>,
    pub timbre: String,
    pub length_class: String,
    pub subgroup: String,
    pub audit: bool,
    pub acoustic_types: Vec<String>,
    pub sound_design_roles: Vec<String>,
    pub instrument_family: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Ucs {
    pub category: String,
    pub subcategory: String,
    pub id: String,
    pub confidence: f64,
    pub alternatives: Vec<crate::ucs::Alternative>,
    /// The synonyms that won the match — the words found in the file or folder name
    /// that belong to the chosen subcategory. Empty means no name evidence at all:
    /// the verdict rests on acoustic signal alone.
    #[serde(default)]
    pub synonyms: Vec<String>,
    pub reason: String,
}

/// One transient's ADSR, measured against that slice's own peak.
///
/// Every field here is unconditionally valid: a slice holds exactly one event by
/// construction, which is the precondition the file-level scalars cannot meet.
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct EnvelopeSlice {
    pub index: usize,
    pub start_seconds: f64,
    pub end_seconds: f64,
    /// This slice's peak as a fraction of the file's loudest frame (0..1). The
    /// slice at 1.0 is the one the file-level reading is taken from.
    pub relative_level: f64,
    pub envelope_attack_seconds: f64,
    pub envelope_decay_seconds: f64,
    pub envelope_sustain_level: f64,
    pub envelope_release_seconds: f64,
    pub envelope_temporal_centroid: f64,
    pub envelope_skewness: f64,
    pub envelope_kurtosis: f64,
    pub envelope_shape: String,
    pub decay_time_seconds_60db: Option<f64>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Envelope {
    pub transient_count: usize,
    pub attack_seconds: f64,
    pub sustain_ratio: f64,
    pub sustained: bool,
    // ---- Peak-relative ADSR. Defined against THE peak of the sound, so they
    // exist only when the file holds one event: on a multi-event file the
    // loudest peak is an artefact of the edit, and measuring 10 %→90 % against
    // it reports the distance to the loudest hit as an "attack" (a 3 s typing
    // recording read 2.5 s). null here means "not measurable at file level" —
    // the real per-event numbers are in `slices`, never zero, never a guess.
    #[serde(default)]
    pub envelope_attack_seconds: Option<f64>,
    #[serde(default)]
    pub envelope_decay_seconds: Option<f64>,
    #[serde(default)]
    pub envelope_sustain_level: Option<f64>,
    #[serde(default)]
    pub envelope_release_seconds: Option<f64>,
    #[serde(default)]
    pub envelope_temporal_centroid: Option<f64>,
    #[serde(default)]
    pub envelope_skewness: Option<f64>,
    #[serde(default)]
    pub envelope_kurtosis: Option<f64>,
    /// The categorical shape stays a plain field: "Multi" is the correct and
    /// useful answer for a multi-event file, where the numbers above are not.
    pub envelope_shape: String,
    #[serde(default)]
    pub decay_time_seconds_60db: Option<f64>,
    /// One entry per transient, in time order — the ADSR array. Always at least
    /// one element for audible audio; exactly one for a one-shot, in which case
    /// it carries the same numbers as the file-level fields above.
    #[serde(default)]
    pub slices: Vec<EnvelopeSlice>,
    #[serde(default)]
    pub onset_periodicity: Option<f64>,
    // Distinct onsets per second — transient_count over the clip length. None only
    // for a zero-length clip. Emitted so consumers read it off the record rather
    // than re-deriving it (the UCS matcher used to compute it on the fly).
    #[serde(default)]
    pub onset_rate_per_second: Option<f64>,
}

impl Envelope {
    /// The loudest slice — the best single-event ADSR the file contains.
    ///
    /// This is what to read when a consumer needs a number on a multi-event file
    /// and "no evidence" is not an option (the clustering feature vector, a UI
    /// bar). It is an honest measurement of one real event, which the file-level
    /// fields are not; it is simply not a measurement of the *file*, so it must
    /// never be written back into them.
    pub fn representative_slice(&self) -> Option<&EnvelopeSlice> {
        self.slices.iter().max_by(|a, b| {
            a.relative_level
                .partial_cmp(&b.relative_level)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct SpectralFeatures {
    pub root_mean_square_level: f64,
    pub crest_factor: f64,
    pub zero_crossings_per_second: f64,
    pub complexity: f64,
    pub spectral_centroid_hz: f64,
    pub spectral_rolloff_hz: f64,
    pub spectral_flatness: f64,
    pub low_band_energy: f64,
    pub mid_band_energy: f64,
    pub high_band_energy: f64,
    pub spectral_flux: f64,
    pub harmonicity: f64,
    pub inharmonicity: f64,
    pub partial_count: usize,
    pub mel_frequency_cepstral_coefficients: Vec<f64>,
    pub spectral_centroid_mean_hz: f64,
    pub spectral_centroid_deviation_hz: f64,
    pub total_harmonic_distortion: f64,
    pub clipping_density: f64,
    pub distortion: String,
    #[serde(default)]
    pub stationarity: Option<f64>,
    #[serde(default)]
    pub spectral_entropy: Option<f64>,
    #[serde(default)]
    pub spectral_slope_db_per_octave: Option<f64>,
    #[serde(default)]
    pub band_limit_high_hz: Option<f64>,
    #[serde(default)]
    pub spectral_centroid_slope_hz_per_second: Option<f64>,
    #[serde(default)]
    pub syllabic_modulation_energy: Option<f64>,
    #[serde(default)]
    pub voicing_ratio: Option<f64>,
    pub mid_rms: f64,
    pub side_rms: f64,
    #[serde(default = "lufs_unmeasurable", deserialize_with = "lufs_from_json")]
    pub lufs: f64,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct BeatMarker {
    pub index: usize,
    pub timestamp_seconds: f64,
    pub sample_index: u64,
    pub is_downbeat: bool,
    pub bar: usize,
    pub beat: usize,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct ChunkNoteMap {
    pub chunk_index: usize,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub duration_seconds: f64,
    pub root_note_name: String,
    pub root_midi_note: i32,
    pub root_frequency_hz: f64,
    pub pitch_hz: f64,
    pub cents_offset: f64,
    pub peak_amplitude: f64,
    pub nearest_beat_index: Option<usize>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct NoteRootKeyBeatMarkerMap {
    pub global_root_note: String,
    pub global_midi_note: i32,
    pub global_bpm: f64,
    pub total_beats: usize,
    pub beat_markers: Vec<BeatMarker>,
    pub chunk_maps: Vec<ChunkNoteMap>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Musicality {
    pub pitch_hz: f64,
    #[serde(default)]
    pub pitch_slope_semitones_per_second: Option<f64>,
    pub root_note_name: String,
    pub root_frequency_hz: f64,
    pub root_cents_offset: f64,
    pub beats_per_minute: f64,
    pub root_midi_note: i32,
    pub chromagram: [f64; 12],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note_root_key_beat_marker_map: Option<NoteRootKeyBeatMarkerMap>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Unsupervised {
    pub cluster: i32,
    pub principal_components: Vec<f64>,
}

/// One sounding stretch inside a file, bounded by silence — its in-point and
/// out-point. `name` is empty from the analyzer and filled in by the user in the
/// Extractor editor.
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Region {
    pub index: usize,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub duration_seconds: f64,
    /// Loudest RMS-envelope value inside the region (linear, 0..~1).
    pub peak_amplitude: f64,
    pub name: String,
    /// The full analysis of JUST this sub-clip's audio — every feature + classification a
    /// standalone file gets, computed by re-running the pipeline on the region's slice.
    /// `None` on records from before per-region analysis, and on single-region files (the
    /// one region is the whole file, so its analysis equals the parent's). Boxed because a
    /// `Peak` contains `Regions`, so a `Region` owning a `Peak` would be infinitely sized.
    #[serde(default)]
    pub analysis: Option<Box<Peak>>,
}

/// Every region found in a file, plus the silence-gate settings that found them.
/// `count > 1` marks a file as multi-region for library-wide discovery.
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Regions {
    pub count: usize,
    pub detection_threshold_decibels: f64,
    pub minimum_silence_seconds: f64,
    pub minimum_region_seconds: f64,
    pub regions: Vec<Region>,
}

/// The binary waveform preview: interleaved signed 8-bit min,max peak pairs per bin,
/// base64-encoded — computed for every file so the UI paints a waveform without
/// decoding any audio. Shape and encoding rationale:
/// Documentation/Audit/binary_peak_preview_audit.md.
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Preview {
    pub preview_version: u32,
    pub samples_per_bin: u32,
    pub bin_count: u32,
    pub bits_per_value: u32,
    pub channel_mode: String,
    pub peak_data_base64: String,
}

impl Preview {
    pub fn is_empty(&self) -> bool {
        self.peak_data_base64.is_empty()
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Peak {
    pub metadata: Metadata,
    pub classification: Classification,
    pub envelope: Envelope,
    pub spectral_features: SpectralFeatures,
    pub musicality: Musicality,
    pub unsupervised: Unsupervised,
    pub ucs: Ucs,
    // Added after the original schema — old sidecars have no regions, so default it
    // in rather than fail the whole record on a missing field.
    #[serde(default)]
    pub regions: Regions,
    // Added after regions; same lenient-read rule. Skipped on write while empty so a
    // re-emitted old record stays byte-comparable to its source.
    #[serde(default, skip_serializing_if = "Preview::is_empty")]
    pub preview: Preview,
    /// The sequence-level analysis: syllable-type vocabulary, slice-to-slice
    /// transition probabilities, and the measured junctions between the slices.
    /// `None` for anything that is not a sequence — a single-region file has no
    /// order to analyze — so absence is meaningful and is not written out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bioacoustic_syntax: Option<crate::syntax::BioacousticSyntax>,
}
