//! Optional redirect of all app data to a custom folder via `data_root.json`
//! in the OS app-data directory (see `resolve_work_dir`).

use serde_json::Value;
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "data_root.json";

pub fn read_override(canonical_app_data: &Path) -> Option<PathBuf> {
    let p = canonical_app_data.join(FILE_NAME);
    let s = std::fs::read_to_string(&p).ok()?;
    let v: Value = serde_json::from_str(&s).ok()?;
    let path = v.get("dataRoot")?.as_str()?.trim();
    if path.is_empty() {
        return None;
    }
    Some(PathBuf::from(path))
}

/// Application work directory: either `canonical_app_data` or the path from `data_root.json`.
///
/// If the user pointed at a **new empty folder**, we would open an empty `localmod.sqlite` there and
/// models / chats / reference look “gone” while the real DB still lives under the OS app-data path.
/// When the custom folder has **no** database but the default folder **does**, we ignore the override
/// and keep using the default (see `data_root.json` note in Storage settings).
pub fn resolve_work_dir(canonical_app_data: &Path) -> Result<PathBuf, String> {
    if let Some(pb) = read_override(canonical_app_data) {
        if pb.as_os_str().is_empty() {
            return Ok(canonical_app_data.to_path_buf());
        }
        std::fs::create_dir_all(&pb).map_err(|e| e.to_string())?;
        let custom_db = pb.join("localmod.sqlite");
        let default_db = canonical_app_data.join("localmod.sqlite");
        if !custom_db.exists() && default_db.exists() {
            eprintln!(
                "[LocalMOD] data_root.json points to {} which has no database; \
using default app data at {} instead. Remove or fix {} if you really want a new location (copy localmod.sqlite and folders first).",
                pb.display(),
                canonical_app_data.display(),
                canonical_app_data.join(FILE_NAME).display()
            );
            return Ok(canonical_app_data.to_path_buf());
        }
        return Ok(pb);
    }
    Ok(canonical_app_data.to_path_buf())
}

pub fn write_override(canonical_app_data: &Path, target: Option<&Path>) -> Result<(), String> {
    let p = canonical_app_data.join(FILE_NAME);
    match target {
        None => {
            let _ = std::fs::remove_file(&p);
            Ok(())
        }
        Some(t) if t.as_os_str().is_empty() => {
            let _ = std::fs::remove_file(&p);
            Ok(())
        }
        Some(t) => {
            let v = serde_json::json!({ "dataRoot": t.to_string_lossy() });
            std::fs::write(
                &p,
                serde_json::to_vec_pretty(&v).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        }
    }
}

/// Path to show in Storage settings: matches `resolve_work_dir` rules (no “phantom” custom path).
pub fn configured_display(canonical_app_data: &Path, effective: &Path) -> PathBuf {
    match read_override(canonical_app_data) {
        Some(pb) if !pb.as_os_str().is_empty() => {
            let custom_db = pb.join("localmod.sqlite");
            let default_db = canonical_app_data.join("localmod.sqlite");
            if !custom_db.exists() && default_db.exists() {
                effective.to_path_buf()
            } else {
                pb
            }
        }
        _ => effective.to_path_buf(),
    }
}
