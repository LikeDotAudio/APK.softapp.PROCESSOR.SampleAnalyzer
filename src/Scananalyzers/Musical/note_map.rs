//! Note Root Key Beat Marker Map extractor module.
//!
//! Orchestrates transient / region / spectral / root analysis to produce a complete
//! note-root-key-beat-marker map per file and chunk.

use crate::amplitude::amplitude_features;
use crate::peak::{BeatMarker, ChunkNoteMap, NoteRootKeyBeatMarkerMap, Region};
use crate::pitch::pitch_features;
use crate::root::extract_root;

/// Extract beat markers, root keys, and note mapping across all chunks/regions of an audio file.
pub fn extract_note_root_key_beat_marker_map(
    data: &[f32],
    sr: u32,
    bpm: f64,
    global_root_name: &str,
    global_midi_note: i32,
    regions: &[Region],
) -> NoteRootKeyBeatMarkerMap {
    let sr_f = sr as f64;
    let length_seconds = if sr > 0 { data.len() as f64 / sr_f } else { 0.0 };

    // 1. Generate Beat Markers based on BPM (defaulting to 120.0 if not estimated).
    let effective_bpm = if bpm > 30.0 && bpm < 300.0 { bpm } else { 120.0 };
    let seconds_per_beat = 60.0 / effective_bpm;
    let total_beats = (length_seconds / seconds_per_beat).ceil() as usize;

    let mut beat_markers = Vec::with_capacity(total_beats);
    for b in 0..total_beats {
        let ts = b as f64 * seconds_per_beat;
        if ts > length_seconds {
            break;
        }
        let sample_idx = (ts * sr_f) as u64;
        let is_downbeat = b % 4 == 0;
        let bar = (b / 4) + 1;
        let beat = (b % 4) + 1;

        beat_markers.push(BeatMarker {
            index: b,
            timestamp_seconds: (ts * 1000.0).round() / 1000.0,
            sample_index: sample_idx,
            is_downbeat,
            bar,
            beat,
        });
    }

    // 2. Extract Chunk Note Maps.
    let mut chunk_maps = Vec::new();

    if !regions.is_empty() {
        for (i, r) in regions.iter().enumerate() {
            let s0 = ((r.start_seconds * sr_f) as usize).min(data.len());
            let s1 = ((r.end_seconds * sr_f) as usize).min(data.len());
            if s1 <= s0 {
                continue;
            }
            let chunk_data = &data[s0..s1];

            // Extract pitch and root note for this chunk
            let (pitch, _) = pitch_features(chunk_data, sr_f);
            let root_res = extract_root(chunk_data, sr_f);
            let amp = amplitude_features(chunk_data, sr_f, r.duration_seconds);

            // Find nearest beat marker index
            let nearest_beat_idx = beat_markers
                .iter()
                .min_by(|a, b| {
                    (a.timestamp_seconds - r.start_seconds)
                        .abs()
                        .partial_cmp(&(b.timestamp_seconds - r.start_seconds).abs())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|bm| bm.index);

            let note_name = if !root_res.note.is_empty() {
                root_res.note
            } else if !global_root_name.is_empty() {
                global_root_name.to_string()
            } else {
                "Unpitched".to_string()
            };

            let midi_note = if root_res.hz > 0.0 {
                let midi_f = 69.0 + 12.0 * (root_res.hz / 440.0).log2();
                midi_f.round() as i32
            } else {
                global_midi_note
            };

            chunk_maps.push(ChunkNoteMap {
                chunk_index: i,
                start_seconds: (r.start_seconds * 1000.0).round() / 1000.0,
                end_seconds: (r.end_seconds * 1000.0).round() / 1000.0,
                duration_seconds: (r.duration_seconds * 1000.0).round() / 1000.0,
                root_note_name: note_name,
                root_midi_note: midi_note,
                root_frequency_hz: (root_res.hz * 100.0).round() / 100.0,
                pitch_hz: (pitch * 100.0).round() / 100.0,
                cents_offset: (root_res.cents * 10.0).round() / 10.0,
                peak_amplitude: ((amp.rms * amp.crest) * 1000.0).round() / 1000.0,
                nearest_beat_index: nearest_beat_idx,
            });
        }
    } else {
        // Fallback if no sub-regions detected: divide by beat markers or treat whole file as 1 chunk
        let (pitch, _) = pitch_features(data, sr_f);
        let root_res = extract_root(data, sr_f);
        let amp = amplitude_features(data, sr_f, length_seconds);

        let note_name = if !root_res.note.is_empty() {
            root_res.note
        } else if !global_root_name.is_empty() {
            global_root_name.to_string()
        } else {
            "Unpitched".to_string()
        };

        chunk_maps.push(ChunkNoteMap {
            chunk_index: 0,
            start_seconds: 0.0,
            end_seconds: (length_seconds * 1000.0).round() / 1000.0,
            duration_seconds: (length_seconds * 1000.0).round() / 1000.0,
            root_note_name: note_name,
            root_midi_note: global_midi_note,
            root_frequency_hz: (root_res.hz * 100.0).round() / 100.0,
            pitch_hz: (pitch * 100.0).round() / 100.0,
            cents_offset: (root_res.cents * 10.0).round() / 10.0,
            peak_amplitude: ((amp.rms * amp.crest) * 1000.0).round() / 1000.0,
            nearest_beat_index: Some(0),
        });
    }

    NoteRootKeyBeatMarkerMap {
        global_root_note: global_root_name.to_string(),
        global_midi_note,
        global_bpm: effective_bpm,
        total_beats: beat_markers.len(),
        beat_markers,
        chunk_maps,
    }
}
