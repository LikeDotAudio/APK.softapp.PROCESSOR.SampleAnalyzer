//! Multi-region detection: find the distinct sounding stretches inside one file,
//! separated by silence, and report an in-point / out-point per region.
//!
//! A field recording or a comp'd take is often several sounds with gaps between
//! them — a door that opens, a pause, then closes. `count > 1` is what flags a file as
//! "has multiple regions" for the library.
//!
//! The detection itself lives in the shared `extractor_engine` crate — the SAME code the
//! interactive web Extractor runs (compiled to WASM). This function calls it with
//! `DetectParams::legacy()`, whose values reproduce the historical algorithm exactly, so
//! the stored `.PEAK` regions are unchanged while the two paths can no longer drift apart.
//! (The Extractor UI passes the improved defaults instead — hysteresis, attack/release
//! padding, transient-onset and zero-crossing snapping.)

use crate::peak::{Region, Regions};

/// Silence floor, in dB below the file's loudest frame. Anything quieter is a gap.
pub const DEFAULT_THRESHOLD_DECIBELS: f64 = -40.0;
/// A quiet stretch shorter than this is a dip *within* a region, not a break between two.
pub const DEFAULT_MINIMUM_SILENCE_SECONDS: f64 = 0.15;
/// A sounding stretch shorter than this is a click, not a region — dropped.
pub const DEFAULT_MINIMUM_REGION_SECONDS: f64 = 0.05;

/// Detect regions from the RMS envelope (the `amplitude_envelope` track) at `rate_hz`
/// frames per second. `length_seconds` clamps the final out-point. Delegates to
/// `extractor_engine` with the legacy parameters, so the result is identical to the
/// historical in-house detector.
pub fn detect_regions(envelope: &[f64], rate_hz: f64, length_seconds: f64) -> Regions {
    let params = extractor_engine::DetectParams::legacy();
    let regions: Vec<Region> =
        extractor_engine::detect_regions_with_params(envelope, rate_hz, length_seconds, &params)
            .into_iter()
            .map(|r| Region {
                index: r.index,
                start_seconds: r.start_seconds,
                end_seconds: r.end_seconds,
                duration_seconds: r.duration_seconds,
                peak_amplitude: r.peak_amplitude,
                name: r.name,
                analysis: None, // filled in by analyze_core for multi-region files
            })
            .collect();

    Regions {
        count: regions.len(),
        detection_threshold_decibels: DEFAULT_THRESHOLD_DECIBELS,
        minimum_silence_seconds: DEFAULT_MINIMUM_SILENCE_SECONDS,
        minimum_region_seconds: DEFAULT_MINIMUM_REGION_SECONDS,
        regions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Build an RMS-like envelope at 200 Hz: `spec` is (loud?, seconds) blocks.
    fn envelope(spec: &[(bool, f64)]) -> (Vec<f64>, f64) {
        let rate = 200.0;
        let mut v = Vec::new();
        for &(loud, secs) in spec {
            let frames = (secs * rate).round() as usize;
            for _ in 0..frames {
                v.push(if loud { 1.0 } else { 0.0 });
            }
        }
        (v, rate)
    }

    #[test]
    fn two_sounds_with_a_real_gap_are_two_regions() {
        let (env, rate) = envelope(&[(true, 0.4), (false, 0.3), (true, 0.4)]);
        let r = detect_regions(&env, rate, 1.1);
        assert_eq!(r.count, 2, "expected two regions");
        assert!(r.regions[0].start_seconds < 0.05);
        assert!((r.regions[1].start_seconds - 0.7).abs() < 0.02);
    }

    #[test]
    fn a_brief_dip_does_not_split_a_region() {
        // 50 ms gap < 150 ms minimum silence → one region.
        let (env, rate) = envelope(&[(true, 0.4), (false, 0.05), (true, 0.4)]);
        let r = detect_regions(&env, rate, 0.85);
        assert_eq!(r.count, 1, "a short dip must not split the sound");
    }

    #[test]
    fn silence_has_no_regions() {
        let (env, rate) = envelope(&[(false, 1.0)]);
        assert_eq!(detect_regions(&env, rate, 1.0).count, 0);
        assert_eq!(detect_regions(&[], 200.0, 0.0).count, 0);
    }
}
