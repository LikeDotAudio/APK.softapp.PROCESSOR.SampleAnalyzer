//! Command-line configuration.
use std::path::PathBuf;

pub struct Config {
    pub root: PathBuf,
    pub out: PathBuf,
    pub workers: usize,
    pub max_len: f64,
    pub clusters: usize,
    pub per_file: bool,
    pub force: bool, // re-analyze even when a sidecar's analyzer version matches
    pub stride: usize, // only process every Nth file
    pub db_url: Option<String>,
}

impl Config {
    /// Parse the full argv (including argv[0]). Returns None when no directory
    /// argument was supplied (caller prints usage and exits).
    pub fn parse(args: Vec<String>) -> Option<Config> {
        if args.len() < 2 {
            return None;
        }
        let root = PathBuf::from(&args[1]);
        let mut out: Option<PathBuf> = None;
        let mut workers = 30usize;
        // 10 s was a drum-sample-pack default: it silently discarded every SFX,
        // ambience and weather bed, which run to minutes. The signature spec calls
        // this out explicitly — UCS work needs the long files.
        let mut max_len = 600.0f64;
        let mut clusters = 8usize;
        let mut per_file = true;
        let mut force = false;
        let mut stride = 1usize;
        let mut db_url = None;
        let mut i = 2;
        while i < args.len() {
            match args[i].as_str() {
                "--out" => { out = args.get(i + 1).map(PathBuf::from); i += 2; }
                "--no-per-file" => { per_file = false; i += 1; }
                "--force" => { force = true; i += 1; }
                "--stride" => { stride = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(1).max(1); i += 2; }
                "--workers" => { workers = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(30); i += 2; }
                "--max-len" => { max_len = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(600.0); i += 2; }
                "--clusters" => { clusters = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(8); i += 2; }
                "--db-url" => { db_url = args.get(i + 1).cloned(); i += 2; }
                _ => { i += 1; }
            }
        }
        let out = out.unwrap_or_else(|| root.join("sample_cloud_data.PEAK"));
        Some(Config { root, out, workers, max_len, clusters, per_file, force, stride, db_url })
    }
}
