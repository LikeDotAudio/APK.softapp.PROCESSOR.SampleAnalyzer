//! The binary waveform preview — interleaved signed 8-bit min,max peak pairs over fixed
//! sample bins, base64-encoded into the `.PEAK`'s `preview` section. Computed for every
//! file in the same pass as the analysis, so the UI can paint a waveform without decoding
//! any audio. Shape follows the industry standard (BWF `levl`, BBC audiowaveform):
//! see Documentation/Audit/binary_peak_preview_audit.md.

use crate::peak::Preview;

pub const PREVIEW_VERSION: u32 = 1;
/// Bin-count ceiling: worst case 16384 bins × 2 bytes = 32 KB raw, ~43 KB as base64 —
/// the sidecar cost of any file, no matter how long.
pub const MAXIMUM_BIN_COUNT: usize = 16384;
/// Finest resolution: 256 samples per bin (~5 ms at 48 kHz), the BWF default block size.
pub const MINIMUM_SAMPLES_PER_BIN: usize = 256;

/// The resolution policy: the smallest power of two (≥ the floor) that keeps the bin
/// count under the ceiling. Short files get fine bins; hour-long files stay ≤ ~43 KB.
pub fn samples_per_bin_for(total_samples: usize) -> usize {
    let mut samples_per_bin = MINIMUM_SAMPLES_PER_BIN;
    while total_samples.div_ceil(samples_per_bin) > MAXIMUM_BIN_COUNT {
        samples_per_bin *= 2;
    }
    samples_per_bin
}

/// Fold mono samples into the interleaved signed 8-bit min,max pairs and wrap them in
/// the versioned `preview` section. One O(n) scan — noise next to the STFT.
pub fn build_preview(mono: &[f32]) -> Preview {
    if mono.is_empty() {
        return Preview::default();
    }
    let samples_per_bin = samples_per_bin_for(mono.len());
    let bin_count = mono.len().div_ceil(samples_per_bin);
    let mut bytes = Vec::with_capacity(bin_count * 2);
    for bin in 0..bin_count {
        let start = bin * samples_per_bin;
        let end = (start + samples_per_bin).min(mono.len());
        let (mut minimum, mut maximum) = (f32::MAX, f32::MIN);
        for &value in &mono[start..end] {
            if value < minimum {
                minimum = value;
            }
            if value > maximum {
                maximum = value;
            }
        }
        bytes.push(quantize(minimum) as u8);
        bytes.push(quantize(maximum) as u8);
    }
    Preview {
        preview_version: PREVIEW_VERSION,
        samples_per_bin: samples_per_bin as u32,
        bin_count: bin_count as u32,
        bits_per_value: 8,
        channel_mode: "mono_mixdown".to_string(),
        peak_data_base64: encode_base64(&bytes),
    }
}

/// Signed 8-bit quantization: ±1.0 → ±127. Two's complement survives the `as u8` cast
/// and the browser's `Int8Array` view reads it back exactly.
fn quantize(value: f32) -> i8 {
    (value.clamp(-1.0, 1.0) * 127.0).round() as i8
}

/// Standard base64 (RFC 4648, padded). Hand-rolled to keep both the native and wasm
/// builds dependency-free — the browser decodes it with plain `atob`.
fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc_vectors() {
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
        assert_eq!(encode_base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn resolution_policy_caps_bins() {
        // 5 s @ 48 kHz stays at the floor.
        assert_eq!(samples_per_bin_for(240_000), 256);
        // 1 h @ 48 kHz: 172.8 M samples / 16384 ≈ 10547 bins.
        let spb = samples_per_bin_for(172_800_000);
        assert_eq!(spb, 16384);
        assert!(172_800_000_usize.div_ceil(spb) <= MAXIMUM_BIN_COUNT);
    }

    #[test]
    fn fold_captures_extremes() {
        // One negative spike and one positive spike, in different bins.
        let mut mono = vec![0.0f32; 1024];
        mono[10] = -1.0;
        mono[700] = 0.5;
        let p = build_preview(&mono);
        assert_eq!(p.samples_per_bin, 256);
        assert_eq!(p.bin_count, 4);
        assert_eq!(p.bits_per_value, 8);
        // Decode the first pair: min = -127, max = 0.
        let decoded = decode_for_test(&p.peak_data_base64);
        assert_eq!(decoded[0], -127); // bin 0 min (the -1.0 spike)
        assert_eq!(decoded[1], 0);    // bin 0 max
        assert_eq!(decoded[5], 64);   // bin 2 max (0.5 * 127 rounded)
    }

    fn decode_for_test(s: &str) -> Vec<i8> {
        const T: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut bits = 0u32;
        let mut nbits = 0;
        let mut out = Vec::new();
        for c in s.chars().filter(|&c| c != '=') {
            bits = (bits << 6) | T.find(c).unwrap() as u32;
            nbits += 6;
            if nbits >= 8 {
                nbits -= 8;
                out.push(((bits >> nbits) & 0xFF) as u8 as i8);
            }
        }
        out
    }
}
