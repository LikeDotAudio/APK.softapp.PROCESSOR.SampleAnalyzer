//! Orchestrate a full run: discover files, analyze them in parallel while
//! streaming progress, cluster, then write the sidecars + aggregate PEAK.
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use rayon::prelude::*;

use crate::analyze::analyze;
use crate::args::Config;
use crate::cluster::cluster_samples;
#[cfg(not(target_arch = "wasm32"))]
use crate::db;
use crate::discover::discover_audio;
use crate::emit::emit;
use crate::pca::pca_assign;
use crate::peak::Peak;
use crate::sidecar::{read_sidecar, write_sidecar};
use crate::stream::{emit_result, emit_skip};
use crate::version::ANALYZER_VERSION;

pub fn run(cfg: &Config) {
    #[cfg(not(target_arch = "wasm32"))]
    let db_pool = cfg.db_url.as_ref().and_then(|url| {
        db::get_pool(url).map_err(|e| eprintln!("DB Connect Error: {}", e)).ok().and_then(|pool| {
            if db::init_db(&pool).is_ok() {
                Some(pool)
            } else {
                eprintln!("Warning: Failed to initialize MySQL DB. DB export will be skipped.");
                None
            }
        })
    });

    let found = discover_audio(&cfg.root);
    let files: Vec<_> = if cfg.stride > 1 {
        found.files.into_iter().step_by(cfg.stride).collect()
    } else {
        found.files
    };
    let total = files.len();
    // Report what we ignored. A silent drop is how a 94%-MP3 library came to
    // look like a directory the walker had failed to recurse into.
    emit(&serde_json::json!({ "type": "start", "total": total, "workers": cfg.workers,
                              "skipped": found.skipped,
                              "analyzer_version": ANALYZER_VERSION }));
    if total == 0 {
        emit(&serde_json::json!({ "type": "done", "count": 0, "out": cfg.out.to_string_lossy() }));
        return;
    }

    let done = AtomicUsize::new(0);
    let wrote = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);
    let reused = AtomicUsize::new(0);
    let stdout_lock = Mutex::new(());
    let pool = rayon::ThreadPoolBuilder::new().num_threads(cfg.workers.max(1)).build().unwrap();

    let mut results: Vec<Peak> = pool.install(|| {
        files
            .par_iter()
            // Force one file per Rayon job. Per-file analysis cost is wildly
            // uneven (a 30 s ambience takes far longer than a one-shot), and the
            // default adaptive splitter hands out contiguous sub-ranges that can
            // only be rebalanced while still un-started. Near the end that left a
            // single thread grinding the last heavy leaf alone while the rest sat
            // idle. At max_len(1) every remaining file stays individually
            // stealable, so all workers stay busy right down to the last file;
            // the extra split overhead is nothing next to a DSP pass.
            .with_max_len(1)
            .filter_map(|f| {
                // Reuse the existing sidecar when its analyzer version matches
                // this binary — same extractor code yields the same results,
                // so the DSP is skipped entirely (`--force` overrides).
                let tid = rayon::current_thread_index().unwrap_or(0);
                let fname = f.file_name().and_then(|x| x.to_str()).unwrap_or("");
                {
                    let _g = stdout_lock.lock().unwrap();
                    crate::emit::emit(&serde_json::json!({
                        "type": "thread_start",
                        "thread_id": tid,
                        "file": fname
                    }));
                }
                
                let cached = if cfg.force { None } else { read_sidecar(f, &cfg.root) };
                let was_cached = cached.is_some();
                // Catch any per-file panic so one bad sample can't abort the run.
                let res = cached.or_else(|| {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                        || analyze(f, &cfg.root, cfg.max_len),
                    ))
                    .unwrap_or(None)
                });
                // Write the per-file sidecar immediately, so it appears during
                // the run and survives an interrupted/killed process. (Cached
                // records are rewritten later anyway, with the new cluster id.)
                if was_cached {
                    reused.fetch_add(1, Ordering::Relaxed);
                } else if cfg.per_file {
                    if let Some(p) = &res {
                        if write_sidecar(p) {
                            wrote.fetch_add(1, Ordering::Relaxed);
                        } else {
                            failed.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                // Stream progress / result (serialized to avoid interleaving).
                let _g = stdout_lock.lock().unwrap();
                match &res {
                    Some(p) => emit_result(p, n, total, tid),
                    None => emit_skip(f.file_name().and_then(|x| x.to_str()).unwrap_or(""), n, total, tid),
                }
                res
            })
            .collect()
    });

    // ---- Blind K-Means grouping over the extracted feature space.
    cluster_samples(&mut results, cfg.clusters);
    let mut cluster_counts = std::collections::BTreeMap::new();
    for p in &results {
        *cluster_counts.entry(p.unsupervised.cluster).or_insert(0usize) += 1;
    }
    emit(&serde_json::json!({ "type": "clusters", "k": cfg.clusters, "counts": cluster_counts }));

    // ---- PCA embedding: 3 principal components per sample for a 2D/3D map.
    pca_assign(&mut results);

    // Sidecars were written incrementally during analysis (above). Rewrite each
    // now so the final cluster id + PCA coordinates are included too.
    if cfg.per_file {
        results.par_iter().for_each(|p| {
            write_sidecar(p);
        });
        emit(&serde_json::json!({
            "type": "per_file", "wrote": wrote.load(Ordering::Relaxed),
            "failed": failed.load(Ordering::Relaxed)
        }));
    }

    // Aggregate PEAK (used by the cloud / Groups / Examiner views).
    if let Ok(json) = serde_json::to_string(&results) {
        if let Ok(mut fh) = std::fs::File::create(&cfg.out) {
            let _ = fh.write_all(json.as_bytes());
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    if let Some(pool) = &db_pool {
        // Report what the server STORED, not how many records were handed to it.
        // The old form emitted results.len() on any 200, so a batch the endpoint
        // silently dropped still showed up as a complete success.
        match db::write_peaks(pool, &results) {
            Err(e) => {
                eprintln!("Warning: DB write failed: {}", e);
                emit(&serde_json::json!({ "type": "db_export", "status": "failed", "error": e }));
            }
            Ok(stored) => {
                if stored < results.len() {
                    eprintln!(
                        "Warning: DB stored {} of {} records",
                        stored,
                        results.len()
                    );
                }
                emit(&serde_json::json!({
                    "type": "db_export", "status": "success",
                    "count": stored, "sent": results.len()
                }));
            }
        }
    }

    // Slim manifest: a compact index over the full sidecars so the UI can load a huge
    // library fast and lazy-load full records on demand. Additive and best-effort — the
    // per-file .PEAK sidecars remain canonical, so a failure here never fails the scan.
    let slim_src: Vec<serde_json::Value> =
        results.iter().filter_map(|p| serde_json::to_value(p).ok()).collect();
    if crate::manifest::write_manifest(&cfg.root, &slim_src) {
        emit(&serde_json::json!({ "type": "manifest", "count": slim_src.len(),
            "path": cfg.root.join(crate::manifest::MANIFEST_NAME).to_string_lossy() }));
    }
    emit(&serde_json::json!({ "type": "done", "count": results.len(), "out": cfg.out.to_string_lossy(),
                              "per_file": cfg.per_file, "reused": reused.load(Ordering::Relaxed),
                              "analyzer_version": ANALYZER_VERSION }));
}
