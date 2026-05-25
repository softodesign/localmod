//! Auto-detect multimodal projector (mmproj) files next to the main GGUF.
//! Consumer builds avoid exposing paths in Settings: keep both files in the same folder.

use std::path::{Path, PathBuf};

fn common_prefix_char_len(a: &str, b: &str) -> usize {
    a.chars()
        .zip(b.chars())
        .take_while(|(x, y)| x == y)
        .count()
}

/// Finds `*.gguf` in the same directory as `main_gguf` whose name contains `mmproj`.
/// If several match, prefers the stem with the longest common prefix with the main file (HF layout).
pub fn auto_discover_mmproj(main_gguf: &Path) -> Option<PathBuf> {
    let dir = main_gguf.parent()?;
    let main_stem = main_gguf
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let rd = std::fs::read_dir(dir).ok()?;
    let mut found: Vec<PathBuf> = Vec::new();
    for ent in rd.flatten() {
        let p = ent.path();
        if !p.is_file() {
            continue;
        }
        if p == main_gguf {
            continue;
        }
        let name = ent.file_name().to_string_lossy().to_lowercase();
        if !name.ends_with(".gguf") {
            continue;
        }
        if name.contains("mmproj") {
            found.push(p);
        }
    }
    if found.is_empty() {
        return None;
    }
    if found.len() == 1 {
        return found.pop();
    }
    found.sort_by(|a, b| {
        let sa = a
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let sb = b
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let la = common_prefix_char_len(&main_stem, &sa);
        let lb = common_prefix_char_len(&main_stem, &sb);
        lb.cmp(&la)
    });
    found.into_iter().next()
}
