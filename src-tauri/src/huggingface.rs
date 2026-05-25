use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

#[derive(Deserialize, Debug)]
pub struct HfTreeEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub path: String,
    pub size: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HfWeightKind {
    Gguf,
    Safetensors,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HfWeightFile {
    pub path: String,
    pub size: Option<i64>,
    pub kind: HfWeightKind,
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HfGgufFile {
    pub path: String,
    pub size: Option<i64>,
}

#[derive(Clone, Debug)]
pub enum AutoDownloadPlan {
    Single {
        repo_path: String,
        kind: HfWeightKind,
    },
    ShardedSafetensors {
        repo_paths: Vec<String>,
    },
}

/// Token for Hugging Face downloads: set by the **maintainer** via environment (or `.env` in dev).
/// Not stored in the app database and not shown in Settings. `HUGGINGFACE_TOKEN` or `HF_TOKEN`.
pub fn resolve_hf_token() -> Option<String> {
    std::env::var("HUGGINGFACE_TOKEN")
        .or_else(|_| std::env::var("HF_TOKEN"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Normalize user input: `org/model`, full URL, or URL with /tree/...
pub fn normalize_repo_id(input: &str) -> Result<String, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("Enter a model repo ID or a Hugging Face link.".into());
    }
    if let Some(rest) = s.strip_prefix("https://huggingface.co/") {
        return normalize_repo_id_from_path(rest);
    }
    if let Some(rest) = s.strip_prefix("http://huggingface.co/") {
        return normalize_repo_id_from_path(rest);
    }
    Ok(s.trim_matches('/').to_string())
}

fn normalize_repo_id_from_path(path: &str) -> Result<String, String> {
    let path = path.split('?').next().unwrap_or(path).trim_end_matches('/');
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 {
        return Err(
            "Could not read org/name from that link. Example: bartowski/Qwen2.5-7B-Instruct-GGUF"
                .into(),
        );
    }
    let mut end = parts.len();
    for (i, p) in parts.iter().enumerate() {
        if *p == "tree" || *p == "blob" || *p == "resolve" || *p == "commits" {
            end = i;
            break;
        }
    }
    if end < 2 {
        return Err("Invalid Hugging Face URL.".into());
    }
    Ok(format!("{}/{}", parts[0], parts[1]))
}

pub fn hf_client(token: Option<String>) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .user_agent("LocalMOD/0.1 (Local AI)")
        .timeout(std::time::Duration::from_secs(7200))
        .connect_timeout(std::time::Duration::from_secs(60));
    if let Some(t) = token {
        let t = t.trim();
        if !t.is_empty() {
            let mut headers = reqwest::header::HeaderMap::new();
            let hv = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", t))
                .map_err(|_| "Invalid Hugging Face token format.".to_string())?;
            headers.insert(reqwest::header::AUTHORIZATION, hv);
            builder = builder.default_headers(headers);
        }
    }
    builder.build().map_err(|e| e.to_string())
}

async fn fetch_tree_entries(
    client: &reqwest::Client,
    repo_id: &str,
    revision: &str,
) -> Result<Vec<HfTreeEntry>, String> {
    let url = format!(
        "https://huggingface.co/api/models/{}/tree/{}?recursive=1",
        repo_id, revision
    );
    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "Hugging Face listing failed ({}): {}",
            status,
            body.chars().take(280).collect::<String>()
        ));
    }
    resp.json().await.map_err(|e| e.to_string())
}

