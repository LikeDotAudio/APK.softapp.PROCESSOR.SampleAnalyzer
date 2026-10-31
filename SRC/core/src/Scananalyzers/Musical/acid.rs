use std::path::Path;

/// Read the ACID chunk (embedded loop metadata) if present: (bpm, root_note).
/// bpm = 0 and root_note = -1 when absent. Walks the RIFF chunk list without
/// loading the whole file.
pub fn read_acid(path: &Path) -> (f64, i32) {
    use std::io::{Read, Seek, SeekFrom};
    let none = (0.0, -1);
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return none,
    };
    let mut hdr = [0u8; 12];
    if f.read_exact(&mut hdr).is_err() {
        return none;
    }
    if &hdr[0..4] != b"RIFF" || &hdr[8..12] != b"WAVE" {
        return none;
    }
    loop {
        let mut ch = [0u8; 8];
        if f.read_exact(&mut ch).is_err() {
            break;
        }
        let size = u32::from_le_bytes([ch[4], ch[5], ch[6], ch[7]]) as u64;
        if &ch[0..4] == b"acid" {
            let mut buf = vec![0u8; size.min(64) as usize];
            if f.read_exact(&mut buf).is_err() || buf.len() < 24 {
                break;
            }
            let flags = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
            let root = u16::from_le_bytes([buf[4], buf[5]]) as i32;
            let tempo = f32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]]) as f64;
            let root_out = if flags & 0x2 != 0 { root } else { -1 };
            let bpm = if tempo.is_finite() && tempo > 0.0 && tempo < 400.0 { tempo } else { 0.0 };
            return (bpm, root_out);
        }
        // Skip this chunk's data (chunks are word-aligned).
        let skip = size + (size & 1);
        if f.seek(SeekFrom::Current(skip as i64)).is_err() {
            break;
        }
    }
    none
}

/// Same as `read_acid`, but over an in-memory buffer (used by the WASM/web
/// path, which only has the file's bytes). Walks the RIFF chunk list.
pub fn read_acid_buffer(buf: &[u8]) -> (f64, i32) {
    let none = (0.0, -1);
    if buf.len() < 12 || &buf[0..4] != b"RIFF" || &buf[8..12] != b"WAVE" {
        return none;
    }
    let mut pos = 12usize;
    while pos + 8 <= buf.len() {
        let id = &buf[pos..pos + 4];
        let size = u32::from_le_bytes([buf[pos + 4], buf[pos + 5], buf[pos + 6], buf[pos + 7]]) as usize;
        let data_start = pos + 8;
        if id == b"acid" {
            let end = (data_start + size.min(64)).min(buf.len());
            let chunk = &buf[data_start..end];
            if chunk.len() < 24 {
                return none;
            }
            let flags = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            let root = u16::from_le_bytes([chunk[4], chunk[5]]) as i32;
            let tempo = f32::from_le_bytes([chunk[20], chunk[21], chunk[22], chunk[23]]) as f64;
            let root_out = if flags & 0x2 != 0 { root } else { -1 };
            let bpm = if tempo.is_finite() && tempo > 0.0 && tempo < 400.0 { tempo } else { 0.0 };
            return (bpm, root_out);
        }
        let skip = size + (size & 1);
        pos = data_start + skip;
    }
    none
}
