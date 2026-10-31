use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::decode::is_audio;

/// What the walk found, so a run can report what it ignored instead of
/// silently dropping it. A library that is 94% MP3 must not look empty.
pub struct Discovery {
    pub files: Vec<PathBuf>,
    /// extension (lower-case) -> count, for every non-audio file seen
    pub skipped: BTreeMap<String, usize>,
}

/// Recursively find every decodable audio file under `root`.
pub fn discover_audio(root: &Path) -> Discovery {
    let mut files = Vec::new();
    let mut skipped: BTreeMap<String, usize> = BTreeMap::new();

    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.into_path();
        if is_audio(&path) {
            files.push(path);
            continue;
        }
        let ext = path
            .extension()
            .and_then(|x| x.to_str())
            .map(|x| x.to_ascii_lowercase())
            .unwrap_or_else(|| "(none)".to_string());
        // Our own sidecars are not "skipped files" in any interesting sense.
        if ext != "peak" {
            *skipped.entry(ext).or_insert(0) += 1;
        }
    }

    files.sort();
    Discovery { files, skipped }
}
