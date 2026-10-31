//! Decode any supported audio file to mono + interleaved f32.
//!
//! WAV keeps its existing `hound` path: it is exact, fast, and reports a true
//! bit depth. Everything else goes through symphonia (MP3, FLAC, AIFF, OGG,
//! M4A/AAC/ALAC).
//!
//! A lossy source is flagged, not corrected. The encoder leaves fingerprints —
//! a lowpass around 16 kHz, and decoded peaks that overshoot 0 dBFS — which
//! land squarely on the brightness and clipping features. We record the raw
//! numbers honestly and let the consumer decide how much to trust them; see
//! `LOSSY_UNRELIABLE` in ucs.rs, which is where that decision is actually made.
use std::fs::File;
use std::io::Cursor;
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// One decoded file. `raw` is interleaved; `mono` is the downmix.
pub struct Decoded {
    pub mono: Vec<f32>,
    pub raw: Vec<f32>,
    pub sample_rate: u32,
    pub bit_depth: u16, // 0 when the format has no meaningful bit depth (MP3, AAC, Vorbis)
    pub channels: u16,
    pub source_format: String, // WAV / MP3 / FLAC / AIFF / OGG / M4A
    pub lossy: bool,
}

/// Extensions we will attempt. Anything else is not an audio file to us.
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "wav", "wave", "mp3", "flac", "aif", "aiff", "aifc", "ogg", "oga", "m4a", "mp4", "aac",
];

/// True for formats that discard signal. Drives the leniency downstream.
fn is_lossy(ext: &str) -> bool {
    matches!(ext, "mp3" | "ogg" | "oga" | "m4a" | "mp4" | "aac")
}

fn format_name(ext: &str) -> &'static str {
    match ext {
        "wav" | "wave" => "WAV",
        "mp3" => "MP3",
        "flac" => "FLAC",
        "aif" | "aiff" | "aifc" => "AIFF",
        "ogg" | "oga" => "OGG",
        "m4a" | "mp4" | "aac" => "M4A",
        _ => "UNKNOWN",
    }
}

pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|x| x.to_str())
        .map(|x| {
            let e = x.to_ascii_lowercase();
            AUDIO_EXTENSIONS.contains(&e.as_str())
        })
        .unwrap_or(false)
}

fn downmix(raw: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return raw.to_vec();
    }
    raw.chunks(channels)
        .map(|frame| frame.iter().copied().sum::<f32>() / channels as f32)
        .collect()
}

/// Decode `path`. Returns None if the file is unreadable or empty.
pub fn read_audio(path: &Path) -> Option<Decoded> {
    let ext = path
        .extension()
        .and_then(|x| x.to_str())
        .map(|x| x.to_ascii_lowercase())
        .unwrap_or_default();

    // WAV: keep the exact, well-tested hound path.
    if ext == "wav" || ext == "wave" {
        let (mono, raw, sample_rate, bit_depth, channels) = crate::wav::read_wav(path)?;
        return Some(Decoded {
            mono,
            raw,
            sample_rate,
            bit_depth,
            channels,
            source_format: "WAV".to_string(),
            lossy: false,
        });
    }

    let file = File::open(path).ok()?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    decode_symphonia(stream, &ext)
}

/// Decode from an in-memory buffer instead of a path — the entry point the WASM
/// build calls so the web front can render a preview for compressed formats
/// (MP3/OGG/M4A/AAC/FLAC/AIFF) that the browser's own decoder may reject.
///
/// `name` is only used for its extension hint. When it is absent or wrong, WAV is
/// still caught by its RIFF/WAVE magic, and every other codec is content-probed by
/// symphonia — so a blob URL with no extension still decodes.
pub fn read_audio_buffer(buffer: &[u8], name: &str) -> Option<Decoded> {
    let ext = name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();

    // WAV by extension or by RIFF/WAVE magic → the exact, well-tested hound path.
    let is_wav_magic =
        buffer.len() >= 12 && &buffer[0..4] == b"RIFF" && &buffer[8..12] == b"WAVE";
    if ext == "wav" || ext == "wave" || is_wav_magic {
        if let Some((mono, raw, sample_rate, bit_depth, channels)) =
            crate::wav::read_wav_buffer(buffer)
        {
            return Some(Decoded {
                mono,
                raw,
                sample_rate,
                bit_depth,
                channels,
                source_format: "WAV".to_string(),
                lossy: false,
            });
        }
    }

    // symphonia's Cursor MediaSource needs an owned, 'static buffer.
    let stream = MediaSourceStream::new(Box::new(Cursor::new(buffer.to_vec())), Default::default());
    decode_symphonia(stream, &ext)
}

/// Shared symphonia decode loop for `read_audio` (file) and `read_audio_buffer`
/// (in-memory). `ext` is the lowercase extension used as a probe hint and to label
/// the format; it may be empty, in which case symphonia probes by content.
fn decode_symphonia(stream: MediaSourceStream, ext: &str) -> Option<Decoded> {
    let mut hint = Hint::new();
    if !ext.is_empty() {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .ok()?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)?;
    let track_id = track.id;
    let params = track.codec_params.clone();

    let mut decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions::default())
        .ok()?;

    let mut raw: Vec<f32> = Vec::new();
    let mut sample_rate = params.sample_rate.unwrap_or(0);
    let mut channels = params.channels.map(|c| c.count()).unwrap_or(0);
    let mut buf: Option<SampleBuffer<f32>> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            // Any error here is end-of-stream or a truncated tail; keep what we
            // decoded rather than losing the whole file.
            Err(_) => break,
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                if sample_rate == 0 {
                    sample_rate = spec.rate;
                }
                if channels == 0 {
                    channels = spec.channels.count();
                }
                let sb = buf.get_or_insert_with(|| {
                    SampleBuffer::<f32>::new(decoded.capacity() as u64, spec)
                });
                sb.copy_interleaved_ref(decoded);
                raw.extend_from_slice(sb.samples());
            }
            // A corrupt frame mid-file is survivable: skip it, keep decoding.
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(_) => break,
        }
    }

    if raw.is_empty() || sample_rate == 0 {
        return None;
    }
    let channels = channels.max(1);
    let mono = downmix(&raw, channels);

    Some(Decoded {
        mono,
        raw,
        sample_rate,
        // Lossy codecs have no bit depth; reporting one would be a lie.
        bit_depth: params.bits_per_sample.unwrap_or(0) as u16,
        channels: channels as u16,
        source_format: format_name(ext).to_string(),
        lossy: is_lossy(ext),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every extension we advertise must actually decode. AIFF was listed here
    /// while symphonia's `aiff` feature was off, so AIFF files were discovered
    /// and then silently dropped — the exact failure this crate exists to avoid.
    #[test]
    fn every_advertised_extension_is_classifiable() {
        for ext in AUDIO_EXTENSIONS {
            let p = std::path::PathBuf::from(format!("x.{ext}"));
            assert!(is_audio(&p), "{ext} advertised but is_audio() rejects it");
            assert_ne!(
                format_name(ext),
                "UNKNOWN",
                "{ext} advertised but has no format name"
            );
        }
    }

    #[test]
    fn lossy_formats_are_flagged_and_lossless_ones_are_not() {
        for ext in ["mp3", "ogg", "oga", "m4a", "mp4", "aac"] {
            assert!(is_lossy(ext), "{ext} should be flagged lossy");
        }
        for ext in ["wav", "wave", "flac", "aif", "aiff", "aifc"] {
            assert!(!is_lossy(ext), "{ext} is lossless and must not be flagged");
        }
    }
}
