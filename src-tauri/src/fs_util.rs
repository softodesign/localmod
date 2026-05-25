use std::fs;
use std::io;
use std::path::Path;

/// Sum file sizes under `path` (recursive). Skips unreadable entries.
pub fn dir_size(path: &Path) -> io::Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    if path.is_file() {
        return Ok(fs::metadata(path)?.len());
    }
    let mut sum = 0u64;
    if path.is_dir() {
        for entry in walkdir::WalkDir::new(path)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() {
                if let Ok(m) = entry.metadata() {
                    sum += m.len();
                }
            }
        }
    }
    Ok(sum)
}
