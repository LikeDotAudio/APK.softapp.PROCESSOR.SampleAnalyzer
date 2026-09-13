//! Contributing records to the shared cloud database over HTTP.
//!
//! Not a database client despite the name — `Pool` is a vestige of the direct
//! MySQL connection this replaced, kept so the Tauri command's call sites did
//! not have to change. Everything here POSTs JSON to `upload_peak.php`.
//!
//! Records are trimmed to the columns the endpoint stores before being sent.
//! They used to go up whole, which was two bugs at once: a 500-record chunk of
//! full records is ~5.7 MB and exceeds `post_max_size` on typical shared hosting
//! (PHP then discards the body and the script sees an empty payload), and the
//! `Vec<String>` fields arrived as JSON arrays that PDO stringified into the
//! literal text "Array" — 1,000 rows in the live database still read that way.
use serde::Serialize;

use crate::peak::Peak;

const UPLOAD_URL: &str = "https://scanalyzer.like.audio/api/upload_peak.php";

/// Records per POST. Small enough that a trimmed batch stays a few hundred KB.
const CHUNK: usize = 250;

pub struct Pool;

pub fn get_pool(_db_url: &str) -> Result<Pool, String> {
    Ok(Pool)
}

pub fn init_db(_pool: &Pool) -> Result<(), String> {
    Ok(())
}

/// A record cut down to the stored columns. Mirrors `slimForUpload` in
/// `Web_Front/src/peakUpload.ts`; keep both in step with `upload_peak.php`.
#[derive(Serialize)]
struct Upload<'a> {
    metadata: Metadata<'a>,
    classification: Classification<'a>,
    ucs: Ucs<'a>,
    spectral_features: Spectral,
    musicality: Musicality<'a>,
    envelope: Envelope<'a>,
    regions: RegionCount,
}

/// Just the count. The per-region detail is a whole nested analysis each and has
/// no columns; the count is what lets the cloud answer "which files have more
/// than one segment", which it previously could not answer at all.
#[derive(Serialize)]
struct RegionCount {
    count: usize,
}

#[derive(Serialize)]
struct Metadata<'a> {
    name: &'a str,
    /// The folder the endpoint files this record under. It reads THIS field —
    /// it used to derive the folder from `name`, which is a bare filename, so
    /// every record in the database landed under "." and same-named files
    /// overwrote one another.
    folder: &'a str,
    path: &'a str,
    analyzer_version: &'a str,
    length_seconds: f64,
    sample_rate: u32,
    bit_depth: u16,
    channels: u16,
    source_format: &'a str,
    lossy_source: bool,
    dc_offset: f64,
}

#[derive(Serialize)]
struct Classification<'a> {
    group: &'a str,
    subgroup: &'a str,
    timbre: &'a str,
    acoustic_types: &'a [String],
    instrument_family: &'a [String],
    reason: &'a [String],
}

#[derive(Serialize)]
struct Alternative<'a> {
    category: &'a str,
    subcategory: &'a str,
}

#[derive(Serialize)]
struct Ucs<'a> {
    category: &'a str,
    subcategory: &'a str,
    alternatives: Vec<Alternative<'a>>,
}

#[derive(Serialize)]
struct Spectral {
    root_mean_square_level: f64,
    crest_factor: f64,
    complexity: f64,
    spectral_centroid_hz: f64,
    spectral_rolloff_hz: f64,
    spectral_flatness: f64,
    harmonicity: f64,
    total_harmonic_distortion: f64,
    clipping_density: f64,
}

#[derive(Serialize)]
struct Musicality<'a> {
    pitch_hz: f64,
    root_note_name: &'a str,
    root_midi_note: i32,
    root_cents_offset: f64,
    beats_per_minute: f64,
}

#[derive(Serialize)]
struct Envelope<'a> {
    transient_count: usize,
    attack_seconds: f64,
    // Peak-relative, so null on a multi-event file — the column takes NULL and
    // that is the honest value, not a zero.
    envelope_decay_seconds: Option<f64>,
    envelope_sustain_level: Option<f64>,
    envelope_release_seconds: Option<f64>,
    envelope_temporal_centroid: Option<f64>,
    envelope_shape: &'a str,
}