pub async fn list_weights_for_rev(
    client: &reqwest::Client,
    repo_id: &str,
    revision: &str,
) -> Result<(Vec<HfWeightFile>, String), String> {
    let entries = fetch_tree_entries(client, repo_id, revision).await?;
    let mut out: Vec<HfWeightFile> = entries
        .into_iter()
        .filter(|e| e.entry_type == "file")
        .filter_map(|e| {
            let lower = e.path.to_ascii_lowercase();
            let kind = if lower.ends_with(".gguf") {
                HfWeightKind::Gguf
            } else if lower.ends_with(".safetensors") {
                HfWeightKind::Safetensors
            } else {
                return None;
            };
            Some(HfWeightFile {
                path: e.path,
                size: e.size,
                kind,
            })
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((out, revision.to_string()))
}

fn resolve_download_url(repo_id: &str, revision: &str, file_path: &str) -> Result<String, String> {
    let enc_path = file_path
        .trim_start_matches('/')
        .split('/')
        .map(|seg| urlencoding::encode(seg).into_owned())
        .collect::<Vec<_>>()
        .join("/");
    Ok(format!(
        "https://huggingface.co/{}/resolve/{}/{}",
        repo_id.trim_matches('/'),
        revision.trim_matches('/'),
        enc_path
    ))
}

/// Pause / cancel toggles checked between response chunks (same HTTP request).
#[derive(Clone)]
pub struct HfDownloadControl {
    pub pause: Arc<AtomicBool>,
    pub cancel: Arc<AtomicBool>,
}

impl HfDownloadControl {
    pub fn new() -> Self {
        Self {
            pause: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

async fn wait_until_unpaused_or_cancelled<P>(pause_cancel: &mut P) -> Result<(), String>
where
    P: FnMut() -> (bool, bool),
{
    loop {
        let (paused, cancelled) = pause_cancel();
        if cancelled {
            return Err("Cancelled.".into());
        }
        if !paused {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    }
}

pub async fn download_hf_file<F, PF>(
    client: &reqwest::Client,
    repo_id: &str,
    revision: &str,
    file_path: &str,
    dest: &Path,
    pause_cancel: &mut PF,
    mut on_progress: F,
) -> Result<u64, String>
where
    F: FnMut(String, u64, Option<u64>) + Send,
    PF: FnMut() -> (bool, bool) + Send,
{
    let url = resolve_download_url(repo_id, revision, file_path)?;
    wait_until_unpaused_or_cancelled(pause_cancel).await?;
    on_progress(
        format!(
            "Requesting {}",
            url.split('?').next().unwrap_or(&url)
        ),
        0,
        None,
    );

    wait_until_unpaused_or_cancelled(pause_cancel).await?;
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "Download failed ({}): {}",
            status,
            body.chars().take(200).collect::<String>()
        ));
    }

    let total = resp.content_length().unwrap_or(0);
    let mut stream = resp.bytes_stream();

    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("Could not create file {}: {e}", dest.display()))?;

    let mut downloaded: u64 = 0;
    let mut last_report = 0u64;

    loop {
        wait_until_unpaused_or_cancelled(pause_cancel).await?;
        let item = match stream.next().await {
            Some(i) => i,
            None => break,
        };
        wait_until_unpaused_or_cancelled(pause_cancel).await?;
        let chunk = item.map_err(|e| format!("Download stream error: {e}"))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Could not write file: {e}"))?;
        downloaded += chunk.len() as u64;

        if total > 0 {
            let pct = (downloaded as f64 / total as f64 * 100.0).min(100.0);
            if downloaded - last_report >= 10 * 1024 * 1024 || downloaded == total {
                last_report = downloaded;
                on_progress(
                    format!(
                        "Downloading… {:.0}% · {} / {} MB",
                        pct,
                        downloaded / 1024 / 1024,
                        total / 1024 / 1024
                    ),
                    downloaded,
                    Some(total),
                );
            }
        } else if downloaded - last_report >= 25 * 1024 * 1024 {
            last_report = downloaded;
            on_progress(
                format!("Downloading… {} MB", downloaded / 1024 / 1024),
                downloaded,
                None,
            );
        }
    }

    file.flush()
        .await
        .map_err(|e| format!("Could not finish file: {e}"))?;
    let bt = if total > 0 { Some(total) } else { None };
    on_progress("Download complete.".into(), downloaded, bt);
    Ok(downloaded)
}

#[derive(Deserialize)]
struct HfModelMeta {
    #[serde(default)]
    sha: Option<String>,
}

/// Hugging Face `GET /api/models/{repo}` includes the default-branch commit as `sha`.
pub async fn resolve_default_revision(
    client: &reqwest::Client,
    repo_id: &str,
) -> Result<String, String> {
    let url = format!("https://huggingface.co/api/models/{}", repo_id);
    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Ok("main".to_string());
    }
    let meta: HfModelMeta = match resp.json().await {
        Ok(m) => m,
        Err(_) => HfModelMeta { sha: None },
    };
    Ok(meta
        .sha
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "main".to_string()))
}

pub async fn list_weights_resolved(
    client: &reqwest::Client,
    repo_id: &str,
    revision_override: Option<&str>,
) -> Result<(Vec<HfWeightFile>, String), String> {
    if let Some(r) = revision_override.filter(|s| !s.is_empty()) {
        return list_weights_for_rev(client, repo_id, r).await;
    }

    let rev_preferred = resolve_default_revision(client, repo_id).await?;
    let (files_pref, rev) = list_weights_for_rev(client, repo_id, &rev_preferred).await?;
    if !files_pref.is_empty() {
        return Ok((files_pref, rev));
    }

    if rev_preferred != "main" {
        return list_weights_for_rev(client, repo_id, "main").await;
    }

    Err("No .gguf or .safetensors files in this repo.".into())
}

/// Pick a reasonable default GGUF when the user does not choose (balanced quant first).
pub fn pick_recommended_gguf(files: &[HfGgufFile]) -> Option<&HfGgufFile> {
    if files.is_empty() {
        return None;
    }
    const PREFS: &[&str] = &[
        "Q4_K_M", "Q4_K_S", "IQ4_XS", "Q5_K_M", "Q5_K_S", "Q4_0", "Q5_0", "Q8_0", "F16",
    ];
    let upper_paths: Vec<String> = files.iter().map(|f| f.path.to_uppercase()).collect();
    for p in PREFS {
        let up = p.to_uppercase();
        if let Some(idx) = upper_paths.iter().position(|path| path.contains(&up)) {
            return files.get(idx);
        }
    }
    files.first()
}

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
struct ShardKey {
    prefix: String,
    total: u32,
}

