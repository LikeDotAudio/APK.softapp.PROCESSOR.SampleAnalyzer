// Part of the APK.audio project — http://APK.audio — made by Anthony Kuzub
// MIT Licence. Free, for everyone, for ever. Full text in LICENSE at the root.
//! The Scanalyzer DSP core as a worker (LOOSE-workers-convert-rest).
//!
//! ```text
//!   client ── APK.audio/System/Protocols/sample_analyzer/outgoing/job ──▶ worker
//!   worker ── APK.audio/System/Protocols/sample_analyzer/incoming/result ─▶ client
//! ```
//!
//! ONE JOB IN, ONE RESULT OUT, MATCHED BY `job_id`. Neither leaf retains (see
//! `contracts/rust/src/baremetal_io.rs`). The job carries the audio file's
//! bytes; the result carries the `.PEAK` record `oa_sample_analyzer` writes
//! beside a file in a batch scan — here nothing is written anywhere but the
//! answer.
//!
//! THE FILE TOUCHES DISK FOR ONE JOB ONLY. `analyze::analyze` takes a path:
//! it picks the decoder by extension (WAV through hound, the rest through
//! symphonia) and reads the ACID chunk from the file. Rather than fork that
//! logic for a buffer (`analyze_buffer` reads WAV only), the job is written to
//! a private directory under the temp dir — the container's tmpfs — analysed,
//! and the directory removed before the answer goes out.

pub mod self_test;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use apkaudio_contracts::baremetal_io;
use base64::Engine as _;
use oa_sample_analyzer::analyze;
use rumqttc::{Client, Event, MqttOptions, Packet, QoS};
use serde_json::{Value, json};

/// The token the bus knows this worker by. The same word as `[sample_analyzer]`.
pub const MODULE: &str = "sample_analyzer";

/// The tool the result came from, for a reader that listens to several.
pub const TOOL: &str = "PROCESSOR.SampleAnalyzer";

/// The compiled-in config, the same text the runner publishes at `/config`.
pub const CONFIG_INI: &str = include_str!("../config.ini");

/// Used when `config.ini` does not say.
const DEFAULT_MAX_JOB_BYTES: usize = 32 * 1024 * 1024;
/// The CLI's own `--max-len` default (`src/Core/args.rs`).
const DEFAULT_MAX_LEN_S: f64 = 600.0;

/// The limits a job runs under, read from `config.ini`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    pub max_job_bytes: usize,
    pub max_len_s: f64,
}

impl Limits {
    pub fn from_ini(config_ini: &str) -> Self {
        let ini = apk_plugin_runner::ini_to_json(config_ini);
        let key = |k: &str| ini.get(MODULE).and_then(|s| s.get(k)).and_then(Value::as_str).map(str::trim).map(str::to_string);
        Limits {
            max_job_bytes: key("max_job_bytes").and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_MAX_JOB_BYTES),
            max_len_s: key("max_len_s").and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_MAX_LEN_S),
        }
    }
}

/// A job's file name reduced to one safe path element: the name only, no
/// directory, nothing but letters, digits and `._- `. The extension survives,
/// because the decoder is chosen by it.
fn safe_name(filename: &str) -> String {
    let base = filename.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || "._- ".contains(c) { c } else { '_' })
        .collect();
    let cleaned = cleaned.trim_start_matches('.').to_string();
    if cleaned.is_empty() { "upload.wav".to_string() } else { cleaned }
}

/// A directory of this job's own, removed when dropped.
struct JobDir(PathBuf);

impl JobDir {
    fn new() -> std::io::Result<Self> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "apk-{MODULE}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir)?;
        Ok(JobDir(dir))
    }
}