fn slim(p: &Peak) -> Upload<'_> {
    Upload {
        metadata: Metadata {
            name: &p.metadata.name,
            folder: &p.metadata.folder,
            path: &p.metadata.path,
            analyzer_version: &p.metadata.analyzer_version,
            length_seconds: p.metadata.length_seconds,
            sample_rate: p.metadata.sample_rate,
            bit_depth: p.metadata.bit_depth,
            channels: p.metadata.channels,
            source_format: &p.metadata.source_format,
            lossy_source: p.metadata.lossy_source,
            dc_offset: p.metadata.dc_offset,
        },
        classification: Classification {
            group: &p.classification.group,
            subgroup: &p.classification.subgroup,
            timbre: &p.classification.timbre,
            acoustic_types: &p.classification.acoustic_types,
            instrument_family: &p.classification.instrument_family,
            reason: &p.classification.reason,
        },
        ucs: Ucs {
            category: &p.ucs.category,
            subcategory: &p.ucs.subcategory,
            // Only three alternatives have columns.
            alternatives: p
                .ucs
                .alternatives
                .iter()
                .take(3)
                .map(|a| Alternative { category: &a.category, subcategory: &a.subcategory })
                .collect(),
        },
        spectral_features: Spectral {
            root_mean_square_level: p.spectral_features.root_mean_square_level,
            crest_factor: p.spectral_features.crest_factor,
            complexity: p.spectral_features.complexity,
            spectral_centroid_hz: p.spectral_features.spectral_centroid_hz,
            spectral_rolloff_hz: p.spectral_features.spectral_rolloff_hz,
            spectral_flatness: p.spectral_features.spectral_flatness,
            harmonicity: p.spectral_features.harmonicity,
            total_harmonic_distortion: p.spectral_features.total_harmonic_distortion,
            clipping_density: p.spectral_features.clipping_density,
        },
        musicality: Musicality {
            pitch_hz: p.musicality.pitch_hz,
            root_note_name: &p.musicality.root_note_name,
            root_midi_note: p.musicality.root_midi_note,
            root_cents_offset: p.musicality.root_cents_offset,
            beats_per_minute: p.musicality.beats_per_minute,
        },
        envelope: Envelope {
            transient_count: p.envelope.transient_count,
            attack_seconds: p.envelope.attack_seconds,
            envelope_decay_seconds: p.envelope.envelope_decay_seconds,
            envelope_sustain_level: p.envelope.envelope_sustain_level,
            envelope_release_seconds: p.envelope.envelope_release_seconds,
            envelope_temporal_centroid: p.envelope.envelope_temporal_centroid,
            envelope_shape: &p.envelope.envelope_shape,
        },
        regions: RegionCount { count: p.regions.count },
    }
}

/// POST every record, in chunks. Returns how many rows the SERVER reports it
/// stored — which is not the same as how many were sent, and the difference is
/// the whole point: the old version returned `Ok(())` on any 200 and the caller
/// reported the full input length as a success.
pub fn write_peaks(_pool: &Pool, peaks: &[Peak]) -> Result<usize, String> {
    if peaks.is_empty() {
        return Ok(0);
    }

    let mut stored = 0usize;
    for chunk in peaks.chunks(CHUNK) {
        let body: Vec<Upload> = chunk.iter().map(slim).collect();
        let resp = ureq::post(UPLOAD_URL)
            .set("Content-Type", "application/json")
            .send_json(&body)
            .map_err(|e| match e {
                // A 4xx/5xx carries the endpoint's own explanation — surface it
                // instead of the generic "http status 400", which told nobody
                // that the payload had been too large.
                ureq::Error::Status(code, resp) => format!(
                    "HTTP {code}: {}",
                    resp.into_string().unwrap_or_default().chars().take(300).collect::<String>()
                ),
                other => other.to_string(),
            })?;

        let text = resp.into_string().map_err(|e| e.to_string())?;
        let parsed: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("bad response: {e} — {text}"))?;
        stored += parsed
            .get("stored")
            .or_else(|| parsed.get("inserted"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
    }

    Ok(stored)
}
