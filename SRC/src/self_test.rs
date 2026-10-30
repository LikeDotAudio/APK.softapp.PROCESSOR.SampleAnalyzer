// Part of the APK.audio project — http://APK.audio — made by Anthony Kuzub
// MIT Licence. Free, for everyone, for ever. Full text in LICENSE at the root.
//! The worker's self test: the runner answers `outgoing/selftest_run` with it.
//!
//! All `Depth::Own` — a worker has no hardware. The decisive check runs a real
//! job through [`crate::handle_job`] on a 440 Hz probe built in memory, so a
//! pass means the analyzer in THIS image decodes a WAV, can stage it in the
//! container's tmpfs, and hears its pitch.

use apkaudio_contracts::self_test::{Check, Depth, Report};

use crate::{CONFIG_INI, Limits, MODULE};

pub fn self_test(depth: Depth) -> Report {
    let mut report = Report::new(MODULE, depth);
    report.record(Check::from_result(
        "landing_spots_build",
        Depth::Own,
        "job, result and the self-test leaves address legal topics",
        landing_spots(),
    ));
    report.record(Check::from_result(
        "config_section_still_names_this_module",
        Depth::Own,
        "config.ini declares this module under the name the bus knows it by",
        config_declares_module(),
    ));
    report.record(Check::from_result(
        "a_job_round_trips_through_the_analyzer",
        Depth::Own,
        "a 440 Hz WAV job comes back ok, fully analysed, pitch within 5 Hz",
        probe_job(),
    ));
    report
}

fn landing_spots() -> Result<(), String> {
    use apkaudio_contracts::{baremetal_io, self_test as st};
    baremetal_io::module_job(MODULE).map_err(|e| format!("{e:?}"))?;
    baremetal_io::module_result(MODULE).map_err(|e| format!("{e:?}"))?;
    st::run_topic(MODULE).map_err(|e| format!("{e:?}"))?;
    Ok(())
}

fn config_declares_module() -> Result<(), String> {
    let wanted = format!("[{MODULE}]");
    if CONFIG_INI.lines().any(|line| line.trim() == wanted) {
        Ok(())
    } else {
        Err(format!("config.ini does not declare {wanted}"))
    }
}

fn probe_job() -> Result<(), String> {
    use base64::Engine as _;
    let wav = base64::engine::general_purpose::STANDARD.encode(crate::probe_wav());
    let job = serde_json::json!({"job_id": "selftest", "filename": "probe.wav", "content_b64": wav});
    let out = crate::handle_job(job.to_string().as_bytes(), Limits::from_ini(CONFIG_INI));
    let depth = out["result"]["metadata"]["analysis_depth"].as_str();
    let pitch = out["result"]["musicality"]["pitch_hz"].as_f64();
    match (out["ok"].as_bool(), depth, pitch) {
        (Some(true), Some("full"), Some(p)) if (p - 440.0).abs() < 5.0 => Ok(()),
        _ => Err(format!("unexpected answer: ok={} error={} depth={depth:?} pitch={pitch:?}", out["ok"], out["error"])),
    }
}

pub fn self_test_json(depth_word: &str) -> String {
    apkaudio_contracts::self_test::run_as_json(MODULE, depth_word, self_test)
}

#[cfg(test)]
mod tests {
    use super::*;
    use apkaudio_contracts::self_test::Verdict;

    #[test]
    fn the_self_test_passes_against_this_build() {
        let report = self_test(Depth::Own);
        assert_eq!(report.verdict(), Verdict::Pass, "{}", report.headline());
    }
}
