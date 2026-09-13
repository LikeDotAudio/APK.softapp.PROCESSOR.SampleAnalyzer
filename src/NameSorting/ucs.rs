//! UCS classification — the implementation of Documentation/ucs_signature_spec.md.
//!
//! The previous version of this module hardcoded `let cat = "MUSICAL"` and could
//! therefore only ever emit one of the 82 categories. It also never looked at the
//! file name, and never read the 753-entry synonym table sitting next to it.
//!
//! What it does now, per the spec:
//!
//!   1. GATES     hard constraints; a violation zeroes the candidate outright.
//!   2. PRIORS    weighted Gaussian log-likelihood -> S(c|x), the signal term.
//!   3. TEXT      synonym hits in the file and folder name -> T(c|name).
//!   4. POSTERIOR P ∝ S^α · T^β, with α/β set by the subcategory's honesty tier,
//!                so a `semantic_only` subcategory is decided by words and a
//!                `signal_separable` one by physics.
//!   5. ABSTAIN   a weak or a too-close win falls back to the category's MISC.
//!
//! Two deliberate departures from the spec, both documented at their constant:
//! the text term is IDF-weighted (`idf`), and lossy sources get leniency
//! (`LOSSY_UNRELIABLE`).
use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Deserializer, Serialize};

use crate::normalize::normalize_name_words;
use crate::peak::Peak;

// ---------------------------------------------------------------- data model

