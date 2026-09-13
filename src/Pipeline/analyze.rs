//! Analyze one audio file into a `Peak` record. This orchestrates the individual
//! single-purpose feature extractors; each `let` is one well-scoped step.
//!
//! `analyze` (from a file path) and `analyze_buffer` (from raw bytes, for the WASM build)
//! both just DECODE + read any ACID tag, then hand the mono samples to `analyze_core`,
//! which is the whole pipeline. For a multi-region file, `analyze_core` also re-runs
//! itself on each region's slice, so every sub-clip carries its own full analysis.
use std::path::Path;

use crate::acid::read_acid;
use crate::amplitude::amplitude_features;
use crate::distortion::distortion_analysis;
use crate::envelope::envelope_analysis;
use crate::family::classify_family;
use crate::flux::spectral_flux;
use crate::framestats::centroid_stats;
use crate::categorize::music_prod_category;
use crate::label::label_sample;
use crate::mfcc::mfcc_mean;
use crate::morphology::morphology;
use crate::partials::partial_analysis;
use crate::peak::Peak;
use crate::pitch::pitch_features;
use crate::root::{extract_root, midi_to_name};
use crate::spectrum::spectral_features;
use crate::sustain::sustain_ratio;
use crate::tags::{acoustic_tags, sound_design_tags};
use crate::transients::{count_transients, transient_onsets};
use crate::version::ANALYZER_VERSION;

/// STFT geometry for the frame-based features (flux, MFCC, centroid stats).
const N_FFT: usize = 2048;
const HOP: usize = 512;

pub fn analyze(path: &Path, root: &Path, max_len: f64) -> Option<Peak> {
    let dec = crate::decode::read_audio(path)?;
    let sr_f = dec.sample_rate as f64;
    let length = dec.mono.len() as f64 / sr_f;

    // Name/folder from the path, relative to the scanned root.
    let name = path.file_name().and_then(|x| x.to_str()).unwrap_or("").to_string();
    let parent = path.parent().unwrap_or(root);
    let mut folder = parent
        .strip_prefix(root)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    if let Some(root_name) = root.file_name().and_then(|n| n.to_str()) {
        if folder.is_empty() || folder == "." {
            folder = root_name.to_string();
        } else {
            folder = format!("{}/{}", root_name, folder);
        }
    }

    // Over the full-analysis cap: a preview-only record instead of the old silent skip.
    // The file lists, plays, and draws instantly; no DSP is spent on it.
    if length > max_len {
        return Some(preview_only_record(
            &dec.mono, dec.sample_rate, dec.bit_depth, dec.channels,
            &dec.source_format, dec.lossy, &name, &folder, &path.to_string_lossy(),
        ));
    }
    let (bpm, root_note) = read_acid(path);

    analyze_core(
        &dec.mono, &dec.raw, dec.sample_rate, dec.bit_depth, dec.channels,
        &dec.source_format, dec.lossy, &name, &folder, &path.to_string_lossy(),
        bpm, root_note, true,
    )
}

pub fn analyze_buffer(buffer: &[u8], name: &str, folder: &str, max_len: f64) -> Option<Peak> {
    let (data, raw_data, sr, bit_depth, channels) = crate::wav::read_wav_buffer(buffer)?;
    let sr_f = sr as f64;
    let length = data.len() as f64 / sr_f;
    let path = format!("{}/{}", folder, name);
    if length > max_len {
        // Same preview-only tier as the path-based entry (see analyze()).
        return Some(preview_only_record(
            &data, sr, bit_depth, channels, "WAV", false, name, folder, &path,
        ));
    }
    let (bpm, root_note) = crate::acid::read_acid_buffer(buffer);
    analyze_core(
        &data, &raw_data, sr, bit_depth, channels, "WAV", false, name, folder, &path,
        bpm, root_note, true,
    )
}