fn parse_shard_leaf(leaf: &str) -> Option<(ShardKey, u32)> {
    let name = leaf.strip_suffix(".safetensors")?;
    let (left, total_s) = name.rsplit_once("-of-")?;
    let total: u32 = total_s.parse().ok()?;
    let (prefix, idx_s) = left.rsplit_once('-')?;
    let idx: u32 = idx_s.parse().ok()?;
    if total == 0 || idx == 0 || idx > total {
        return None;
    }
    Some((ShardKey {
        prefix: prefix.to_string(),
        total,
    }, idx))
}

fn shard_groups_from_files(files: &[HfWeightFile]) -> HashMap<ShardKey, HashMap<u32, String>> {
    let mut m: HashMap<ShardKey, HashMap<u32, String>> = HashMap::new();
    for f in files {
        if HfWeightKind::Safetensors != f.kind {
            continue;
        }
        let Some(leaf) = Path::new(&f.path).file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some((key, idx)) = parse_shard_leaf(leaf) else {
            continue;
        };
        m.entry(key).or_default().insert(idx, f.path.clone());
    }
    m
}

fn best_complete_shard_plan(files: &[HfWeightFile]) -> Option<Vec<String>> {
    let grouped = shard_groups_from_files(files);
    let mut best: Option<(i64, Vec<String>)> = None;
    for (key, parts) in grouped {
        if parts.len() as u32 != key.total {
            continue;
        }
        if !(1..=key.total).all(|i| parts.contains_key(&i)) {
            continue;
        }
        let paths: Vec<String> = (1..=key.total).map(|i| parts[&i].clone()).collect();
        let sum: i64 = paths
            .iter()
            .filter_map(|p| files.iter().find(|f| f.path == *p).and_then(|f| f.size))
            .sum();
        if best.as_ref().map(|(s, _)| *s).unwrap_or(-1) < sum {
            best = Some((sum, paths));
        }
    }
    best.map(|(_, p)| p)
}

fn shard_groups_detect_incomplete(files: &[HfWeightFile]) -> bool {
    let grouped = shard_groups_from_files(files);
    if grouped.is_empty() {
        return false;
    }
    grouped.iter().any(|(key, parts)| {
        parts.len() as u32 != key.total || (1..=key.total).any(|i| !parts.contains_key(&i))
    })
}

pub fn pick_auto_download_plan(files: &[HfWeightFile]) -> Result<AutoDownloadPlan, String> {
    if files.is_empty() {
        return Err("No weight files to download.".into());
    }

    let gguf_slice: Vec<HfGgufFile> = files
        .iter()
        .filter(|f| f.kind == HfWeightKind::Gguf)
        .map(|f| HfGgufFile {
            path: f.path.clone(),
            size: f.size,
        })
        .collect();
    if !gguf_slice.is_empty() {
        let picked = pick_recommended_gguf(&gguf_slice)
            .ok_or_else(|| "Could not pick a GGUF file.".to_string())?;
        return Ok(AutoDownloadPlan::Single {
            repo_path: picked.path.clone(),
            kind: HfWeightKind::Gguf,
        });
    }

    if shard_groups_detect_incomplete(files) {
        return Err(
            "Split safetensors (shards) in this repo look incomplete; pick files manually or fix the revision."
                .into(),
        );
    }

    if let Some(paths) = best_complete_shard_plan(files) {
        return Ok(AutoDownloadPlan::ShardedSafetensors { repo_paths: paths });
    }

    let singles: Vec<&HfWeightFile> = files
        .iter()
        .filter(|f| f.kind == HfWeightKind::Safetensors)
        .filter(|f| {
            Path::new(&f.path)
                .file_name()
                .and_then(|n| n.to_str())
                .map(|leaf| parse_shard_leaf(leaf).is_none())
                .unwrap_or(false)
        })
        .collect();

    if singles.is_empty() {
        return Err("No safetensors candidate found.".into());
    }

    const PREF_ENDINGS: &[&str] = &[
        "model.safetensors",
        "pytorch_model.safetensors",
        "adapter_model.safetensors",
    ];
    for ending in PREF_ENDINGS {
        if let Some(f) = singles
            .iter()
            .find(|f| f.path.to_ascii_lowercase().ends_with(ending))
        {
            return Ok(AutoDownloadPlan::Single {
                repo_path: (*f).path.clone(),
                kind: HfWeightKind::Safetensors,
            });
        }
    }

    let best = *singles
        .iter()
        .max_by_key(|f| f.size.unwrap_or(0))
        .expect("singles non-empty");
    Ok(AutoDownloadPlan::Single {
        repo_path: best.path.clone(),
        kind: HfWeightKind::Safetensors,
    })
}