impl Drop for JobDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Answer one job. Always returns a result document, never panics on input:
/// a malformed job gets `ok: false` and an `error`, with its `job_id` when it
/// had one, so the caller waiting on that id is not left waiting forever.
pub fn handle_job(payload: &[u8], limits: Limits) -> Value {
    let started = Instant::now();
    let job: Value = match serde_json::from_slice(payload) {
        Ok(v @ Value::Object(_)) => v,
        Ok(_) => return failure(Value::Null, "a job is a JSON object", started),
        Err(e) => return failure(Value::Null, &format!("the job is not JSON: {e}"), started),
    };
    let job_id = match job.get("job_id") {
        Some(Value::String(s)) if !s.is_empty() => Value::String(s.clone()),
        _ => return failure(Value::Null, "the job has no job_id", started),
    };
    let name = safe_name(job.get("filename").and_then(Value::as_str).unwrap_or(""));
    let folder = job.get("folder").and_then(Value::as_str).unwrap_or("").to_string();
    // A job may lower the full-analysis cap, never raise it past config.ini.
    let max_len = job
        .get("max_len")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v > 0.0)
        .map_or(limits.max_len_s, |v| v.min(limits.max_len_s));

    let bytes = match job.get("content_b64") {
        Some(Value::String(b)) => match base64::engine::general_purpose::STANDARD.decode(b.trim()) {
            Ok(bytes) => bytes,
            Err(e) => return failure(job_id, &format!("content_b64 does not decode: {e}"), started),
        },
        _ => return failure(job_id, "the job carries no content_b64 (audio is bytes, never text)", started),
    };
    if bytes.len() > limits.max_job_bytes {
        return failure(
            job_id,
            &format!("the audio is {} bytes; this worker takes at most {}", bytes.len(), limits.max_job_bytes),
            started,
        );
    }

    match analyse_bytes(&bytes, &name, &folder, max_len) {
        Ok(peak) => json!({
            "job_id": job_id,
            "tool": TOOL,
            "ok": true,
            "result": peak,
            "error": null,
            "elapsed_ms": started.elapsed().as_millis() as u64,
        }),
        Err(e) => failure(job_id, &e, started),
    }
}

/// The bytes through the analyzer, by way of a file of their own. The record
/// names the file as the job named it, not by its temporary path.
fn analyse_bytes(bytes: &[u8], name: &str, folder: &str, max_len: f64) -> Result<Value, String> {
    let dir = JobDir::new().map_err(|e| format!("no scratch directory for the job: {e}"))?;
    let path = dir.0.join(name);
    std::fs::write(&path, bytes).map_err(|e| format!("could not stage the audio: {e}"))?;
    let mut peak = analyze::analyze(&path, Path::new(&dir.0), max_len)
        .ok_or_else(|| format!("{name} did not decode as audio (WAV, AIFF, FLAC, MP3, OGG or M4A)"))?;
    peak.metadata.folder = folder.to_string();
    peak.metadata.path = if folder.is_empty() { name.to_string() } else { format!("{folder}/{name}") };
    serde_json::to_value(&peak).map_err(|e| format!("the record does not serialise: {e}"))
}

fn failure(job_id: Value, error: &str, started: Instant) -> Value {
    json!({
        "job_id": job_id,
        "tool": TOOL,
        "ok": false,
        "result": null,
        "error": error,
        "elapsed_ms": started.elapsed().as_millis() as u64,
    })
}