#[derive(Debug, Clone, Deserialize)]
pub struct Gate {
    pub feature: String,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Prior {
    pub feature: String,
    pub mean: f64,
    pub deviation: f64,
    pub transform: String, // "log" | "linear"
    pub weight: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Signature {
    pub morphology: String,
    pub separability: String,
    #[serde(default)]
    pub gates: Vec<Gate>,
    #[serde(default)]
    pub priors: Vec<Prior>,
    #[serde(default)]
    pub discriminators: Vec<String>,
    #[serde(default)]
    pub confusable_with: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Subcategory {
    pub subcategory: String,
    pub category_id: String,
    pub category_short: String,
    pub explanation: Vec<String>,
    pub synonyms: Vec<String>,
    pub acoustic_signature: Signature,
}

#[derive(Debug, Clone, Deserialize)]
struct CategoryFile {
    category: String,
    subcategories: Vec<Subcategory>,
}

/// One flattened, searchable UCS subcategory.
pub struct Entry {
    pub category: String,
    pub sub: Subcategory,
    /// Distinct match tokens: synonyms + the subcategory's own words.
    pub tokens: Vec<String>,
}

pub struct Index {
    pub entries: Vec<Entry>,
    /// token -> inverse document frequency over the 756 subcategories
    idf: HashMap<String, f64>,
}

/// The signed category data, spliced from UCS/categories/*.json by build.rs.
const UCS_BUNDLE: &str = include_str!(concat!(env!("OUT_DIR"), "/ucs_signed.json"));

fn build_index() -> Index {
    let files: Vec<CategoryFile> = serde_json::from_str(UCS_BUNDLE)
        .expect("embedded UCS bundle is not valid JSON — check build.rs");

    let mut entries: Vec<Entry> = Vec::new();
    for f in files {
        for sub in f.subcategories {
            let mut tokens: Vec<String> = Vec::new();
            // The subcategory's OWN name words ("CAT WILD" -> cat, wild). A generic
            // modifier adjective (see MODIFIER_STOPWORDS) is real evidence only when it
            // is one of these; borrowed into a synonym it is dropped.
            let own: Vec<String> = normalize_name_words(&sub.subcategory)
                .split_whitespace()
                .filter(|t| is_useful_token(t))
                .map(|t| t.to_string())
                .collect();
            // Synonyms, plus the subcategory name itself ("SLIDING", "HORSE").
            let sources = sub
                .synonyms
                .iter()
                .cloned()
                .chain(std::iter::once(sub.subcategory.clone()));
            for s in sources {
                for t in normalize_name_words(&s).split_whitespace() {
                    if !is_useful_token(t) {
                        continue;
                    }
                    // A borrowed generic modifier ("big" in a CAT WILD synonym) carries no
                    // category evidence — keep it only as the subcategory's own word.
                    if MODIFIER_STOPWORDS.contains(&t) && !own.iter().any(|x| x == t) {
                        continue;
                    }
                    if !tokens.iter().any(|x| x == t) {
                        tokens.push(t.to_string());
                    }
                }
            }
            entries.push(Entry {
                category: f.category.clone(),
                sub,
                tokens,
            });
        }
    }

    // Document frequency across subcategories, then IDF.
    let n = entries.len() as f64;
    let mut df: HashMap<String, usize> = HashMap::new();
    for e in &entries {
        for t in &e.tokens {
            *df.entry(t.clone()).or_insert(0) += 1;
        }
    }
    // ln(N/df): a token in one subcategory ("tambourine") scores ~6.6; one in
    // half of them ("large", "misc") scores ~0.7; one in all of them scores 0.
    // This is the fix for the naive hit-count matcher, which fired on generic
    // synonyms and mapped ORGAN to PERCUSSION TUNED.
    let idf = df
        .into_iter()
        .map(|(t, d)| (t, (n / d as f64).ln().max(0.0)))
        .collect();

    Index { entries, idf }
}

pub fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(build_index)
}

/// Short and all-digit tokens that DO carry evidence, despite the length rule below.
///
/// A producer names a kick `BD`, a hi-hat `HH` or `CH`/`OH`, and a drum machine by its
/// model number — `808`, `909`. The generic filter threw every one of them away as
/// noise, so the most reliable signal in a drum library ("BD_01.wav" is a bass drum,
/// with certainty) was the one signal the classifier could not see. These are only
/// evidence because the producer overlay maps them to a subcategory; a short token that
/// no synonym table claims still scores nothing, because its IDF is zero.
const SHORT_EVIDENCE: &[&str] = &[
    "bd", "sd", "hh", "ch", "oh", "kk", "tom", "dj", "ir", "fx", "808", "909", "707", "606", "303",
];

/// Generic size/quality modifier adjectives. Flattened out of a descriptive phrase
/// into a standalone synonym — "Big" from "big cat", "Large" from "large drum" — these
/// become rare, high-IDF tokens that hijack any file name containing the adjective. A
/// `BigDrum_...wav` scored as ANIMALS/CAT WILD because "big" (df 3, so high IDF) beat
/// the far more on-topic but common "drum" (df 8, so low IDF). We keep such a word ONLY
/// when it is the subcategory's own name word; borrowed into a synonym it carries no
/// category evidence and is dropped (see build_index). Deliberately excludes nouns like
/// "snow"/"ice" whose category may name itself only via synonyms.
const MODIFIER_STOPWORDS: &[&str] = &[
    "big", "small", "large", "little", "tiny", "huge", "giant", "massive", "long",
    "short", "deep", "low", "high", "fat", "thin", "heavy", "light", "soft", "hard",
    "wet", "dry", "hot", "cold", "old", "new", "fast", "slow", "loud", "quiet",
    "dark", "loose", "tight",
];

/// Descriptor idioms a whole-token matcher otherwise shatters into a misleading
/// category noun. "old school" is a vintage descriptor — an old-school hip-hop
/// beat, a synth patch that sounds old school — not a school; but tokenized to
/// `["old", "school"]` the common noun "school" fires SCHOOL BELL, SCHOOL BUS and
/// CROWDS/CHILDREN, none of which the file is about. We drop the idiom's trailing
/// noun so it stops injecting a category it never meant. "old" itself is already a
/// MODIFIER_STOPWORD and carries no evidence, so the pair goes silent. Rare literal
/// uses ("old school bell.wav") lose the school-noun hit — an acceptable trade for
/// killing the far more common vintage-descriptor false positive.
fn strip_descriptor_idioms(tokens: &mut Vec<String>) {
    let mut i = 0;
    while i + 1 < tokens.len() {
        if tokens[i] == "old" && tokens[i + 1] == "school" {
            tokens.remove(i + 1);
        } else {
            i += 1;
        }
    }
}

/// Tokens too short or too generic to carry category evidence.
fn is_useful_token(t: &str) -> bool {
    if SHORT_EVIDENCE.contains(&t) {
        return true;
    }
    if t.len() < 3 || t.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    !matches!(
        t,
        "the" | "and" | "for" | "with" | "from" | "into" | "onto" | "out" | "off"
            | "track" | "sound" | "sounds" | "audio" | "misc" | "wav" | "mp3"
    )
}

// ------------------------------------------------------------------ features

/// Look a spec feature name up in a finished record.
///
/// Returns None when the feature is one of the spec's "proposed" set that we do
/// not compute yet, or when the stored value is a sentinel meaning "unmeasurable".
/// Per spec §6, a missing feature is skipped and its weight dropped from the
/// normalizer — never treated as zero.
fn feature(p: &Peak, name: &str) -> Option<f64> {
    let v = match name {
        "length_seconds" => p.metadata.length_seconds,
        "transient_count" => p.envelope.transient_count as f64,
        "root_mean_square_level" => p.spectral_features.root_mean_square_level,
        "lufs" => {
            // −70 is the "unmeasurable" sentinel, not a loudness.
            if p.spectral_features.lufs <= -69.0 {
                return None;
            }
            p.spectral_features.lufs
        }
        "crest_factor" => p.spectral_features.crest_factor,
        "zero_crossings_per_second" => p.spectral_features.zero_crossings_per_second,
        "pitch_hz" => {
            // The spec declares this valid only over 50–2000 Hz.
            if !(50.0..=2000.0).contains(&p.musicality.pitch_hz) {
                return None;
            }
            p.musicality.pitch_hz
        }
        "harmonicity" => p.spectral_features.harmonicity,
        "inharmonicity" => p.spectral_features.inharmonicity,
        "partial_count" => p.spectral_features.partial_count as f64,
        "sustain_ratio" => p.envelope.sustain_ratio,
        "spectral_centroid_hz" => p.spectral_features.spectral_centroid_hz,
        "spectral_centroid_mean_hz" => p.spectral_features.spectral_centroid_mean_hz,
        "spectral_centroid_deviation_hz" => p.spectral_features.spectral_centroid_deviation_hz,
        "spectral_rolloff_hz" => p.spectral_features.spectral_rolloff_hz,
        "spectral_flatness" => p.spectral_features.spectral_flatness,
        "spectral_flux" => p.spectral_features.spectral_flux,
        "complexity" => p.spectral_features.complexity,
        "low_band_energy" => p.spectral_features.low_band_energy,
        "mid_band_energy" => p.spectral_features.mid_band_energy,
        "high_band_energy" => p.spectral_features.high_band_energy,
        "total_harmonic_distortion" => p.spectral_features.total_harmonic_distortion,
        "clipping_density" => p.spectral_features.clipping_density,
        // --- Peak-relative ADSR. None on a multi-event file, where there is no
        // --- single peak to be relative to, so the scorer drops the term rather
        // --- than matching a signature against the shape of somebody's edit.
        // --- (These signatures were only ever calibrated on one-shot clips —
        // --- see SINGLE_EVENT_ONLY in UCS/fsd50k_calibrate.py — so scoring them
        // --- on a multi-event record was always outside their evidence.)
        "envelope_attack_seconds" => p.envelope.envelope_attack_seconds?,
        "envelope_decay_seconds" => p.envelope.envelope_decay_seconds?,
        "envelope_sustain_level" => p.envelope.envelope_sustain_level?,
        "envelope_release_seconds" => p.envelope.envelope_release_seconds?,
        "envelope_temporal_centroid" => p.envelope.envelope_temporal_centroid?,
        "envelope_skewness" => p.envelope.envelope_skewness?,
        "envelope_kurtosis" => p.envelope.envelope_kurtosis?,
        "dc_offset" => p.metadata.dc_offset,
        "beats_per_minute" => {
            if p.musicality.beats_per_minute <= 0.0 {
                return None;
            }
            p.musicality.beats_per_minute
        }
        "sample_rate" => p.metadata.sample_rate as f64,

        // --- spec §4b, the morphology axis. Measured by `morphology.rs` and
        // --- `envelope.rs`; each is an Option on the record because the sound
        // --- may simply not have the property (a drone has no ring time), and
        // --- None must mean "skip this term", never "the value is 0".
        "stationarity" => p.spectral_features.stationarity?,
        "spectral_entropy" => p.spectral_features.spectral_entropy?,
        "spectral_slope_db_per_octave" => p.spectral_features.spectral_slope_db_per_octave?,
        "band_limit_high_hz" => p.spectral_features.band_limit_high_hz?,
        "spectral_centroid_slope_hz_per_second" => p.spectral_features.spectral_centroid_slope_hz_per_second?,
        "pitch_slope_semitones_per_second" => p.musicality.pitch_slope_semitones_per_second?,
        "syllabic_modulation_energy" => p.spectral_features.syllabic_modulation_energy?,
        "decay_time_seconds_60db" => p.envelope.decay_time_seconds_60db?,
        "voicing_ratio" => p.spectral_features.voicing_ratio?,
        "onset_periodicity" => p.envelope.onset_periodicity?,

        // Prefer the stored field; fall back to deriving it for older records that
        // predate the field (transient_count / length, guarded against zero length).
        "onset_rate_per_second" => p.envelope.onset_rate_per_second.or_else(|| {
            if p.metadata.length_seconds <= 0.0 {
                return None;
            }
            Some(p.envelope.transient_count as f64 / p.metadata.length_seconds)
        })?,
        "stereo_width" => p.spectral_features.side_rms / (p.spectral_features.mid_rms + 1e-9),

        _ => return None,
    };
    if v.is_finite() {
        Some(v)
    } else {
        None
    }
}

// -------------------------------------------------------------- lossy sources

/// Features a lossy encoder demonstrably corrupts.
///
/// MP3/AAC/Vorbis lowpass the signal (typically ~16 kHz), which drags every
/// brightness measure down; and their decoded output routinely overshoots
/// 0 dBFS, which manufactures clipping that was never in the source.
///
/// For a lossy file we therefore (a) do not let a gate on these features KILL a
/// candidate — a file must never be disqualified for an artifact the encoder
/// introduced — and (b) damp their prior weights, since they still carry some
/// signal, just less than they claim.
///
/// Note this changes only the DECISION. The measured numbers on the record stay
/// exactly as measured; we do not launder the data.
const LOSSY_UNRELIABLE: &[&str] = &[
    // encoder lowpass -> brightness is understated
    "high_band_energy",
    "spectral_rolloff_hz",
    "spectral_centroid_hz",
    "spectral_centroid_mean_hz",
    "spectral_centroid_deviation_hz",
    "spectral_flatness",
    "zero_crossings_per_second",
    "band_limit_high_hz",
    "spectral_slope_db_per_octave",
    "spectral_entropy",
    // decode overshoot / codec noise -> clipping and peak stats are overstated
    "clipping_density",
    "crest_factor",
    "total_harmonic_distortion",
];

const LOSSY_WEIGHT: f64 = 0.35;

fn unreliable(feature_name: &str, lossy: bool) -> bool {
    lossy && LOSSY_UNRELIABLE.contains(&feature_name)
}

// ------------------------------------------------------------------- scoring

/// α (signal) and β (text) exponents per honesty tier. Rebalanced so the ACOUSTIC
/// ANALYSIS leads and the name is secondary — the extracted features are strong, so a
/// bare word ("bass", "kick") should no longer out-vote the physics. Each pair sums to
/// 1.0, making the posterior a clean weighted geometric mean S^α·T^β (the (α+β)-th root
/// downstream becomes a no-op) and directly readable as "α% analysis, β% name":
///   signal_separable / signal_narrowable → 60% analysis, 40% name
///   semantic_only (weak/no signature)    → 50/50 (name still gets a fair half)
///   provenance_only                      → name/provenance only, by definition
fn tier_exponents(tier: &str) -> (f64, f64) {
    match tier {
        "signal_separable" => (0.6, 0.4),
        "signal_narrowable" => (0.6, 0.4),
        "semantic_only" => (0.5, 0.5),
        "provenance_only" => (0.0, 1.0),
        _ => (0.55, 0.45),
    }
}

/// Hard constraints. Returns the gate that killed it, if any.
fn gate_violation(sig: &Signature, p: &Peak, lossy: bool) -> Option<String> {
    for g in &sig.gates {
        if unreliable(&g.feature, lossy) {
            continue; // never disqualify a lossy file on an encoder artifact
        }
        let Some(v) = feature(p, &g.feature) else {
            continue; // feature not computed -> gate cannot be evaluated
        };
        if let Some(min) = g.min {
            if v < min {
                return Some(format!("{} {:.4} < min {:.4}", g.feature, v, min));
            }
        }
        if let Some(max) = g.max {
            if v > max {
                return Some(format!("{} {:.4} > max {:.4}", g.feature, v, max));
            }
        }
    }
    None
}

/// Weighted Gaussian log-likelihood over the priors -> S(c|x) ∈ (0, 1].
fn signal_likelihood(sig: &Signature, p: &Peak, lossy: bool) -> f64 {
    let mut acc = 0.0;
    let mut wsum = 0.0;
    for pr in &sig.priors {
        let Some(x) = feature(p, &pr.feature) else {
            continue; // spec §6: skip, and drop from the normalizer
        };
        let mut w = pr.weight;
        if unreliable(&pr.feature, lossy) {
            w *= LOSSY_WEIGHT;
        }
        if w <= 0.0 {
            continue;
        }

        let z = if pr.transform == "log" {
            // deviation is a MULTIPLICATIVE factor here; ln() it to get a sigma.
            if x <= 0.0 || pr.mean <= 0.0 || pr.deviation <= 1.0 {
                continue;
            }
            (x.ln() - pr.mean.ln()) / pr.deviation.ln()
        } else {
            if pr.deviation <= 0.0 {
                continue;
            }
            (x - pr.mean) / pr.deviation
        };
        // One wild feature must not veto everything.
        let z = z.clamp(-4.0, 4.0);
        acc += w * (-0.5 * z * z);
        wsum += w;
    }
    if wsum <= 0.0 {
        // No usable priors -> NEUTRAL (0.5), not a free perfect 1.0. A category that offers
        // no acoustic evidence for itself must not out-score one whose physics actually fit
        // the sound: the old 1.0 is exactly why a signature-less "bass boat" / "impact" could
        // beat a real bass/kick on a bare word. It is still not 0.0 (that would be "impossible").
        return 0.5;
    }
    (acc / wsum).exp()
}

const KAPPA: f64 = 2.0;
/// A single maximally-specific synonym hit has idf ≈ ln(756) ≈ 6.6; dividing by
/// this makes one such hit worth roughly one unit of evidence.
const IDF_UNIT: f64 = 6.0;
/// The folder is weaker evidence than the file's own name, but it is real: a
/// file in "1012 Explosions, Fairs, Farms, Football, Fire, Guns" is telling us
/// something.
const FOLDER_WEIGHT: f64 = 0.5;

/// T(c|name) = 1 + κ · h, with h an IDF-weighted hit mass rather than a raw count.
fn text_evidence(e: &Entry, idx: &Index, name_tokens: &[String], folder_tokens: &[String]) -> (f64, Vec<String>) {
    let mut mass = 0.0;
    let mut hits: Vec<String> = Vec::new();
    for t in &e.tokens {
        let in_name = name_tokens.iter().any(|x| x == t);
        let in_folder = folder_tokens.iter().any(|x| x == t);
        if !in_name && !in_folder {
            continue;
        }
        let idf = *idx.idf.get(t).unwrap_or(&0.0);
        if idf <= 0.0 {
            continue;
        }
        let w = if in_name { 1.0 } else { FOLDER_WEIGHT };
        mass += idf * w;
        hits.push(t.clone());
    }
    (1.0 + KAPPA * mass / IDF_UNIT, hits)
}

// -------------------------------------------------------------------- verdict

/// A runner-up the matcher scored. Spelled out in full English like every other
/// field in the .PEAK — the old form was a single packed string of the ABBREVIATED
/// id plus a number ("DSGNMisc 0.003"), which no consumer could read or filter on
/// without re-parsing it.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Alternative {
    pub category: String,
    pub subcategory: String,
    pub id: String,
    pub probability: f64,
    /// The synonyms this runner-up matched in the name or folder. Empty means it
    /// placed on acoustic signal alone.
    #[serde(default)]
    pub synonyms: Vec<String>,
}

/// Read either form. A .PEAK written before the struct existed holds the packed
/// string described above ("DSGNMisc 0.003"); refusing it made whole libraries
/// unreadable to their own analyzer over a runner-up nobody reads. We recover the id
/// and the probability, and leave the names blank rather than inventing them — the
/// abbreviated id is all the old form ever recorded.
impl<'de> Deserialize<'de> for Alternative {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Either {
            Packed(String),
            Full {
                #[serde(default)]
                category: String,
                #[serde(default)]
                subcategory: String,
                #[serde(default)]
                id: String,
                #[serde(default)]
                probability: f64,
                #[serde(default)]
                synonyms: Vec<String>,
            },
        }
        Ok(match Either::deserialize(d)? {
            Either::Full { category, subcategory, id, probability, synonyms } => Alternative {
                category,
                subcategory,
                id,
                probability,
                synonyms,
            },
            Either::Packed(s) => {
                let (id, prob) = s.rsplit_once(' ').unwrap_or((s.as_str(), "0"));
                Alternative {
                    id: id.to_string(),
                    probability: prob.parse().unwrap_or(0.0),
                    ..Default::default()
                }
            }
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Verdict {
    pub category: String,
    pub subcategory: String,
    pub id: String,
    pub confidence: f64,
    pub alternatives: Vec<Alternative>,
    /// The words that actually won this match — the synonyms of the chosen
    /// subcategory found in the file or folder name. Empty means the verdict rests
    /// on acoustic signal alone, with no name evidence at all.
    ///
    /// A list, not a joined string: a match can hit several words ("kick", "drum"),
    /// and a consumer must be able to filter on them without re-parsing prose. It
    /// was previously recoverable only by reading `reason`.
    #[serde(default)]
    pub synonyms: Vec<String>,
    pub reason: String,
}

/// Below this normalized posterior, or this close to the runner-up, we do not
/// commit — we fall back to the winning category's MISC. Spec §6: "Abstention
/// is a correct answer, not a failure."
const MIN_CONFIDENCE: f64 = 0.12;

/// See the call site: restores the score spread the abstain thresholds expect, without
/// touching the ranking. Swept over both labelled corpora; 4.0 is the knee.
const POSTERIOR_SHARPNESS: f64 = 4.0;
const MIN_MARGIN: f64 = 0.04;

pub fn classify(p: &Peak) -> Verdict {
    let idx = index();
    let lossy = p.metadata.lossy_source;

    let stem = p
        .metadata.name
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(&p.metadata.name)
        .to_string();
    let mut name_tokens: Vec<String> = normalize_name_words(&stem)
        .split_whitespace()
        .filter(|t| is_useful_token(t))
        .map(str::to_string)
        .collect();
    strip_descriptor_idioms(&mut name_tokens);
    let mut folder_tokens: Vec<String> = normalize_name_words(&p.metadata.folder)
        .split_whitespace()
        .filter(|t| is_useful_token(t))
        .map(str::to_string)
        .collect();
    strip_descriptor_idioms(&mut folder_tokens);

    struct Cand {
        i: usize,
        post: f64,
        hits: Vec<String>,
        signal: f64,
    }

    let mut cands: Vec<Cand> = Vec::new();
    let mut killed: Option<(String, String)> = None; // (category_id, gate)

    for (i, e) in idx.entries.iter().enumerate() {
        let sig = &e.sub.acoustic_signature;

        // A `provenance_only` subcategory is defined by HOW a file came to exist
        // — an ADR take, a bounce, a temp mix, a trademarked asset — not by what
        // it sounds like. The audio cannot know that, and neither can the name:
        // scoring them produced exactly the nonsense you would predict, tagging
        // a dog's collar TAG as ARCHIVED-ASSET, "falling DOWN stairs" as
        // ARCHIVED-BOUNCE, and an electric MIXER as ARCHIVED-MIX.
        //
        // Provenance is a librarian's assertion. We never auto-assert it.
        if sig.separability == "provenance_only" {
            continue;
        }

        // A MISC bucket is the ABSTENTION TARGET, not a candidate. It is where a
        // verdict goes when the evidence will not support a real subcategory — so it
        // must not be able to WIN one. It carries no acoustic signature, which means
        // `signal_likelihood` hands it a free 1.0 ("no usable priors -> uninformative"),
        // and it inherits its category's vaguest synonyms. Left in the pool it beats
        // subcategories that actually describe the sound: a drum machine sample in a
        // folder called "Tape Drum Machines" was won by ROBOTS/MISC on the word
        // "machines", ahead of MUSICAL/PERCUSSION on "bd".
        //
        // Abstention still reaches these — the fallback below looks MISC up in the
        // index by category, not in this candidate pool.
        if e.sub.subcategory == "MISC" {
            continue;
        }

        if let Some(g) = gate_violation(sig, p, lossy) {
            if killed.is_none() {
                killed = Some((e.sub.category_id.clone(), g));
            }
            continue;
        }
        let (t, hits) = text_evidence(e, idx, &name_tokens, &folder_tokens);
        let s = signal_likelihood(sig, p, lossy);
        let (alpha, beta) = tier_exponents(&sig.separability);

        // A WEIGHTED GEOMETRIC MEAN, not the raw product of spec §5.
        //
        // The spec's S^α·T^β is not comparable across candidates, because α and β
        // differ per tier: a `semantic_only` subcategory cubes its text term while a
        // `signal_separable` one leaves its own linear. The tier then decides the
        // MAGNITUDE of the score rather than the balance of evidence within it, and a
        // vague category with weak evidence outranks a precise one with strong
        // evidence. Measured on a 36k drum library: "DMX_AnalogBD1_A.wav" in a folder
        // called "Tape Drum Machines" was filed under ROBOTS, because ROBOTS/MISC
        // (semantic_only) cubed a half-weight FOLDER hit on "machines" while
        // MUSICAL/PERCUSSION (signal_separable) could only multiply a full-weight
        // FILENAME hit on "bd" — the rarer, stronger, more specific evidence lost.
        //
        // Taking the (α+β)-th root puts every candidate back on one scale. The tier
        // still sets how much each source of evidence is TRUSTED — that is what it is
        // for — but it no longer inflates the score it produces.
        let post = (s.powf(alpha) * t.powf(beta)).powf(1.0 / (alpha + beta).max(1e-9));
        cands.push(Cand { i, post, hits, signal: s });
    }

    if cands.is_empty() {
        return Verdict {
            category: "ARCHIVED".into(),
            subcategory: "WTF".into(),
            id: "ARCHWtf".into(),
            confidence: 0.0,
            alternatives: vec![],
            synonyms: vec![],
            reason: "every subcategory was gated out — no candidate survived".into(),
        };
    }

    // Keep only the subcategories the NAME actually points at.
    //
    // Without this the posterior is meaningless: ~700 subcategories with zero
    // name evidence still score T = 1 each, so they collectively soak up the
    // probability mass and even a strong winner lands at ~0.04 — below any
    // sane threshold, so everything abstained to MISC.
    //
    // UCS is a predominantly semantic taxonomy (the spec puts roughly half the
    // 753 at `semantic_only`), so a subcategory that the file name does not
    // mention is not a serious candidate — UNLESS nothing is named at all, in
    // which case we keep the field open and let the signal decide alone.
    let named = cands.iter().any(|c| !c.hits.is_empty());
    if named {
        cands.retain(|c| !c.hits.is_empty());
    }

    let n_cands = cands.len();

    // Sharpen, then normalize.
    //
    // Taking the (α+β)-th root above made candidates COMPARABLE, but it also made them
    // CLOSE: divided by their sum, the winner's share often falls under MIN_CONFIDENCE
    // and we abstain on a call we got right. The abstain thresholds were calibrated
    // against the old, inflated scale. This restores the spread they expect.
    //
    // Monotonic, so it cannot change WHICH candidate wins — only how confidently the win
    // is stated. Ranking is settled above; this decides whether we commit to it.
    //
    // 4.0 is the knee, swept over both labelled corpora (examples/ucs_scorecard):
    //
    //     sharpness    1      2      3      4      6      8
    //     recall     27.7   30.9   32.5   33.8   34.8   35.4    (music -> MUSICAL)
    //     false pos   3.0    4.2    4.5    4.7    4.9    4.9    (sfx -> MUSICAL)
    //
    // Past 4 the recall flattens while the false positives do not, so it buys confidence
    // in wrong answers and little else.
    //
    // Measure this on the SFX collection, never on FSD50K. FSD50K is not a sound-effects
    // corpus — a large slice of it is music ("Accordion-...-Musical_instrument-Music"), so
    // MUSICAL verdicts there are CORRECT. Scoring them as false positives inverts the
    // result and condemns this transform; it is how an earlier pass talked itself out of it.
    for c in cands.iter_mut() {
        c.post = c.post.powf(POSTERIOR_SHARPNESS);
    }

    let total: f64 = cands.iter().map(|c| c.post).sum();
    for c in cands.iter_mut() {
        c.post /= total.max(1e-12);
    }
    cands.sort_by(|a, b| b.post.partial_cmp(&a.post).unwrap_or(std::cmp::Ordering::Equal));

    let top = &cands[0];
    let runner = cands.get(1);
    let e = &idx.entries[top.i];

    let alternatives: Vec<Alternative> = cands
        .iter()
        .skip(1)
        .take(3)
        .map(|c| {
            let alt = &idx.entries[c.i];
            Alternative {
                category: alt.category.clone(),
                subcategory: alt.sub.subcategory.clone(),
                id: alt.sub.category_id.clone(),
                probability: (c.post * 1000.0).round() / 1000.0,
                synonyms: c.hits.clone(),
            }
        })
        .collect();

    // Abstain on a weak or a photo-finish win.
    let margin = top.post - runner.map(|r| r.post).unwrap_or(0.0);
    let weak = top.post < MIN_CONFIDENCE;
    let close = margin < MIN_MARGIN;

    let mut reason = String::new();
    if top.hits.is_empty() {
        reason.push_str("no name evidence; ");
    } else {
        reason.push_str(&format!("name matched [{}]; ", top.hits.join(", ")));
    }
    reason.push_str(&format!(
        "signal {:.2}, tier {}, {} candidate{} in contention",
        top.signal,
        e.sub.acoustic_signature.separability,
        n_cands,
        if n_cands == 1 { "" } else { "s" }
    ));
    if lossy {
        reason.push_str("; lossy source — brightness and clipping gates relaxed");
    }
    if let Some((cid, g)) = &killed {
        reason.push_str(&format!("; gate ruled out {} ({})", cid, g));
    }

    if weak || close {
        // Fall back to the winning category's MISC, when it has one.
        if let Some(m) = idx
            .entries
            .iter()
            .find(|x| x.category == e.category && x.sub.subcategory == "MISC")
        {
            reason.push_str(if weak {
                "; ABSTAINED to MISC (confidence below threshold)"
            } else {
                "; ABSTAINED to MISC (top two too close to call)"
            });
            return Verdict {
                category: m.category.clone(),
                subcategory: m.sub.subcategory.clone(),
                id: m.sub.category_id.clone(),
                confidence: top.post,
                alternatives,
                // The synonyms that got us to this *category* before we abstained on
                // the subcategory — the evidence is still what it was.
                synonyms: top.hits.clone(),
                reason,
            };
        }
    }

    Verdict {
        category: e.category.clone(),
        subcategory: e.sub.subcategory.clone(),
        id: e.sub.category_id.clone(),
        confidence: top.post,
        alternatives,
        synonyms: top.hits.clone(),
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_loads_every_signed_subcategory() {
        let idx = index();
        assert!(
            idx.entries.len() >= 753,
            "expected the full UCS table, got {}",
            idx.entries.len()
        );
        // The categories the old hardcoded mapper could never reach.
        for want in ["DOORS", "EXPLOSIONS", "BIRDS", "FOOTSTEPS", "AIRCRAFT"] {
            assert!(
                idx.entries.iter().any(|e| e.category == want),
                "missing category {want}"
            );
        }
    }

    #[test]
    fn specific_synonyms_outrank_generic_ones() {
        let idx = index();
        // "tambourine" lives in very few subcategories; "large" is everywhere.
        let tamb = idx.idf.get("tambourine").copied().unwrap_or(0.0);
        let generic = idx.idf.get("large").copied().unwrap_or(0.0);
        assert!(
            tamb > generic,
            "IDF should favour the specific token: tambourine {tamb} vs large {generic}"
        );
    }

    #[test]
    fn provenance_is_never_auto_asserted() {
        // ARCHIVED-ADR, FOLEY-FEET and friends describe how a file was MADE.
        // No amount of audio or filename evidence may assert them.
        let idx = index();
        for id in ["ARCHAdr", "ADR", "FOLYFeet", "FOLYProp", "DSGNSrce"] {
            if let Some(e) = idx.entries.iter().find(|e| e.sub.category_id == id) {
                assert_eq!(
                    e.sub.acoustic_signature.separability, "provenance_only",
                    "{id} should be provenance_only"
                );
            }
        }
    }

    #[test]
    fn the_new_subcategories_are_reachable() {
        let idx = index();
        for id in ["SPRTEqus", "SPRTClimb", "AEROBaln"] {
            assert!(
                idx.entries.iter().any(|e| e.sub.category_id == id),
                "missing {id}"
            );
        }
    }

    // A hip-hop beat used to land in BELLS/MISC: the only name token that matched
    // anything was "school" (from ".../Old school"), while "hip", "hop" and the
    // plural "beats" matched no synonym at all. LOOPS now carries the hip-hop
    // vocabulary, so the folder points the verdict at the LOOP category it belongs to.
    #[test]
    fn a_hip_hop_beat_lands_in_loops_not_bells() {
        let p: Peak = serde_json::from_str(include_str!("testdata/hiphop_beat.peak.json"))
            .expect("fixture must match the current Peak schema");
        let v = classify(&p);
        assert_eq!(
            v.category, "LOOPS",
            "hip-hop beat should be a LOOP, got {}/{} — {}",
            v.category, v.subcategory, v.reason
        );
    }

    // "old school" is a vintage descriptor, not a school — it must not fire the
    // school-noun categories (SCHOOL BELL / SCHOOL BUS / CROWDS CHILDREN).
    #[test]
    fn old_school_idiom_does_not_fire_school_categories() {
        let mut toks = vec!["old".to_string(), "school".to_string(), "beat".to_string()];
        strip_descriptor_idioms(&mut toks);
        assert_eq!(toks, vec!["old".to_string(), "beat".to_string()]);
        // A real school (not preceded by "old") still carries its evidence.
        let mut real = vec!["school".to_string(), "bell".to_string()];
        strip_descriptor_idioms(&mut real);
        assert_eq!(real, vec!["school".to_string(), "bell".to_string()]);
    }
}
