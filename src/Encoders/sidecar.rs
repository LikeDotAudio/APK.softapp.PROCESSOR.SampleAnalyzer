use std::path::Path;

use crate::peak::Peak;
use crate::version::ANALYZER_VERSION;

/// Write a per-file `<sample>.PEAK` sidecar next to the audio file. Returns true
/// on success. Used both for the incremental write during analysis and the
/// rewrite afterwards (once the cluster id is known).
pub fn write_sidecar(p: &Peak) -> bool {
    let peak_path = Path::new(&p.metadata.path).with_extension("PEAK");
    serde_json::to_string_pretty(p)
        .ok()
        .and_then(|js| std::fs::write(&peak_path, js).ok())
        .is_some()
}

/// Try to reuse the existing sidecar instead of re-analyzing: valid only when
/// it parses as a full record AND its `analyzer` version matches this binary
/// (same crate version + same source hash ⇒ identical results by definition)
/// AND it describes this same file name. Returns the record with its
/// path/folder refreshed to the file's current location, cluster reset.
pub fn read_sidecar(wav: &Path, root: &Path) -> Option<Peak> {
    let peak_path = wav.with_extension("PEAK");
    let txt = std::fs::read_to_string(&peak_path).ok()?;
    let mut p: Peak = serde_json::from_str(&txt).ok()?;
    if p.metadata.analyzer_version != ANALYZER_VERSION {
        return None; // produced by different extractor code — re-analyze
    }
    let name = wav.file_name().and_then(|x| x.to_str()).unwrap_or("");
    if p.metadata.name != name || name.is_empty() {
        return None; // sidecar describes some other file — re-analyze
    }
    // Refresh location fields (the file may have been moved with its sidecar).
    p.metadata.path = wav.to_string_lossy().to_string();
    let folder = wav
        .parent()
        .and_then(|par| par.strip_prefix(root).ok())
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    p.metadata.folder = folder.clone();
    p.metadata.sub = folder;
    p.unsupervised.cluster = -1; // clustering is global and re-run every time
    Some(p)
}