/// The shallow record for a file over the full-analysis cap: real metadata, the binary
/// waveform preview, and the cheap single-pass amplitude numbers — no STFT, no MFCC,
/// no pitch, no UCS. `analysis_depth: "preview_only"` marks it for a future deep pass.
#[allow(clippy::too_many_arguments)]
fn preview_only_record(
    data: &[f32], sr: u32, bit_depth: u16, channels: u16,
    source_format: &str, lossy_source: bool, name: &str, folder: &str, path: &str,
) -> Peak {
    let sr_f = sr as f64;
    let length = data.len() as f64 / sr_f;
    // Peak + RMS fall out of one O(n) pass — the same scan the preview fold makes.
    let mut peak_abs = 0.0f64;
    let mut sum_squares = 0.0f64;
    for &v in data {
        let a = (v as f64).abs();
        if a > peak_abs {
            peak_abs = a;
        }
        sum_squares += (v as f64) * (v as f64);
    }
    let rms = if data.is_empty() { 0.0 } else { (sum_squares / data.len() as f64).sqrt() };
    let crest = if rms > 0.0 { peak_abs / rms } else { 0.0 };

    Peak {
        metadata: crate::peak::Metadata {
            analyzer_version: ANALYZER_VERSION.to_string(),
            name: name.to_string(),
            folder: folder.to_string(),
            sub: folder.to_string(),
            path: path.to_string(),
            length_seconds: length,
            sample_rate: sr,
            bit_depth,
            channels,
            source_format: source_format.to_string(),
            lossy_source,
            dc_offset: 0.0,
            trailing_silence_ms: 0.0,
            analysis_depth: "preview_only".to_string(),
        },
        classification: crate::peak::Classification {
            group: "Unclassified".to_string(),
            reason: vec![format!(
                "1) preview-only: {:.0} s exceeds the full-analysis cap — waveform mapped, DSP skipped",
                length
            )],
            length_class: "Long".to_string(),
            ..Default::default()
        },
        envelope: crate::peak::Envelope::default(),
        spectral_features: crate::peak::SpectralFeatures {
            root_mean_square_level: rms,
            crest_factor: crest,
            ..Default::default()
        },
        musicality: crate::peak::Musicality::default(),
        unsupervised: crate::peak::Unsupervised {
            cluster: -1,
            principal_components: Vec::new(),
        },
        ucs: crate::peak::Ucs::default(),
        regions: crate::peak::Regions::default(),
        preview: crate::preview::build_preview(data),
        bioacoustic_syntax: None,
    }
}