/// A one-second 440 Hz sine as a 16-bit mono WAV, built by hand so the self
/// test needs no file and no writer crate.
pub fn probe_wav() -> Vec<u8> {
    let sr: u32 = 44_100;
    let samples: Vec<i16> = (0..sr)
        .map(|n| {
            let t = n as f64 / sr as f64;
            let fade = (t * 20.0).min(1.0) * ((1.0 - t) * 20.0).min(1.0);
            ((2.0 * std::f64::consts::PI * 440.0 * t).sin() * fade * 16_000.0) as i16
        })
        .collect();
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sr.to_le_bytes());
    out.extend_from_slice(&(sr * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// The job loop. Blocking; the runner starts it on its own thread and exits
/// the process if it ever returns, so Docker restarts the container.
///
/// Its own session rather than the runner's: an analysis takes seconds and
/// must not hold up the config, the will or the self-test door.
pub fn run_job_agent(host: &str, port: u16) {
    let (job_topic, _) = baremetal_io::module_job(MODULE).expect("sample_analyzer is a legal module token");
    let (result_topic, retain) =
        baremetal_io::module_result(MODULE).expect("sample_analyzer is a legal module token");
    let limits = Limits::from_ini(CONFIG_INI);

    let client_id = format!("apk-worker-{MODULE}-{}", std::process::id());
    let mut options = MqttOptions::new(client_id, host, port);
    options.set_keep_alive(Duration::from_secs(30));
    // A job is base64 audio: a third larger than the file.
    let ceiling = limits.max_job_bytes / 3 * 4 + 64 * 1024;
    options.set_max_packet_size(ceiling, ceiling);
    if let Some((username, password)) = apkaudio_contracts::broker_auth::credentials() {
        options.set_credentials(username, password);
    }

    let (client, mut connection) = Client::new(options, 32);
    // The job runs on a thread of its own, never on this one: rumqttc only
    // pings while `connection` is polled, and a job that held this loop past
    // 1.5x the keep-alive had the broker drop the session mid-job
    // (LOOSE-worker-keepalive-long-job).
    let answers = client.clone();
    let result_leaf = result_topic.clone();
    let lane = JobLane::start(
        move |payload| handle_job(payload, limits),
        move |answer| {
            println!(
                "🧰 [{MODULE}] job {} → ok={} {} ({} ms)",
                answer["job_id"],
                answer["ok"],
                answer["result"]["metadata"]["name"],
                answer["elapsed_ms"]
            );
            // Blocking is fine here: the event loop drains the request queue.
            let _ = answers.publish(result_leaf.clone(), QoS::AtLeastOnce, retain, answer.to_string());
        },
    );
    println!("🧰 [{MODULE}] worker listening on {job_topic}, answering on {result_topic}");
    for event in connection.iter() {
        // Every event, ping responses included, is progress — and with the
        // job off this thread, pings keep coming however long the job runs.
        apk_plugin_runner::tick();
        match event {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                let _ = client.try_subscribe(job_topic.clone(), QoS::AtLeastOnce);
            }
            Ok(Event::Incoming(Packet::Publish(p))) if p.topic == job_topic => match lane.offer(p.payload.to_vec()) {
                Ok(()) => {}
                Err(Refused::Busy(answer)) => {
                    let _ = client.try_publish(result_topic.clone(), QoS::AtLeastOnce, retain, answer.to_string());
                }
                Err(Refused::Gone) => {
                    // The job thread died (a panic in a decoder): return, and
                    // the runner exits the process so Docker restarts it.
                    eprintln!("⚠️ [{MODULE}] the job thread is gone; exiting");
                    return;
                }
            },
            Ok(_) => {}
            Err(e) => {
                eprintln!("⚠️ [{MODULE}] broker session: {e}; retrying");
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }
}

/// Jobs that may wait behind the one running before a new one is refused.
pub const JOB_QUEUE: usize = 4;

/// Why [`JobLane::offer`] did not take a job.
#[derive(Debug)]
pub enum Refused {
    /// [`JOB_QUEUE`] jobs already wait: the answer to send now, under the
    /// job's own `job_id` when it had one, so the caller is not left waiting.
    Busy(Value),
    /// The job thread has ended.
    Gone,
}

/// One thread that runs jobs in turn, fed without ever blocking the feeder.
pub struct JobLane {
    jobs: std::sync::mpsc::SyncSender<Vec<u8>>,
}

impl JobLane {
    /// Start the job thread: each job goes through `run`, and its answer to
    /// `answer`, one at a time in arrival order.
    pub fn start(
        run: impl Fn(&[u8]) -> Value + Send + 'static,
        answer: impl Fn(Value) + Send + 'static,
    ) -> Self {
        let (jobs, queue) = std::sync::mpsc::sync_channel::<Vec<u8>>(JOB_QUEUE);
        std::thread::Builder::new()
            .name(format!("{MODULE}-job"))
            .spawn(move || {
                for payload in queue {
                    answer(run(&payload));
                }
            })
            .expect("the job thread starts");
        JobLane { jobs }
    }

    /// Hand a job to the job thread. Returns at once, whatever is running.
    pub fn offer(&self, payload: Vec<u8>) -> Result<(), Refused> {
        use std::sync::mpsc::TrySendError;
        match self.jobs.try_send(payload) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(payload)) => Err(Refused::Busy(busy(&payload))),
            Err(TrySendError::Disconnected(_)) => Err(Refused::Gone),
        }
    }
}

/// The answer to a job refused because the queue is full.
fn busy(payload: &[u8]) -> Value {
    let started = Instant::now();
    let job_id = serde_json::from_slice::<Value>(payload)
        .ok()
        .and_then(|job| job.get("job_id").filter(|id| id.as_str().is_some_and(|s| !s.is_empty())).cloned())
        .unwrap_or(Value::Null);
    failure(job_id, &format!("the worker is busy ({JOB_QUEUE} jobs already wait); send the job again later"), started)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(body: Value) -> Vec<u8> {
        body.to_string().into_bytes()
    }

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    const LIMITS: Limits = Limits { max_job_bytes: 1 << 22, max_len_s: 600.0 };

    #[test]
    fn a_wav_job_comes_back_as_a_peak_record_under_its_id() {
        let out = handle_job(
            &job(json!({"job_id": "a1", "filename": "tone.wav", "folder": "probe", "content_b64": b64(&probe_wav())})),
            LIMITS,
        );
        assert_eq!(out["job_id"], "a1");
        assert_eq!(out["ok"], true, "{}", out["error"]);
        let meta = &out["result"]["metadata"];
        assert_eq!(meta["name"], "tone.wav");
        assert_eq!(meta["path"], "probe/tone.wav", "the temp path never leaks into the record");
        assert_eq!(meta["folder"], "probe");
        assert_eq!(meta["sample_rate"], 44_100);
        assert_eq!(meta["source_format"], "WAV");
        assert_eq!(meta["analysis_depth"], "full");
        let pitch = out["result"]["musicality"]["pitch_hz"].as_f64().unwrap_or(0.0);
        assert!((pitch - 440.0).abs() < 5.0, "the analyzer hears the 440 Hz probe: {pitch}");
    }

    #[test]
    fn a_job_may_lower_the_cap_and_the_record_is_preview_only() {
        let out = handle_job(
            &job(json!({"job_id": "a2", "filename": "tone.wav", "max_len": 0.5, "content_b64": b64(&probe_wav())})),
            LIMITS,
        );
        assert_eq!(out["ok"], true, "{}", out["error"]);
        assert_eq!(out["result"]["metadata"]["analysis_depth"], "preview_only");
    }

    /// Counted by its own directory, not by listing the temp dir: the tests
    /// run in parallel and each stages a file of its own.
    #[test]
    fn the_job_dir_is_gone_after_the_job() {
        let dir = JobDir::new().unwrap();
        let path = dir.0.clone();
        std::fs::write(path.join("x.wav"), probe_wav()).unwrap();
        assert!(path.exists());
        drop(dir);
        assert!(!path.exists(), "{} survived its job", path.display());
    }

    #[test]
    fn a_name_cannot_walk_out_of_the_job_dir() {
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_name("C:\\x\\kick 01.wav"), "kick 01.wav");
        assert_eq!(safe_name("..wav"), "wav");
        assert_eq!(safe_name(""), "upload.wav");
        assert_eq!(safe_name("a/b$c.flac"), "b_c.flac");
    }

    #[test]
    fn a_bad_job_is_answered_not_dropped() {
        let out = handle_job(b"not json", LIMITS);
        assert_eq!(out["ok"], false);
        assert!(out["job_id"].is_null());

        let out = handle_job(&job(json!({"content_b64": b64(&probe_wav())})), LIMITS);
        assert_eq!(out["ok"], false, "no job_id");

        let out = handle_job(&job(json!({"job_id": "b1", "text": "RIFF"})), LIMITS);
        assert_eq!((out["job_id"].as_str(), out["ok"].as_bool()), (Some("b1"), Some(false)), "text is not audio");

        let out = handle_job(&job(json!({"job_id": "b2", "content_b64": "%%%"})), LIMITS);
        assert_eq!(out["ok"], false, "bad base64");

        let out = handle_job(&job(json!({"job_id": "b3", "filename": "x.wav", "content_b64": b64(b"not audio at all")})), LIMITS);
        assert_eq!(out["ok"], false, "undecodable audio");
        assert!(out["error"].as_str().unwrap().contains("did not decode"));
    }

    #[test]
    fn an_oversized_job_is_refused_by_size() {
        let small = Limits { max_job_bytes: 100, max_len_s: 600.0 };
        let out = handle_job(&job(json!({"job_id": "c1", "content_b64": b64(&probe_wav())})), small);
        assert_eq!(out["ok"], false);
        assert!(out["error"].as_str().unwrap().contains("at most 100"));
    }

    #[test]
    fn the_limits_come_from_config_ini() {
        assert_eq!(Limits::from_ini(CONFIG_INI), Limits { max_job_bytes: 33_554_432, max_len_s: 600.0 });
        assert_eq!(
            Limits::from_ini("[sample_analyzer]\n"),
            Limits { max_job_bytes: DEFAULT_MAX_JOB_BYTES, max_len_s: DEFAULT_MAX_LEN_S }
        );
    }

    #[test]
    fn job_and_result_topics_are_this_modules() {
        let (job, retain) = baremetal_io::module_job(MODULE).unwrap();
        assert_eq!(job, "APK.audio/System/Protocols/sample_analyzer/outgoing/job");
        assert!(!retain);
        let (result, retain) = baremetal_io::module_result(MODULE).unwrap();
        assert_eq!(result, "APK.audio/System/Protocols/sample_analyzer/incoming/result");
        assert!(!retain);
    }

    #[test]
    fn a_long_job_never_holds_up_the_feeder() {
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let (running_tx, running_rx) = std::sync::mpsc::channel();
        let lane = JobLane::start(
            move |payload| {
                let _ = running_tx.send(());
                std::thread::sleep(Duration::from_millis(400));
                json!({ "job_id": String::from_utf8_lossy(payload) })
            },
            move |answer| done_tx.send(answer).unwrap(),
        );
        lane.offer(b"j0".to_vec()).expect("queued");
        running_rx.recv_timeout(Duration::from_secs(5)).expect("the job started");
        let t = Instant::now();
        // One running, JOB_QUEUE waiting, one more refused — all at once.
        for i in 1..=JOB_QUEUE {
            lane.offer(format!("j{i}").into_bytes()).expect("queued");
        }
        let refused = lane.offer(br#"{"job_id":"late"}"#.to_vec());
        assert!(t.elapsed() < Duration::from_millis(200), "offer waited on the job: {:?}", t.elapsed());
        match refused {
            Err(Refused::Busy(a)) => {
                assert_eq!(a["job_id"], "late");
                assert_eq!(a["ok"], false);
            }
            other => panic!("expected busy, got {other:?}"),
        }
        assert_eq!(done_rx.recv_timeout(Duration::from_secs(5)).unwrap()["job_id"], "j0");
    }

    #[test]
    fn a_dead_job_thread_is_reported_not_hidden() {
        let lane = JobLane::start(|_| panic!("decoder blew up"), |_| {});
        lane.offer(b"x".to_vec()).expect("first is queued");
        let t = Instant::now();
        loop {
            if let Err(Refused::Gone) = lane.offer(b"x".to_vec()) {
                break;
            }
            assert!(t.elapsed() < Duration::from_secs(5), "never saw the thread go");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