/// The whole pipeline over already-decoded mono samples. `detect_subregions` is true for a
/// top-level file (find silence-separated regions and fully analyze each), false for the
/// per-region re-runs (so a slice does not recurse into itself).
#[allow(clippy::too_many_arguments)]
fn analyze_core(
    data: &[f32], raw_data: &[f32], sr: u32, bit_depth: u16, channels: u16,
    source_format: &str, lossy_source: bool, name: &str, folder: &str, path: &str,
    mut bpm: f64, root_note: i32, detect_subregions: bool,
) -> Option<Peak> {
    let sr_f = sr as f64;
    let length = data.len() as f64 / sr_f;

    // Feature extraction — each helper does one thing.
    let amp = amplitude_features(data, sr_f, length);
    let (pitch, harmonicity) = pitch_features(data, sr_f);
    let spec = spectral_features(data, sr_f)?; // None ⇒ too short to analyze
    let transients = count_transients(data, sr);
    let sustain = sustain_ratio(data, sr);

    // Frame-based features: one STFT shared by flux, MFCC, and centroid stats.
    let frames = crate::stft::stft_mags(data, N_FFT, HOP);
    let flux = spectral_flux(&frames);
    let mfcc = mfcc_mean(&frames, sr_f, N_FFT);
    let (centroid_mean, centroid_std) = centroid_stats(&frames, sr_f, N_FFT);

    // Measured ADSR, one entry per transient. On a one-shot that is a single
    // slice over the whole file and `file_level()` is Some; on a multi-event
    // file it is None, because a peak-relative measurement has no defined peak
    // to be relative to. The classifiers below read `representative()` — the
    // loudest slice — since "what object is vibrating" is a per-event question.
    let onsets = transient_onsets(data, sr);
    let env_analysis = envelope_analysis(data, sr, &onsets);
    let env = env_analysis.representative();

    // The morphology axis (spec §4b) rides on the STFT and the amplitude envelope.
    let (amplitude_track, envelope_rate_hz) = crate::envelope::amplitude_envelope(data, sr);
    // Silence-separated regions, off the same RMS track — only at the top level; a region's
    // own re-analysis does not carry nested regions.
    let regions = if detect_subregions {
        crate::regions::detect_regions(&amplitude_track, envelope_rate_hz, length)
    } else {
        crate::peak::Regions::default()
    };
    let morph = morphology(&frames, sr_f, N_FFT, HOP, &amplitude_track, envelope_rate_hz);
    // One pass answers both "how voiced is this" and "is this a vocal".
    let voicing = crate::vad::voice_activity(data, sr);
    let (voicing_ratio, is_vocal) = (voicing.ratio, voicing.is_speech);

    // Advanced Stats
    let (mid_rms, side_rms) = crate::advanced_stats::analyze_stereo_width(raw_data, channels);
    let lufs = crate::advanced_stats::get_integrated_lufs(raw_data, channels, sr);
    let onset_periodicity = crate::advanced_stats::onset_periodicity(
        &crate::advanced_stats::detect_transient_onsets(data, 512),
    );
    let (dc_offset, trailing_silence_ms) = crate::advanced_stats::calculate_qa_metrics(data, sr);

    // ROOT note (musical key). Prefer the embedded ACID root when present, otherwise detect
    // it from the spectrum (FFT + harmonic product spectrum).
    let fft_root = extract_root(data, sr_f);
    let (root_name, root_hz, root_cents) = if root_note >= 0 {
        (midi_to_name(root_note), 0.0, 0.0)
    } else {
        (fft_root.note.clone(), fft_root.hz, fft_root.cents)
    };

    // Partials vs the fundamental: harmonic series or detuned/metallic?
    let f0 = if fft_root.hz > 0.0 { fft_root.hz } else { pitch };
    let parts = partial_analysis(data, sr_f, f0);

    // Distortion: THD + clipping density, with the crest factor as the third metric.
    let dist = distortion_analysis(data, sr_f, f0, amp.crest);

    // Blind classifications — from the measurements alone, before the name is looked at.
    let acoustic = acoustic_tags(harmonicity, spec.flatness, &parts, length, &env);
    let sound_design =
        sound_design_tags(&env, transients, harmonicity, spec.centroid, spec.low, spec.high, pitch, fft_root.hz);

    // ---- The file name (curated taxonomy on top of the acoustic evidence).
    let mut l = label_sample(
        folder, name, length, transients, bpm, harmonicity, sustain,
        amp.attack, amp.crest, spec.centroid, spec.low, spec.high,
    );
    let family = classify_family(&l.group, &l.subgroup, &acoustic, flux, l.sustained);

    if l.group == "Unclassified" {
        if family.contains(&"Membranophone".to_string()) || family.contains(&"Idiophone".to_string()) {
            l.group = "Perc".to_string();
            l.subgroup = "Acoustic Guess".to_string();
            l.reason = "Acoustic ML Fallback (ZCR/Inharmonicity)".to_string();
        } else if family.contains(&"Electrophone".to_string()) {
            l.group = "Keyboards".to_string();
            l.subgroup = "Synth".to_string();
            l.reason = "Acoustic ML Fallback (Sustained Harmonic)".to_string();
        } else if family.contains(&"Chordophone".to_string()) {
            l.group = "Strings".to_string();
            l.reason = "Acoustic ML Fallback (Plucked Harmonic)".to_string();
        }
    }

    let is_loop = l.group == "Loops/Patterns";
    if bpm == 0.0 && (is_loop || l.timbre == "Loop" || l.length_class == "Loop") {
        bpm = crate::tempo::estimate_bpm(&frames, sr_f, HOP);
    }
    bpm = bpm.round();
    let mut music_class =
        music_prod_category(&l.group, &l.subgroup, is_loop, env_analysis.shape, env).to_string();

    // The voice detector overrides the NAME — but not a loop.
    if is_vocal && !is_loop {
        music_class = crate::categorize::family_of("Vocal").unwrap_or("MISC").to_string();
    }

    // Percussive hits rarely carry a meaningful root note.
    let (root_name, root_hz, root_cents) =
        if crate::categorize::is_percussive_family(&music_class) && root_note < 0 && harmonicity < 0.6 {
            (String::new(), 0.0, 0.0)
        } else {
            (root_name, root_hz, root_cents)
        };

    // Three-part membership reason: name, envelope, spectral evidence.
    let band = if spec.low >= spec.mid && spec.low >= spec.high {
        ("low", spec.low)
    } else if spec.high >= spec.mid {
        ("high", spec.high)
    } else {
        ("mid", spec.mid)
    };
    let root_part = if root_name.is_empty() { String::new() } else { format!(" · root {}", root_name) };
    let reason = vec![
        format!("1) {}", l.reason),
        // On a multi-event file the ADSR quoted here is the LOUDEST SLICE's, not
        // the file's — say so, rather than presenting a per-event number as if it
        // described the whole recording.
        if env_analysis.single_event {
            format!("2) envelope {} (attack {:.0} ms, sustain {:.0}%, 1 transient)",
                env_analysis.shape, env.attack * 1000.0, env.sustain * 100.0)
        } else {
            format!(
                "2) envelope {} ({} transients — loudest of {} slices: attack {:.0} ms, sustain {:.0}%)",
                env_analysis.shape, transients, env_analysis.slices.len(),
                env.attack * 1000.0, env.sustain * 100.0)
        },
        format!("3) {} · {}-band {:.0}%{}", acoustic.join("+"), band.0, band.1 * 100.0, root_part),
    ];

    let note_map = crate::note_map::extract_note_root_key_beat_marker_map(
        data, sr, bpm, &root_name, root_note, &regions.regions,
    );

    let mut peak = finish(Peak {
        metadata: crate::peak::Metadata {
            analyzer_version: ANALYZER_VERSION.to_string(),
            name: name.to_string(),
            folder: folder.to_string(),
            sub: folder.to_string(),
            path: path.to_string(),
            length_seconds: length,
            sample_rate: sr,
            bit_depth,
            channels,
            source_format: source_format.to_string(),
            lossy_source,
            dc_offset,
            trailing_silence_ms,
            analysis_depth: "full".to_string(),
        },
        classification: crate::peak::Classification {
            group: l.group,
            reason,
            timbre: l.timbre,
            length_class: l.length_class,
            subgroup: l.subgroup,
            audit: l.audit,
            acoustic_types: acoustic,
            sound_design_roles: sound_design,
            instrument_family: family,
        },
        envelope: crate::peak::Envelope {
            transient_count: transients,
            attack_seconds: amp.attack,
            sustain_ratio: sustain,
            sustained: l.sustained,
            // Peak-relative, so file-level only when the file holds one event.
            // `file_level()` is None otherwise — the numbers live per-slice.
            envelope_attack_seconds: env_analysis.file_level().map(|e| e.attack),
            envelope_decay_seconds: env_analysis.file_level().map(|e| e.decay),
            envelope_sustain_level: env_analysis.file_level().map(|e| e.sustain),
            envelope_release_seconds: env_analysis.file_level().map(|e| e.release),
            envelope_temporal_centroid: env_analysis.file_level().map(|e| e.centroid),
            envelope_skewness: env_analysis.file_level().map(|e| e.skew),
            envelope_kurtosis: env_analysis.file_level().map(|e| e.kurt),
            envelope_shape: env_analysis.shape.to_string(),
            decay_time_seconds_60db: env_analysis.file_level().and_then(|e| e.decay_time_60db),
            slices: env_analysis
                .slices
                .iter()
                .map(|s| crate::peak::EnvelopeSlice {
                    index: s.index,
                    start_seconds: s.start_seconds,
                    end_seconds: s.end_seconds,
                    relative_level: s.relative_level,
                    envelope_attack_seconds: s.envelope.attack,
                    envelope_decay_seconds: s.envelope.decay,
                    envelope_sustain_level: s.envelope.sustain,
                    envelope_release_seconds: s.envelope.release,
                    envelope_temporal_centroid: s.envelope.centroid,
                    envelope_skewness: s.envelope.skew,
                    envelope_kurtosis: s.envelope.kurt,
                    envelope_shape: s.envelope.shape.to_string(),
                    decay_time_seconds_60db: s.envelope.decay_time_60db,
                })
                .collect(),
            onset_periodicity,
            onset_rate_per_second: if length > 0.0 { Some(transients as f64 / length) } else { None },
        },
        spectral_features: crate::peak::SpectralFeatures {
            root_mean_square_level: amp.rms,
            crest_factor: amp.crest,
            zero_crossings_per_second: amp.zcr,
            complexity: spec.complexity,
            spectral_centroid_hz: spec.centroid,
            spectral_rolloff_hz: spec.rolloff,
            spectral_flatness: spec.flatness,
            low_band_energy: spec.low,
            mid_band_energy: spec.mid,
            high_band_energy: spec.high,
            spectral_flux: flux,
            harmonicity,
            inharmonicity: parts.inharmonicity,
            partial_count: parts.count,
            mel_frequency_cepstral_coefficients: mfcc,
            spectral_centroid_mean_hz: centroid_mean,
            spectral_centroid_deviation_hz: centroid_std,
            total_harmonic_distortion: dist.thd,
            clipping_density: dist.clipping,
            distortion: dist.label.to_string(),
            stationarity: morph.stationarity,
            spectral_entropy: morph.spectral_entropy,
            spectral_slope_db_per_octave: morph.spectral_slope_db_per_octave,
            band_limit_high_hz: morph.band_limit_high_hz,
            spectral_centroid_slope_hz_per_second: morph.spectral_centroid_slope_hz_per_second,
            syllabic_modulation_energy: morph.syllabic_modulation_energy,
            voicing_ratio,
            mid_rms,
            side_rms,
            lufs,
        },
        musicality: crate::peak::Musicality {
            pitch_hz: pitch,
            pitch_slope_semitones_per_second: morph.pitch_slope_semitones_per_second,
            root_note_name: root_name,
            root_frequency_hz: root_hz,
            root_cents_offset: root_cents,
            beats_per_minute: (bpm * 10.0).round() / 10.0,
            root_midi_note: root_note,
            chromagram: spec.chromagram,
            note_root_key_beat_marker_map: Some(note_map),
        },
        unsupervised: crate::peak::Unsupervised {
            cluster: -1,
            principal_components: Vec::new(),
        },
        ucs: crate::peak::Ucs::default(),
        regions,
        // The binary waveform preview — every file gets one, computed from the PCM
        // already in memory, so the UI paints without decoding any audio.
        preview: crate::preview::build_preview(data),
        // Filled in below, once every region carries its own analysis — the
        // syntax is read off those, so it cannot be computed before they exist.
        bioacoustic_syntax: None,
    });

    // Per-region full analysis: re-run the whole pipeline on each region's slice so every
    // sub-clip is fully measured + classified. Only for genuinely multi-region files — a
    // single region is the whole file, so its analysis would just duplicate the parent's.
    if detect_subregions && peak.regions.count > 1 {
        let ch = channels.max(1) as usize;
        let bounds: Vec<(usize, f64, f64)> = peak.regions.regions.iter()
            .map(|r| (r.index, r.start_seconds, r.end_seconds)).collect();
        for (i, start_s, end_s) in bounds {
            let s0 = ((start_s * sr_f) as usize).min(data.len());
            let s1 = ((end_s * sr_f) as usize).min(data.len());
            if s1 <= s0 { continue; }
            let r0 = (s0 * ch).min(raw_data.len());
            let r1 = (s1 * ch).min(raw_data.len());
            let raw_slice = if r1 > r0 { &raw_data[r0..r1] } else { &data[s0..s1] };
            if let Some(sub) = analyze_core(
                &data[s0..s1], raw_slice, sr, bit_depth, channels, source_format, lossy_source,
                name, folder, path, 0.0, -1, false,
            ) {
                if let Some(region) = peak.regions.regions.get_mut(i) {
                    region.analysis = Some(Box::new(sub));
                }
            }
        }

        // Bioacoustic syntax LAST, for the same reason the UCS verdict is last:
        // it reads the finished record. Every other extractor describes one
        // sound; this one describes the ORDER of them and the junctions in
        // between, so it needs every region's own analysis already in place.
        peak.bioacoustic_syntax = crate::syntax::bioacoustic_syntax(
            &peak.regions, &frames, sr_f, N_FFT, HOP, &amplitude_track, envelope_rate_hz,
        );
    }

    Some(peak)
}

/// Classify a finished record. UCS scoring runs LAST, against the completed `Peak`, so the
/// classifier can read every extracted feature by its spec name.
fn finish(mut p: Peak) -> Peak {
    let v = crate::ucs::classify(&p);
    p.ucs.category = v.category;
    p.ucs.subcategory = v.subcategory;
    p.ucs.id = v.id;
    p.ucs.confidence = v.confidence;
    p.ucs.alternatives = v.alternatives;
    p.ucs.synonyms = v.synonyms;
    p.ucs.reason = v.reason;
    p
}
