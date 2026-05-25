use crate::chat_tools;
use crate::cloud_infer;
use crate::api_server;
use crate::db;
use crate::download_manager::{DownloadJobDto, DownloadManager};
use crate::engine::{self, GenerationParams};
use crate::huggingface::{self, HfWeightFile, HfWeightKind};
#[cfg(feature = "llama-sidecar")]
use crate::llama_runtime;
use crate::model_knowledge;
use crate::state::AppState;
use crate::system_metrics;
use crate::tool_agent;
use chrono::Utc;
use rusqlite::params;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;
use tauri::Manager;
use tauri::ipc::Channel;
use uuid::Uuid;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageDto {
    pub id: String,
    pub chat_id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeadlessServerStatusDto {
    pub running: bool,
    pub pid: Option<u32>,
    pub host: String,
    pub port: u16,
    pub auth_mode: String,
    pub base_url: String,
    pub data_dir: String,
    pub models_dir: String,
    pub runtime_dir: String,
    pub command: String,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeadlessServerStartArgs {
    pub host: String,
    pub port: u16,
    pub data_dir: String,
    pub models_dir: String,
    pub runtime_dir: String,
    pub auth_mode: String,
    pub api_key: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatDto {
    pub id: String,
    pub title: String,
    pub model_id: Option<String>,
    pub model_name: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub preview: Option<String>,
    /// Per-chat instructions prepended as a `system` message (optional).
    pub system_prompt: String,
    pub project_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDto {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub context: String,
    pub created_at: String,
    pub updated_at: String,
    pub chat_count: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatContextUsageDto {
    pub used_tokens: u32,
    pub limit_tokens: u32,
    pub remaining_tokens: u32,
    pub reserved_output_tokens: u32,
    pub used_percent: f32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDto {
    pub id: String,
    pub name: String,
    pub path: String,
    pub quant: Option<String>,
    pub size_bytes: Option<i64>,
    pub created_at: String,
    /// `gguf` (loadable) or `safetensors` (weights on disk only).
    pub weights_format: String,
    pub shard_index: Option<i32>,
    pub shard_total: Option<i32>,
    /// `chat` (default).
    pub model_kind: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedDto {
    pub id: String,
    pub name: String,
    pub path: String,
    /// Present when an mmproj GGUF sits next to the main model; llama-server gets `--mmproj` automatically.
    pub mmproj_path: Option<String>,
    /// `chat` or `cloud`.
    pub model_kind: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkPromptResultDto {
    pub id: String,
    pub title: String,
    pub category: String,
    pub prompt: String,
    pub output: String,
    pub error: Option<String>,
    pub latency_ms: u128,
    pub chars: usize,
    pub estimated_tokens: usize,
    pub tokens_per_second: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkModelResultDto {
    pub model_id: String,
    pub model_name: String,
    pub weights_format: String,
    pub load_ms: u128,
    pub total_ms: u128,
    pub ram_before_gb: f64,
    pub ram_after_gb: f64,
    pub ram_delta_gb: f64,
    pub avg_latency_ms: f64,
    pub avg_tokens_per_second: f64,
    pub total_estimated_tokens: usize,
    pub prompts: Vec<BenchmarkPromptResultDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkRunDto {
    pub run_id: String,
    pub created_at: String,
    pub models: Vec<BenchmarkModelResultDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardDto {
    pub chat_count: i64,
    pub model_count: i64,
    pub loaded: Option<LoadedDto>,
    pub models_dir_path: String,
    pub models_dir_used_bytes: u64,
    pub app_data_used_bytes: u64,
}

fn loaded_dto_from_slot(m: &engine::LoadedModel, model_kind: &str) -> LoadedDto {
    let mmproj = crate::mmproj_detect::auto_discover_mmproj(m.path.as_path())
        .map(|p| p.to_string_lossy().into_owned());
    LoadedDto {
        id: m.id.clone(),
        name: m.name.clone(),
        path: m.path.to_string_lossy().into_owned(),
        mmproj_path: mmproj,
        model_kind: model_kind.to_string(),
    }
}

fn model_kind_for_id(conn: &rusqlite::Connection, id: &str) -> String {
    conn.query_row(
        "SELECT COALESCE(model_kind,'chat') FROM models WHERE id = ?1",
        params![id],
        |r| r.get(0),
    )
    .unwrap_or_else(|_| "chat".into())
}

fn loaded_dto_for_model(
    conn: &rusqlite::Connection,
    id: String,
    name: String,
    path: String,
    mmproj_path: Option<String>,
) -> LoadedDto {
    let kind = model_kind_for_id(conn, &id);
    LoadedDto {
        id,
        name,
        path,
        mmproj_path,
        model_kind: kind,
    }
}

fn parse_u32_setting(conn: &rusqlite::Connection, key: &str, default: u32) -> u32 {
    db::get_setting(conn, key)
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn parse_f32_setting(conn: &rusqlite::Connection, key: &str, default: f32) -> f32 {
    db::get_setting(conn, key)
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn parse_i32_setting(conn: &rusqlite::Connection, key: &str, default: i32) -> i32 {
    db::get_setting(conn, key)
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

#[tauri::command]
pub fn get_settings(state: tauri::State<'_, AppState>) -> Result<Vec<(String, String)>, String> {
    let conn = state.db.lock();
    let mut stmt = conn
        .prepare("SELECT key, value FROM settings ORDER BY key")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn set_setting(
    state: tauri::State<'_, AppState>,
    key: String,
    value: String,
) -> Result<(), String> {
    if key == "app_data_dir" {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            crate::data_root::write_override(&state.bootstrap_dir, None)?;
        } else {
            let pb = PathBuf::from(trimmed);
            std::fs::create_dir_all(&pb).map_err(|e| e.to_string())?;
            crate::data_root::write_override(&state.bootstrap_dir, Some(pb.as_path()))?;
        }
        return Ok(());
    }
    let conn = state.db.lock();
    db::set_setting(&conn, &key, &value).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_api_server_settings(
    state: tauri::State<'_, AppState>,
) -> Result<api_server::ApiServerSettingsDto, String> {
    Ok(api_server::settings_from_db(&state))
}

#[tauri::command]
pub fn get_api_server_status(
    state: tauri::State<'_, AppState>,
) -> Result<api_server::ApiServerStatusDto, String> {
    Ok(api_server::status(&state))
}

#[cfg(feature = "llama-sidecar")]
#[tauri::command]
pub fn get_llm_runtime_status(
    state: tauri::State<'_, AppState>,
) -> Result<llama_runtime::LlamaRuntimeStatusDto, String> {
    Ok(llama_runtime::status(&state))
}

#[cfg(not(feature = "llama-sidecar"))]
#[tauri::command]
pub fn get_llm_runtime_status() -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "phase": "idle",
        "modelId": null,
        "modelName": null,
        "error": null,
        "baseUrl": null
    }))
}

#[cfg(feature = "llama-sidecar")]
#[tauri::command]
pub fn validate_llama_runtime(app: tauri::AppHandle) -> Result<String, String> {
    llama_runtime::runtime_validation(&app)
}

#[cfg(not(feature = "llama-sidecar"))]
#[tauri::command]
pub fn validate_llama_runtime() -> Result<String, String> {
    Err("This build does not include the bundled llama-server runtime.".into())
}

fn headless_pid_path(state: &AppState) -> PathBuf {
    state.app_data_dir.join("localmod-server.pid")
}

fn headless_server_exe(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    fn looks_like_real_server(path: &Path) -> bool {
        path.metadata()
            .map(|m| m.is_file() && m.len() > 4096)
            .unwrap_or(false)
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = if cfg!(target_os = "windows") {
                dir.join("localmod-server.exe")
            } else {
                dir.join("localmod-server")
            };
            if looks_like_real_server(&candidate) {
                return Ok(candidate);
            }
        }
    }
    #[cfg(debug_assertions)]
    {
        let candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("debug")
            .join(if cfg!(target_os = "windows") {
                "localmod-server.exe"
            } else {
                "localmod-server"
            });
        if looks_like_real_server(&candidate) {
            return Ok(candidate);
        }
    }
    if let Ok(resource_dir) = app.path().resource_dir() {
        let candidate = resource_dir
            .join("binaries")
            .join(if cfg!(target_os = "windows") {
                "localmod-server.exe"
            } else {
                "localmod-server"
            });
        if looks_like_real_server(&candidate) {
            return Ok(candidate);
        }
        let candidate = resource_dir.join(if cfg!(target_os = "windows") {
            "localmod-server.exe"
        } else {
            "localmod-server"
        });
        if looks_like_real_server(&candidate) {
            return Ok(candidate);
        }
    }
    Err("Could not find localmod-server executable. Build the headless server binary first.".into())
}

fn is_pid_running(pid: u32) -> bool {
    #[cfg(windows)]
    {
        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.contains(&pid.to_string()))
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

fn kill_pid(pid: u32) -> Result<(), String> {
    #[cfg(windows)]
    {
        Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .status()
            .map_err(|e| e.to_string())?
            .success()
            .then_some(())
            .ok_or_else(|| format!("Failed to stop localmod-server process {pid}"))
    }
    #[cfg(not(windows))]
    {
        Command::new("kill")
            .args([&pid.to_string()])
            .status()
            .map_err(|e| e.to_string())?
            .success()
            .then_some(())
            .ok_or_else(|| format!("Failed to stop localmod-server process {pid}"))
    }
}

fn headless_settings(state: &AppState) -> (String, u16, String, String, String, String, String) {
    let conn = state.db.lock();
    let get = |key: &str, default: &str| {
        db::get_setting(&conn, key)
            .ok()
            .flatten()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| default.into())
    };
    let host = get("headless_server_host", "127.0.0.1");
    let port = get("headless_server_port", "11435")
        .parse::<u16>()
        .ok()
        .filter(|p| *p > 0)
        .unwrap_or(11435);
    let auth_mode = get("headless_server_auth_mode", "none");
    let api_key = db::get_setting(&conn, "headless_server_key")
        .ok()
        .flatten()
        .unwrap_or_default();
    let data_dir = get(
        "headless_server_data_dir",
        &state.app_data_dir.to_string_lossy(),
    );
    let models_dir = db::get_setting(&conn, "headless_server_models_dir")
        .ok()
        .flatten()
        .unwrap_or_default();
    let runtime_dir = db::get_setting(&conn, "headless_server_runtime_dir")
        .ok()
        .flatten()
        .unwrap_or_default();
    (host, port, auth_mode, api_key, data_dir, models_dir, runtime_dir)
}

fn read_headless_pid(state: &AppState) -> Option<u32> {
    std::fs::read_to_string(headless_pid_path(state))
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

#[tauri::command]
pub fn get_headless_server_status(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<HeadlessServerStatusDto, String> {
    let (host, port, auth_mode, _api_key, data_dir, models_dir, runtime_dir) =
        headless_settings(&state);
    let pid = read_headless_pid(&state).filter(|pid| is_pid_running(*pid));
    let exe = headless_server_exe(&app).ok();
    let command = exe
        .as_ref()
        .map(|p| {
            format!(
                "\"{}\" --host {} --port {} --data-dir \"{}\"{}{} --auth {}",
                p.display(),
                host,
                port,
                data_dir,
                if models_dir.trim().is_empty() { "" } else { " --models-dir " },
                if models_dir.trim().is_empty() { "" } else { models_dir.as_str() },
                auth_mode
            )
        })
        .unwrap_or_default();
    Ok(HeadlessServerStatusDto {
        running: pid.is_some(),
        pid,
        host: host.clone(),
        port,
        auth_mode: auth_mode.clone(),
        base_url: format!("http://{}:{}/v1", host, port),
        data_dir,
        models_dir,
        runtime_dir,
        command,
        last_error: None,
    })
}

#[tauri::command]
pub async fn start_headless_server(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    args: HeadlessServerStartArgs,
) -> Result<HeadlessServerStatusDto, String> {
    let host = args.host.trim().to_string();
    if host.is_empty() {
        return Err("Host cannot be empty.".into());
    }
    if args.port == 0 {
        return Err("Port must be between 1 and 65535.".into());
    }
    let auth_mode = if args.auth_mode == "bearer" { "bearer" } else { "none" }.to_string();
    if auth_mode == "bearer" && args.api_key.trim().is_empty() {
        return Err("Bearer auth requires an API key.".into());
    }
    let data_dir = if args.data_dir.trim().is_empty() {
        state.app_data_dir.to_string_lossy().into_owned()
    } else {
        args.data_dir.trim().to_string()
    };
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;

    if let Some(pid) = read_headless_pid(&state).filter(|pid| is_pid_running(*pid)) {
        return Err(format!("Headless server is already running as process {pid}. Stop it first."));
    }

    {
        let conn = state.db.lock();
        db::set_setting(&conn, "headless_server_host", &host).map_err(|e| e.to_string())?;
        db::set_setting(&conn, "headless_server_port", &args.port.to_string()).map_err(|e| e.to_string())?;
        db::set_setting(&conn, "headless_server_auth_mode", &auth_mode).map_err(|e| e.to_string())?;
        db::set_setting(&conn, "headless_server_key", args.api_key.trim()).map_err(|e| e.to_string())?;
        db::set_setting(&conn, "headless_server_data_dir", &data_dir).map_err(|e| e.to_string())?;
        db::set_setting(&conn, "headless_server_models_dir", args.models_dir.trim()).map_err(|e| e.to_string())?;
        db::set_setting(&conn, "headless_server_runtime_dir", args.runtime_dir.trim()).map_err(|e| e.to_string())?;
    }

    let exe = headless_server_exe(&app)?;
    let mut cmd = Command::new(&exe);
    cmd.arg("--host")
        .arg(&host)
        .arg("--port")
        .arg(args.port.to_string())
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("--auth")
        .arg(&auth_mode)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if auth_mode == "bearer" {
        cmd.arg("--api-key").arg(args.api_key.trim());
    }
    if !args.models_dir.trim().is_empty() {
        cmd.arg("--models-dir").arg(args.models_dir.trim());
    }
    if !args.runtime_dir.trim().is_empty() {
        cmd.arg("--runtime-dir").arg(args.runtime_dir.trim());
    }
    let child = cmd.spawn().map_err(|e| {
        format!(
            "Failed to start headless server from {}: {e}",
            exe.display()
        )
    })?;
    let pid = child.id();
    std::fs::write(headless_pid_path(&state), pid.to_string()).map_err(|e| e.to_string())?;
    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    get_headless_server_status(app, state)
}

#[tauri::command]
pub fn stop_headless_server(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<HeadlessServerStatusDto, String> {
    if let Some(pid) = read_headless_pid(&state) {
        if is_pid_running(pid) {
            kill_pid(pid)?;
        }
        let _ = std::fs::remove_file(headless_pid_path(&state));
    }
    get_headless_server_status(app, state)
}

#[tauri::command]
pub async fn start_api_server(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    auth_mode: String,
    api_key: String,
    enabled: bool,
) -> Result<api_server::ApiServerStatusDto, String> {
    {
        let conn = state.db.lock();
        db::set_setting(&conn, "api_server_enabled", if enabled { "true" } else { "false" })
            .map_err(|e| e.to_string())?;
        db::set_setting(&conn, "api_server_host", host.trim()).map_err(|e| e.to_string())?;
        db::set_setting(&conn, "api_server_port", &port.to_string()).map_err(|e| e.to_string())?;
        db::set_setting(
            &conn,
            "api_server_auth_mode",
            if auth_mode == "bearer" { "bearer" } else { "none" },
        )
        .map_err(|e| e.to_string())?;
        db::set_setting(&conn, "api_server_key", api_key.trim()).map_err(|e| e.to_string())?;
    }
    if enabled {
        api_server::start(app, &state, host, port, auth_mode, api_key).await
    } else {
        api_server::stop(&state).await?;
        Ok(api_server::status(&state))
    }
}

#[tauri::command]
pub async fn stop_api_server(
    state: tauri::State<'_, AppState>,
) -> Result<api_server::ApiServerStatusDto, String> {
    {
        let conn = state.db.lock();
        db::set_setting(&conn, "api_server_enabled", "false").map_err(|e| e.to_string())?;
    }
    api_server::stop(&state).await?;
    Ok(api_server::status(&state))
}

#[tauri::command]
pub fn get_dashboard(state: tauri::State<'_, AppState>) -> Result<DashboardDto, String> {
    let conn = state.db.lock();
    let loaded = state
        .loaded
        .lock()
        .as_ref()
        .map(|m| loaded_dto_from_slot(m, &model_kind_for_id(&conn, &m.id)));
    let (chat_count, model_count, models_dir_path, models_dir_used_bytes, app_data_used_bytes) = {
        let chat_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM chats", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let model_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM models", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let models_dir_path = db::get_setting(&conn, "models_dir")
            .map_err(|e| e.to_string())?
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| state.models_dir.to_string_lossy().into_owned());
        let models_dir_used_bytes =
            crate::fs_util::dir_size(Path::new(&models_dir_path)).unwrap_or(0);
        let app_data_used_bytes = crate::fs_util::dir_size(&state.app_data_dir).unwrap_or(0);
        (
            chat_count,
            model_count,
            models_dir_path,
            models_dir_used_bytes,
            app_data_used_bytes,
        )
    };
    Ok(DashboardDto {
        chat_count,
        model_count,
        loaded,
        models_dir_path,
        models_dir_used_bytes,
        app_data_used_bytes,
    })
}

#[tauri::command]
pub fn get_system_snapshot() -> Result<system_metrics::SystemSnapshot, String> {
    Ok(system_metrics::capture_system_snapshot())
}

fn resolved_context_dir(
    conn: &rusqlite::Connection,
    app_data: &Path,
) -> Result<PathBuf, String> {
    let p = db::get_setting(conn, "context_dir").map_err(|e| e.to_string())?;
    let path = if let Some(ref s) = p {
        if !s.trim().is_empty() {
            PathBuf::from(s)
        } else {
            app_data.join("context")
        }
    } else {
        app_data.join("context")
    };
    std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    Ok(path)
}

fn sanitize_filename(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        s = "note".into();
    }
    if s.len() > 120 {
        s.truncate(120);
    }
    s
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathsDto {
    pub app_data_dir: String,
    pub database_path: String,
    pub models_dir: String,
    pub context_dir_resolved: String,
    /// Folder where `data_root.json` lives (Tauri default app data).
    pub builtin_app_data_dir: String,
    /// Folder chosen for databases & reference storage (same as `app_data_dir` after restart).
    pub configured_app_data_dir: String,
}

#[tauri::command]
pub fn get_paths(state: tauri::State<'_, AppState>) -> Result<PathsDto, String> {
    let conn = state.db.lock();
    let models_dir = db::get_setting(&conn, "models_dir")
        .map_err(|e| e.to_string())?
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| state.models_dir.to_string_lossy().into_owned());
    let context_dir_resolved = resolved_context_dir(&conn, &state.app_data_dir)?.to_string_lossy().into_owned();
    let configured =
        crate::data_root::configured_display(&state.bootstrap_dir, &state.app_data_dir);
    Ok(PathsDto {
        app_data_dir: state.app_data_dir.to_string_lossy().into_owned(),
        database_path: state.db_path.to_string_lossy().into_owned(),
        models_dir,
        context_dir_resolved,
        builtin_app_data_dir: state.bootstrap_dir.to_string_lossy().into_owned(),
        configured_app_data_dir: configured.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub fn list_models(state: tauri::State<'_, AppState>) -> Result<Vec<ModelDto>, String> {
    let conn = state.db.lock();
    db::sync_cloud_models_from_settings(&conn).map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, name, path, quant, size_bytes, created_at, weights_format, shard_index, shard_total, COALESCE(model_kind, 'chat') FROM models ORDER BY name",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ModelDto {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                quant: row.get(3)?,
                size_bytes: row.get(4)?,
                created_at: row.get(5)?,
                weights_format: row.get(6)?,
                shard_index: row.get(7)?,
                shard_total: row.get(8)?,
                model_kind: row.get(9)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudProviderUiDto {
    pub id: String,
    pub model: String,
    pub has_api_key: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub base_url: String,
    pub image_generation_enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub image_model: String,
}

#[tauri::command]
pub fn get_cloud_provider_configs(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<CloudProviderUiDto>, String> {
    let conn = state.db.lock();
    db::sync_cloud_models_from_settings(&conn).map_err(|e| e.to_string())?;
    let mut out: Vec<CloudProviderUiDto> = Vec::new();
    for (id, setting_key) in [
        ("openai", "cloud_openai"),
        ("anthropic", "cloud_anthropic"),
        ("openrouter", "cloud_openrouter"),
        ("custom", "cloud_custom"),
    ] {
        let raw = db::get_setting(&conn, setting_key).map_err(|e| e.to_string())?;
        let (model, has_api_key, base_url, image_generation_enabled, image_model) =
            if let Some(s) = raw.filter(|x| !x.trim().is_empty()) {
                match serde_json::from_str::<cloud_infer::CloudProviderStored>(&s) {
                    Ok(c) => (
                        c.model,
                        !c.api_key.trim().is_empty(),
                        c.base_url.unwrap_or_default(),
                        c.image_generation_enabled,
                        c.image_model.unwrap_or_default(),
                    ),
                    Err(_) => (String::new(), false, String::new(), false, String::new()),
                }
            } else {
                (String::new(), false, String::new(), false, String::new())
            };
        out.push(CloudProviderUiDto {
            id: id.into(),
            model,
            has_api_key,
            base_url,
            image_generation_enabled,
            image_model,
        });
    }
    Ok(out)
}

#[tauri::command]
pub fn set_cloud_provider_config(
    state: tauri::State<'_, AppState>,
    provider: String,
    api_key: String,
    model: String,
    base_url: Option<String>,
    image_generation_enabled: Option<bool>,
    image_model: Option<String>,
) -> Result<(), String> {
    let key = match provider.trim() {
        "openai" => "cloud_openai",
        "anthropic" => "cloud_anthropic",
        "openrouter" => "cloud_openrouter",
        "custom" => "cloud_custom",
        _ => return Err("Unknown provider.".into()),
    };
    let conn = state.db.lock();
    let model_t = model.trim().to_string();
    if model_t.is_empty() {
        db::set_setting(&conn, key, "").map_err(|e| e.to_string())?;
        db::sync_cloud_models_from_settings(&conn).map_err(|e| e.to_string())?;
        return Ok(());
    }
    let mut api_key_fin = api_key;
    if api_key_fin.trim().is_empty() {
        if let Some(raw) = db::get_setting(&conn, key).map_err(|e| e.to_string())? {
            if let Ok(prev) = serde_json::from_str::<cloud_infer::CloudProviderStored>(&raw) {
                if !prev.api_key.trim().is_empty() {
                    api_key_fin = prev.api_key;
                }
            }
        }
    }
    if api_key_fin.trim().is_empty() && provider.trim() != "custom" {
        return Err("API key is required when a model name is set.".into());
    }
    let base_url_fin = if provider.trim() == "custom" {
        let b = base_url.unwrap_or_default().trim().to_string();
        if b.is_empty() {
            if let Some(raw) = db::get_setting(&conn, key).map_err(|e| e.to_string())? {
                if let Ok(prev) = serde_json::from_str::<cloud_infer::CloudProviderStored>(&raw) {
                    if let Some(ref prev_base) = prev.base_url {
                        if !prev_base.trim().is_empty() {
                            Some(prev_base.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            Some(b)
        }
    } else {
        None
    };
    if provider.trim() == "custom" && base_url_fin.as_deref().unwrap_or("").trim().is_empty() {
        return Err("Base URL is required for custom providers.".into());
    }
    let image_gen_was_none = image_generation_enabled.is_none();
    let image_model_was_none = image_model.is_none();
    let mut image_gen_enabled = image_generation_enabled.unwrap_or(false);
    let mut image_model_fin = image_model.as_deref().unwrap_or("").trim().to_string();
    if let Some(raw) = db::get_setting(&conn, key).map_err(|e| e.to_string())? {
        if let Ok(prev) = serde_json::from_str::<cloud_infer::CloudProviderStored>(&raw) {
            if image_gen_was_none {
                image_gen_enabled = prev.image_generation_enabled;
            }
            if image_model_was_none {
                image_model_fin = prev.image_model.unwrap_or_default();
            }
        }
    }
    let stored = cloud_infer::CloudProviderStored {
        api_key: api_key_fin,
        model: model_t,
        base_url: base_url_fin,
        image_generation_enabled: image_gen_enabled,
        image_model: if image_model_fin.is_empty() {
            None
        } else {
            Some(image_model_fin)
        },
    };
    let json = serde_json::to_string(&stored).map_err(|e| e.to_string())?;
    db::set_setting(&conn, key, &json).map_err(|e| e.to_string())?;
    db::sync_cloud_models_from_settings(&conn).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn chat_image_gen_available(
    state: tauri::State<'_, AppState>,
    chat_id: String,
    model_id: Option<String>,
) -> Result<bool, String> {
    let conn = state.db.lock();
    let _ = db::sync_cloud_models_from_settings(&conn);
    let override_id = model_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    Ok(
        crate::image_gen::resolve_for_chat_with_model(
            &conn,
            chat_id.trim(),
            override_id,
        )
        .is_some(),
    )
}

#[tauri::command]
pub fn read_generated_image_data_url(
    state: tauri::State<'_, AppState>,
    filename: String,
) -> Result<String, String> {
    let name = filename.trim();
    if name.is_empty()
        || name.contains("..")
        || name.contains('/')
        || name.contains('\\')
    {
        return Err("Invalid image filename.".into());
    }
    let path = crate::image_gen::generated_dir(&state.app_data_dir).join(name);
    if !path.is_file() {
        return Err(format!("Generated image not found: {name}"));
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:image/png;base64,{b64}"))
}

#[tauri::command]
pub fn export_generated_image(
    state: tauri::State<'_, AppState>,
    filename: String,
    dest_path: String,
) -> Result<(), String> {
    let name = filename.trim();
    if name.is_empty()
        || name.contains("..")
        || name.contains('/')
        || name.contains('\\')
    {
        return Err("Invalid image filename.".into());
    }
    let src = crate::image_gen::generated_dir(&state.app_data_dir).join(name);
    if !src.is_file() {
        return Err(format!("Generated image not found: {name}"));
    }
    let dest = std::path::PathBuf::from(dest_path.trim());
    if dest.as_os_str().is_empty() {
        return Err("Destination path is required.".into());
    }
    if let Some(parent) = dest.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    std::fs::copy(&src, &dest).map_err(|e| format!("Failed to save image: {e}"))?;
    Ok(())
}

/// Register a `.gguf` or `.safetensors` on disk.
pub fn register_weights_inner(
    conn: &rusqlite::Connection,
    path: &Path,
    weights_format: &str,
    shard_index: Option<i32>,
    shard_total: Option<i32>,
) -> Result<ModelDto, String> {
    if !path.exists() {
        return Err(format!("File not found: {}", path.display()));
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let inferred = match ext.as_str() {
        "gguf" => "gguf",
        "safetensors" => "safetensors",
        _ => {
            return Err("Only .gguf and .safetensors are supported.".into());
        }
    };
    if weights_format != inferred {
        return Err("Internal format mismatch for weight file.".into());
    }
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let path_str = path.to_string_lossy().into_owned();
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "model".into());
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    conn.execute(
        "INSERT INTO models (id, name, path, quant, size_bytes, created_at, weights_format, shard_index, shard_total, model_kind) VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, ?8, 'chat')",
        params![
            id,
            name,
            path_str,
            meta.len() as i64,
            now.clone(),
            weights_format,
            shard_index,
            shard_total,
        ],
    )
    .map_err(|e| {
        let msg = e.to_string();
        if msg.contains("UNIQUE") {
            "This file is already in your model library.".into()
        } else {
            msg
        }
    })?;
    Ok(ModelDto {
        id,
        name,
        path: path.to_string_lossy().into_owned(),
        quant: None,
        size_bytes: Some(meta.len() as i64),
        created_at: now,
        weights_format: weights_format.to_string(),
        shard_index,
        shard_total,
        model_kind: "chat".to_string(),
    })
}

pub fn register_model_inner(conn: &rusqlite::Connection, path: &Path) -> Result<ModelDto, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let wf = match ext.as_str() {
        "gguf" => "gguf",
        "safetensors" => "safetensors",
        _ => {
            return Err("Only .gguf or .safetensors.".into());
        }
    };
    register_weights_inner(conn, path, wf, None, None)
}

#[tauri::command]
pub fn register_model(state: tauri::State<'_, AppState>, path: String) -> Result<ModelDto, String> {
    let p = PathBuf::from(path);
    let conn = state.db.lock();
    register_model_inner(&conn, &p)
}

fn fetch_model_dto(conn: &rusqlite::Connection, id: &str) -> Result<ModelDto, String> {
    conn.query_row(
        "SELECT id, name, path, quant, size_bytes, created_at, weights_format, shard_index, shard_total, COALESCE(model_kind, 'chat') FROM models WHERE id = ?1",
        params![id],
        |row| {
            Ok(ModelDto {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                quant: row.get(3)?,
                size_bytes: row.get(4)?,
                created_at: row.get(5)?,
                weights_format: row.get(6)?,
                shard_index: row.get(7)?,
                shard_total: row.get(8)?,
                model_kind: row.get(9)?,
            })
        },
    )
    .map_err(|_| "Model not found.".to_string())
}

#[tauri::command]
pub fn update_model(
    state: tauri::State<'_, AppState>,
    model_id: String,
    name: Option<String>,
    path: Option<String>,
) -> Result<ModelDto, String> {
    let id = model_id.trim();
    if id.is_empty() {
        return Err("Model id is required.".into());
    }
    let conn = state.db.lock();
    let wf: String = conn
        .query_row(
            "SELECT COALESCE(weights_format, 'gguf') FROM models WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|_| "Model not found.".to_string())?;
    if wf == "cloud" {
        return Err("Cloud models are edited via Set up cloud models.".into());
    }

    if let Some(n) = name {
        let trimmed = n.trim();
        if trimmed.is_empty() {
            return Err("Display name cannot be empty.".into());
        }
        conn.execute(
            "UPDATE models SET name = ?1 WHERE id = ?2",
            params![trimmed, id],
        )
        .map_err(|e| e.to_string())?;
    }

    if let Some(p) = path {
        let pb = PathBuf::from(p.trim());
        if !pb.is_file() {
            return Err(format!("File not found: {}", pb.display()));
        }
        let ext = pb
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let inferred = match ext.as_str() {
            "gguf" => "gguf",
            "safetensors" => "safetensors",
            _ => return Err("Path must be a .gguf or .safetensors file.".into()),
        };
        if inferred != wf {
            return Err(format!(
                "This library entry is {wf}; pick a .{wf} file."
            ));
        }
        let path_str = pb.to_string_lossy().into_owned();
        let size = std::fs::metadata(&pb).map_err(|e| e.to_string())?.len() as i64;
        conn.execute(
            "UPDATE models SET path = ?1, size_bytes = ?2 WHERE id = ?3",
            params![path_str, size, id],
        )
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("UNIQUE") {
                "Another model already uses that file path.".into()
            } else {
                msg
            }
        })?;
    }

    fetch_model_dto(&conn, id)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HfWeightFileDto {
    pub path: String,
    pub size: Option<i64>,
    pub kind: HfWeightKind,
}

impl From<HfWeightFile> for HfWeightFileDto {
    fn from(f: HfWeightFile) -> Self {
        Self {
            path: f.path,
            size: f.size,
            kind: f.kind,
        }
    }
}

#[tauri::command]
pub async fn list_huggingface_gguf_files(
    repo_input: String,
    revision: Option<String>,
) -> Result<Vec<HfWeightFileDto>, String> {
    let token = huggingface::resolve_hf_token();
    let repo_id = huggingface::normalize_repo_id(&repo_input)?;
    let client = huggingface::hf_client(token)?;
    let rev_override = revision
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let (files, _) = huggingface::list_weights_resolved(&client, &repo_id, rev_override).await?;
    Ok(files.into_iter().map(HfWeightFileDto::from).collect())
}

#[tauri::command]
pub fn hf_download_list(state: tauri::State<'_, AppState>) -> Vec<DownloadJobDto> {
    state.downloads.list_jobs()
}

#[tauri::command]
pub fn hf_download_pause(state: tauri::State<'_, AppState>, job_id: String) -> Result<(), String> {
    state.downloads.pause(&job_id)
}

#[tauri::command]
pub fn hf_download_resume(state: tauri::State<'_, AppState>, job_id: String) -> Result<(), String> {
    state.downloads.resume(&job_id)
}

#[tauri::command]
pub fn hf_download_cancel(state: tauri::State<'_, AppState>, job_id: String) -> Result<(), String> {
    state.downloads.cancel(&job_id)
}

#[tauri::command]
pub fn hf_download_dismiss(state: tauri::State<'_, AppState>, job_id: String) -> Result<(), String> {
    state.downloads.dismiss(&job_id)
}

#[tauri::command]
pub fn hf_download_start_auto(
    state: tauri::State<'_, AppState>,
    repo_input: String,
) -> Result<String, String> {
    let s = repo_input.trim();
    if s.is_empty() {
        return Err("Repo required.".into());
    }
    let job_id = state.downloads.create_job(s.to_string());
    let dm = state.downloads.clone();
    let db = state.db.clone();
    let models_dir = state.models_dir.clone();
    let input = repo_input;
    let j = job_id.clone();
    tauri::async_runtime::spawn(async move {
        run_hf_download_auto_job(dm, db, models_dir, j, input).await;
    });
    Ok(job_id)
}

#[tauri::command]
pub fn hf_download_start_manual(
    state: tauri::State<'_, AppState>,
    repo_input: String,
    file_path: String,
    revision: Option<String>,
) -> Result<String, String> {
    let repo_in = repo_input.trim().to_string();
    let file_in = file_path.trim().to_string();
    if file_in.is_empty() {
        return Err("File path required.".into());
    }
    if repo_in.is_empty() {
        return Err("Repo required.".into());
    }
    let job_id = state.downloads.create_job(format!(
        "{} » {}",
        repo_in,
        Path::new(&file_in)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&file_in)
    ));
    let dm = state.downloads.clone();
    let db = state.db.clone();
    let models_dir = state.models_dir.clone();
    let j = job_id.clone();
    tauri::async_runtime::spawn(async move {
        run_hf_download_manual_job(
            dm,
            db,
            models_dir,
            j,
            repo_in,
            file_in,
            revision,
        )
        .await;
    });
    Ok(job_id)
}

#[tauri::command]
pub fn get_model_knowledge(path: String) -> Result<model_knowledge::ModelKnowledgeDto, String> {
    let p = PathBuf::from(path.trim());
    if !p.exists() {
        return Err(format!("File not found: {}", p.display()));
    }
    Ok(model_knowledge::knowledge_for_weights_path(&p))
}

/// Local files to delete for a single `models` row (`path` points at shard 1 or the only file).
fn weight_paths_for_model_row(main: &Path, shard_total: Option<i32>) -> Vec<PathBuf> {
    let Some(total_db) = shard_total.filter(|&t| t > 1) else {
        return vec![main.to_path_buf()];
    };
    if u32::try_from(total_db).is_err() {
        return vec![main.to_path_buf()];
    }
    let Some(leaf) = main.file_name().and_then(|n| n.to_str()) else {
        return vec![main.to_path_buf()];
    };
    let Some(name) = leaf.strip_suffix(".safetensors") else {
        return vec![main.to_path_buf()];
    };
    let Some((left, total_s)) = name.rsplit_once("-of-") else {
        return vec![main.to_path_buf()];
    };
    let Ok(parsed_total) = total_s.parse::<u32>() else {
        return vec![main.to_path_buf()];
    };
    let Some((prefix, idx_s)) = left.rsplit_once('-') else {
        return vec![main.to_path_buf()];
    };
    if idx_s.parse::<u32>().is_err() {
        return vec![main.to_path_buf()];
    }
    let width_idx = idx_s.len();
    let width_tot = total_s.len();
    let dir = main
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    (1..=parsed_total)
        .map(|i| {
            dir.join(format!(
                "{}-{:0wi$}-of-{:0wt$}.safetensors",
                prefix,
                i,
                parsed_total,
                wi = width_idx,
                wt = width_tot
            ))
        })
        .collect()
}

#[tauri::command]
pub async fn delete_model(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    let id = id.trim().to_string();
    if id.is_empty() {
        return Err("No model id.".into());
    }

    let row = {
        let conn = state.db.lock();
        conn.query_row(
            "SELECT path, shard_total, COALESCE(weights_format, 'gguf') FROM models WHERE id = ?1",
            params![&id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<i32>>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?
    };
    let Some((path_str, shard_total, weights_format)) = row else {
        return Err("Model not found.".into());
    };

    let is_cloud = weights_format == "cloud";

    let was_loaded = state
        .loaded
        .lock()
        .as_ref()
        .map(|l| l.id == id)
        .unwrap_or(false);
    if was_loaded {
        #[cfg(feature = "llama-sidecar")]
        {
            crate::llama_sidecar::kill_sidecar_slot_async(&state).await?;
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        }
        #[cfg(not(feature = "llama-sidecar"))]
        {
            *state.loaded.lock() = None;
        }
        let conn = state.db.lock();
        db::set_setting(&conn, "loaded_model_id", "").map_err(|e| e.to_string())?;
    }

    {
        let conn = state.db.lock();
        let n = conn
            .execute("DELETE FROM models WHERE id = ?1", params![&id])
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("Model not found.".into());
        }
    }

    if is_cloud {
        let conn = state.db.lock();
        db::clear_cloud_setting_for_model_id(&conn, &id).map_err(|e| e.to_string())?;
        return Ok(());
    }

    let main_pb = PathBuf::from(&path_str);
    let mut to_remove = weight_paths_for_model_row(&main_pb, shard_total);
    if weights_format == "gguf" {
        if let Some(mm) = crate::mmproj_detect::auto_discover_mmproj(&main_pb) {
            if mm != main_pb && !to_remove.contains(&mm) {
                to_remove.push(mm);
            }
        }
    }

    let mut errs: Vec<String> = Vec::new();
    for p in to_remove {
        if !p.exists() {
            continue;
        }
        if let Err(e) = std::fs::remove_file(&p) {
            errs.push(format!("{} ({e})", p.display()));
        }
    }
    if !errs.is_empty() {
        return Err(format!(
            "Model removed from the library but some files could not be deleted: {}",
            errs.join("; ")
        ));
    }

    Ok(())
}

#[tauri::command]
pub fn list_chats(state: tauri::State<'_, AppState>) -> Result<Vec<ChatDto>, String> {
    let conn = state.db.lock();
    let mut stmt = conn
        .prepare(
            r#"SELECT c.id, c.title, c.model_id, c.created_at, c.updated_at,
            COALESCE(c.system_prompt, ''), m.name as model_name,
            (SELECT content FROM messages WHERE chat_id = c.id ORDER BY created_at DESC LIMIT 1) as preview,
            c.project_id
            FROM chats c LEFT JOIN models m ON m.id = c.model_id
            ORDER BY c.updated_at DESC"#,
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ChatDto {
                id: row.get(0)?,
                title: row.get(1)?,
                model_id: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                system_prompt: row.get(5)?,
                model_name: row.get(6)?,
                preview: row.get(7)?,
                project_id: row.get(8)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn create_chat(
    state: tauri::State<'_, AppState>,
    title: Option<String>,
    model_id: Option<String>,
    project_id: Option<String>,
) -> Result<ChatDto, String> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let title_resolved = title.unwrap_or_else(|| "New chat".into());
    let conn = state.db.lock();
    if let Some(ref pid) = project_id {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM projects WHERE id = ?1",
                params![pid],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if exists == 0 {
            return Err("Project not found.".into());
        }
    }
    conn.execute(
        "INSERT INTO chats (id, title, model_id, created_at, updated_at, project_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, title_resolved.clone(), model_id, now, now, project_id],
    )
    .map_err(|e| e.to_string())?;
    let model_name: Option<String> = if let Some(ref mid) = model_id {
        conn
            .query_row(
                "SELECT name FROM models WHERE id = ?1",
                params![mid],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
    } else {
        None
    };
    Ok(ChatDto {
        id,
        title: title_resolved,
        model_id,
        model_name,
        created_at: now.clone(),
        updated_at: now,
        preview: None,
        system_prompt: String::new(),
        project_id,
    })
}

fn parse_project_tags(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

fn tags_to_json(tags: &[String]) -> String {
    let cleaned: Vec<String> = tags
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect();
    serde_json::to_string(&cleaned).unwrap_or_else(|_| "[]".into())
}

#[tauri::command]
pub fn list_projects(state: tauri::State<'_, AppState>) -> Result<Vec<ProjectDto>, String> {
    let conn = state.db.lock();
    let mut stmt = conn
        .prepare(
            r#"SELECT p.id, p.name, p.description, p.tags, p.context, p.created_at, p.updated_at,
            (SELECT COUNT(*) FROM chats c WHERE c.project_id = p.id) AS chat_count
            FROM projects p ORDER BY p.updated_at DESC"#,
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ProjectDto {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                tags: parse_project_tags(&row.get::<_, String>(3)?),
                context: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
                chat_count: row.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn create_project(
    state: tauri::State<'_, AppState>,
    name: String,
    description: Option<String>,
    tags: Option<Vec<String>>,
) -> Result<ProjectDto, String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("Project name is required.".into());
    }
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let desc = description.unwrap_or_default();
    let tags_json = tags_to_json(&tags.unwrap_or_default());
    let conn = state.db.lock();
    conn.execute(
        "INSERT INTO projects (id, name, description, tags, context, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, '', ?5, ?6)",
        params![id, n, desc, tags_json, now, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(ProjectDto {
        id,
        name: n.to_string(),
        description: desc,
        tags: parse_project_tags(&tags_json),
        context: String::new(),
        created_at: now.clone(),
        updated_at: now,
        chat_count: 0,
    })
}

#[tauri::command]
pub fn update_project(
    state: tauri::State<'_, AppState>,
    project_id: String,
    name: Option<String>,
    description: Option<String>,
    tags: Option<Vec<String>>,
    context: Option<String>,
) -> Result<ProjectDto, String> {
    let conn = state.db.lock();
    let row: (String, String, String, String, String, String) = conn
        .query_row(
            "SELECT name, description, tags, context, created_at, updated_at FROM projects WHERE id = ?1",
            params![project_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .map_err(|_| "Project not found.".to_string())?;
    let mut name_fin = row.0;
    let mut desc_fin = row.1;
    let mut tags_fin = row.2;
    let mut ctx_fin = row.3;
    if let Some(n) = name {
        let t = n.trim();
        if t.is_empty() {
            return Err("Project name cannot be empty.".into());
        }
        name_fin = t.to_string();
    }
    if let Some(d) = description {
        desc_fin = d;
    }
    if let Some(t) = tags {
        tags_fin = tags_to_json(&t);
    }
    if let Some(c) = context {
        ctx_fin = c;
    }
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE projects SET name = ?1, description = ?2, tags = ?3, context = ?4, updated_at = ?5 WHERE id = ?6",
        params![name_fin, desc_fin, tags_fin, ctx_fin, now, project_id],
    )
    .map_err(|e| e.to_string())?;
    let chat_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM chats WHERE project_id = ?1",
            params![project_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(ProjectDto {
        id: project_id,
        name: name_fin,
        description: desc_fin,
        tags: parse_project_tags(&tags_fin),
        context: ctx_fin,
        created_at: row.4,
        updated_at: now,
        chat_count,
    })
}

#[tauri::command]
pub fn delete_project(state: tauri::State<'_, AppState>, project_id: String) -> Result<(), String> {
    let conn = state.db.lock();
    let n = conn
        .execute("DELETE FROM projects WHERE id = ?1", params![project_id])
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("Project not found.".into());
    }
    Ok(())
}

#[tauri::command]
pub fn set_chat_system_prompt(
    state: tauri::State<'_, AppState>,
    chat_id: String,
    system_prompt: String,
) -> Result<(), String> {
    let now = Utc::now().to_rfc3339();
    let conn = state.db.lock();
    let n = conn
        .execute(
            "UPDATE chats SET system_prompt = ?1, updated_at = ?2 WHERE id = ?3",
            params![system_prompt, now, chat_id],
        )
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("Chat not found.".into());
    }
    Ok(())
}

#[tauri::command]
pub fn delete_chat(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    let mut conn = state.db.lock();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "DELETE FROM messages WHERE chat_id = ?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM chats WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn set_chat_model(
    state: tauri::State<'_, AppState>,
    chat_id: String,
    model_id: String,
) -> Result<(), String> {
    let exists: i64 = {
        let conn = state.db.lock();
        conn.query_row(
            "SELECT COUNT(*) FROM models WHERE id = ?1",
            params![model_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?
    };
    if exists == 0 {
        return Err("Model not found.".into());
    }
    let now = Utc::now().to_rfc3339();
    let conn = state.db.lock();
    let n = conn
        .execute(
            "UPDATE chats SET model_id = ?1, updated_at = ?2 WHERE id = ?3",
            params![model_id, now, chat_id],
        )
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("Chat not found.".into());
    }
    Ok(())
}

#[tauri::command]
pub fn rename_chat(
    state: tauri::State<'_, AppState>,
    chat_id: String,
    title: String,
) -> Result<(), String> {
    let t = title.trim();
    if t.is_empty() {
        return Err("Title cannot be empty.".into());
    }
    let now = Utc::now().to_rfc3339();
    let conn = state.db.lock();
    let n = conn
        .execute(
            "UPDATE chats SET title = ?1, updated_at = ?2 WHERE id = ?3",
            params![t, now, chat_id],
        )
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("Chat not found.".into());
    }
    Ok(())
}

#[tauri::command]
pub fn list_messages(
    state: tauri::State<'_, AppState>,
    chat_id: String,
) -> Result<Vec<MessageDto>, String> {
    let conn = state.db.lock();
    let mut stmt = conn
        .prepare("SELECT id, chat_id, role, content, created_at FROM messages WHERE chat_id = ?1 ORDER BY created_at ASC")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![chat_id], |row| {
            Ok(MessageDto {
                id: row.get(0)?,
                chat_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                created_at: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub async fn load_llm(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    model_id: String,
) -> Result<LoadedDto, String> {
    #[cfg(feature = "llama-sidecar")]
    {
        let row = llama_runtime::select_model(app, state.inner().clone(), model_id.clone()).await?;
        let conn = state.db.lock();
        Ok(loaded_dto_for_model(
            &conn,
            row.id,
            row.name,
            row.path.clone(),
            crate::mmproj_detect::auto_discover_mmproj(Path::new(&row.path))
                .map(|x| x.to_string_lossy().into_owned()),
        ))
    }

    #[cfg(not(feature = "llama-sidecar"))]
    {
        let _ = app;
        let (path, name, weights_format) = {
            let conn = state.db.lock();
            conn.query_row(
                "SELECT path, name, COALESCE(weights_format, 'gguf') FROM models WHERE id = ?1",
                params![model_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(|e| e.to_string())?
        };
        if weights_format == "cloud" {
            *state.loaded.lock() = None;
            let conn = state.db.lock();
            db::set_setting(&conn, "loaded_model_id", &model_id).map_err(|e| e.to_string())?;
            return Ok(loaded_dto_for_model(&conn, model_id, name, path, None));
        }
        if weights_format != "gguf" {
            return Err(
                "Only GGUF rows can load in chat. This entry is safetensors (or other) on disk."
                    .into(),
            );
        }
        let p = PathBuf::from(&path);
        if !p.exists() {
            return Err(format!("Model file missing: {}", p.display()));
        }
        let n_gpu = {
            let conn = state.db.lock();
            parse_u32_setting(&conn, "n_gpu_layers", 0)
        };
        #[cfg(feature = "llama-engine")]
        {
            let backend = state.backend.clone();
            let mid = model_id.clone();
            let nam = name.clone();
            let pp = p.clone();
            let loaded = tokio::task::spawn_blocking(move || {
                engine::load_model_file(&backend, mid, nam, pp, n_gpu)
            })
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
            *state.loaded.lock() = Some(loaded);
        }
        #[cfg(not(feature = "llama-engine"))]
        {
            let loaded =
                engine::load_model_file(model_id.clone(), name.clone(), p.clone(), n_gpu).map_err(|e| e.to_string())?;
            *state.loaded.lock() = Some(loaded);
        }
        let conn = state.db.lock();
        db::set_setting(&conn, "loaded_model_id", &model_id).map_err(|e| e.to_string())?;
        Ok(loaded_dto_for_model(
            &conn,
            model_id,
            name,
            p.to_string_lossy().into_owned(),
            crate::mmproj_detect::auto_discover_mmproj(p.as_path()).map(|x| x.to_string_lossy().into_owned()),
        ))
    }
}

#[derive(Clone)]
struct BenchmarkPrompt {
    id: &'static str,
    title: &'static str,
    category: &'static str,
    prompt: &'static str,
}

fn benchmark_prompts() -> Vec<BenchmarkPrompt> {
    vec![
        BenchmarkPrompt {
            id: "latency",
            title: "Latency",
            category: "latency",
            prompt: "Reply with one concise sentence explaining what a benchmark measures.",
        },
        BenchmarkPrompt {
            id: "reasoning",
            title: "Reasoning",
            category: "reasoning",
            prompt: "Solve this step by step, then give the final answer: A train leaves at 09:00 traveling 60 km/h. Another leaves the same station at 10:30 traveling 90 km/h on the same route. When does the second train catch the first?",
        },
        BenchmarkPrompt {
            id: "coding",
            title: "Coding",
            category: "coding",
            prompt: "Write a TypeScript function `groupBy<T>(items: T[], key: (item: T) => string): Record<string, T[]>` and include one short usage example.",
        },
    ]
}

fn estimate_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    ((chars as f64) / 4.0).ceil().max(1.0) as usize
}

fn avg_f64(values: impl Iterator<Item = f64>) -> f64 {
    let mut total = 0.0;
    let mut count = 0usize;
    for v in values {
        if v.is_finite() {
            total += v;
            count += 1;
        }
    }
    if count == 0 { 0.0 } else { total / count as f64 }
}

async fn complete_benchmark_prompt(
    state: &AppState,
    model_id: &str,
    weights_format: &str,
    cloud_provider: Option<&str>,
    prompt: &str,
) -> Result<String, String> {
    let api_msgs = vec![(
        "user".to_string(),
        Value::String(prompt.to_string()),
    )];
    let cancel = state.cancel.clone();
    cancel.store(false, Ordering::SeqCst);

    if weights_format == "cloud" {
        let slug = cloud_provider
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| model_id.strip_prefix("lm-cloud-"))
            .ok_or_else(|| "Cloud provider is missing for this model.".to_string())?;
        let setting_key = match slug {
            "openai" => "cloud_openai",
            "anthropic" => "cloud_anthropic",
            "openrouter" => "cloud_openrouter",
            "custom" => "cloud_custom",
            _ => return Err(format!("Unknown cloud provider: {slug}")),
        };
        let raw = {
            let conn = state.db.lock();
            db::get_setting(&conn, setting_key)
                .map_err(|e| e.to_string())?
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| "Cloud provider is not configured.".to_string())?
        };
        let cfg: cloud_infer::CloudProviderStored =
            serde_json::from_str(&raw).map_err(|e| format!("Invalid cloud config: {e}"))?;
        cloud_infer::complete_by_provider_slug(
            slug,
            &cfg,
            &api_msgs,
            0.2,
            384,
            &cancel,
        )
        .await
    } else {
        #[cfg(feature = "llama-sidecar")]
        {
            let base = {
                let g = state.sidecar.lock();
                let s = g
                    .as_ref()
                    .ok_or_else(|| "Local inference server is not running.".to_string())?;
                format!("http://127.0.0.1:{}", s.port)
            };
            let params = GenerationParams {
                n_ctx: 4096,
                n_threads: 0,
                n_threads_batch: 0,
                n_gpu_layers: 0,
                temperature: 0.2,
                top_p: 0.9,
                max_tokens: 384,
                seed: 1234,
            };
            crate::llama_sidecar::complete_chat_completion(
                &base,
                &api_msgs,
                params.temperature,
                params.top_p,
                params.max_tokens,
                false,
                &cancel,
            )
            .await
        }
        #[cfg(not(feature = "llama-sidecar"))]
        {
            Err("Local benchmark requires llama-sidecar.".into())
        }
    }
}

#[tauri::command]
pub async fn run_model_benchmark(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    model_ids: Vec<String>,
) -> Result<BenchmarkRunDto, String> {
    let ids: Vec<String> = model_ids
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .take(2)
        .collect();
    if ids.is_empty() {
        return Err("Pick at least one model to benchmark.".into());
    }

    let mut out = Vec::new();
    for model_id in ids {
        let (name, weights_format, cloud_provider) = {
            let conn = state.db.lock();
            conn.query_row(
                "SELECT name, COALESCE(weights_format,'gguf'), cloud_provider FROM models WHERE id = ?1",
                params![model_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?)),
            )
            .map_err(|_| format!("Model not found: {model_id}"))?
        };
        if weights_format == "safetensors" {
            return Err(format!("{name} is safetensors and cannot be benchmarked directly."));
        }

        let ram_before = system_metrics::capture_system_snapshot().ram_used_gb;
        let model_start = Instant::now();
        let load_start = Instant::now();
        if weights_format == "gguf" {
            load_llm(app.clone(), state.clone(), model_id.clone()).await?;
        }
        let load_ms = load_start.elapsed().as_millis();

        let mut prompt_results = Vec::new();
        for bp in benchmark_prompts() {
            let prompt_start = Instant::now();
            let result = complete_benchmark_prompt(
                &state,
                &model_id,
                &weights_format,
                cloud_provider.as_deref(),
                bp.prompt,
            )
            .await;
            let latency_ms = prompt_start.elapsed().as_millis();
            let (output, error) = match result {
                Ok(text) => (text, None),
                Err(e) => (String::new(), Some(e)),
            };
            let chars = output.chars().count();
            let estimated_tokens = if output.trim().is_empty() {
                0
            } else {
                estimate_tokens(&output)
            };
            let elapsed_secs = (latency_ms as f64 / 1000.0).max(0.001);
            let tokens_per_second = estimated_tokens as f64 / elapsed_secs;
            prompt_results.push(BenchmarkPromptResultDto {
                id: bp.id.to_string(),
                title: bp.title.to_string(),
                category: bp.category.to_string(),
                prompt: bp.prompt.to_string(),
                output,
                error,
                latency_ms,
                chars,
                estimated_tokens,
                tokens_per_second,
            });
        }

        let ram_after = system_metrics::capture_system_snapshot().ram_used_gb;
        let successful = prompt_results.iter().filter(|p| p.error.is_none());
        let avg_latency_ms = avg_f64(successful.clone().map(|p| p.latency_ms as f64));
        let avg_tokens_per_second = avg_f64(
            prompt_results
                .iter()
                .filter(|p| p.error.is_none())
                .map(|p| p.tokens_per_second),
        );
        let total_estimated_tokens = prompt_results
            .iter()
            .map(|p| p.estimated_tokens)
            .sum::<usize>();

        out.push(BenchmarkModelResultDto {
            model_id,
            model_name: name,
            weights_format,
            load_ms,
            total_ms: model_start.elapsed().as_millis(),
            ram_before_gb: ram_before,
            ram_after_gb: ram_after,
            ram_delta_gb: ram_after - ram_before,
            avg_latency_ms,
            avg_tokens_per_second,
            total_estimated_tokens,
            prompts: prompt_results,
        });
    }

    Ok(BenchmarkRunDto {
        run_id: Uuid::new_v4().to_string(),
        created_at: Utc::now().to_rfc3339(),
        models: out,
    })
}

#[tauri::command]
pub async fn unload_llm(state: tauri::State<'_, AppState>) -> Result<(), String> {
    #[cfg(feature = "llama-sidecar")]
    {
        crate::llama_sidecar::kill_sidecar_slot_async(&state).await?;
    }
    #[cfg(not(feature = "llama-sidecar"))]
    {
        *state.loaded.lock() = None;
    }
    let conn = state.db.lock();
    db::set_setting(&conn, "loaded_model_id", "").map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_loaded_llm(state: tauri::State<'_, AppState>) -> Result<Option<LoadedDto>, String> {
    if let Some(m) = state.loaded.lock().as_ref() {
        let conn = state.db.lock();
        return Ok(Some(loaded_dto_from_slot(
            m,
            &model_kind_for_id(&conn, &m.id),
        )));
    }
    let conn = state.db.lock();
    let loaded_id = db::get_setting(&conn, "loaded_model_id")
        .map_err(|e| e.to_string())?
        .filter(|s| !s.trim().is_empty());
    let Some(lid) = loaded_id else {
        return Ok(None);
    };
    let row: Option<(String, String, String)> = conn
        .query_row(
            "SELECT name, path, COALESCE(weights_format,'gguf') FROM models WHERE id = ?1",
            params![lid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((name, path, wf)) = row else {
        return Ok(None);
    };
    if wf == "cloud" {
        return Ok(Some(loaded_dto_for_model(&conn, lid, name, path, None)));
    }
    Ok(None)
}

#[tauri::command]
pub fn stop_generation(state: tauri::State<'_, AppState>) {
    state.cancel.store(true, Ordering::SeqCst);
}

fn user_title_snippet(raw: &str) -> String {
    let t = raw.trim_start();
    if t.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) {
            if v.get("_lm").and_then(|x| x.as_u64()) == Some(1) {
                if let Some(d) = v.get("displayText").and_then(|x| x.as_str()) {
                    return d.chars().take(48).collect();
                }
                if let Some(d) = v.get("display").and_then(|x| x.as_str()) {
                    return d.chars().take(48).collect();
                }
            }
        }
    }
    raw.chars().take(48).collect()
}

/// OpenAI `content`: string or array of content parts.
fn db_content_to_api_content(role: &str, raw: &str) -> Result<Value, String> {
    if role == "assistant" {
        return Ok(Value::String(raw.to_string()));
    }
    if role == "user" {
        let t = raw.trim_start();
        if t.starts_with('{') {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) {
                if v.get("_lm").and_then(|x| x.as_u64()) == Some(1) {
                    if let Some(parts) = v.get("parts") {
                        return Ok(parts.clone());
                    }
                    if let Some(d) = v.get("displayText").and_then(|x| x.as_str()) {
                        return Ok(Value::String(d.to_string()));
                    }
                    if let Some(d) = v.get("display").and_then(|x| x.as_str()) {
                        return Ok(Value::String(d.to_string()));
                    }
                }
            }
        }
    }
    Ok(Value::String(raw.to_string()))
}

const MAX_CONTEXT_BUNDLE_CHARS: usize = 100_000;
const MAX_CONTEXT_PER_DOC_CHARS: usize = 60_000;
const MAX_PROJECT_CONTEXT_CHARS: usize = 80_000;

fn chars_to_tokens_estimate(chars: usize) -> u32 {
    ((chars as f64) / 4.0).ceil().max(1.0) as u32
}

fn content_char_len(role: &str, raw: &str) -> Result<usize, String> {
    let api_c = db_content_to_api_content(role, raw)?;
    let flat = flatten_api_content_for_engine(&api_c)?;
    Ok(flat.chars().count())
}

fn project_context_for_chat(conn: &rusqlite::Connection, chat_id: &str) -> Result<String, String> {
    let pid: Option<String> = conn
        .query_row(
            "SELECT project_id FROM chats WHERE id = ?1",
            params![chat_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .flatten();
    let Some(pid) = pid.filter(|s| !s.trim().is_empty()) else {
        return Ok(String::new());
    };
    let ctx: String = conn
        .query_row(
            "SELECT COALESCE(context, '') FROM projects WHERE id = ?1",
            params![pid],
            |r| r.get(0),
        )
        .unwrap_or_default();
    let mut text = ctx.trim().to_string();
    if text.chars().count() > MAX_PROJECT_CONTEXT_CHARS {
        text = text.chars().take(MAX_PROJECT_CONTEXT_CHARS).collect();
        text.push_str("\n… [project context truncated]\n");
    }
    Ok(text)
}

fn estimate_chat_prompt_chars(
    conn: &rusqlite::Connection,
    chat_id: &str,
    draft: Option<&str>,
    context_document_ids: &[String],
) -> Result<usize, String> {
    let user_system: String = conn
        .query_row(
            "SELECT COALESCE(system_prompt, '') FROM chats WHERE id = ?1",
            params![chat_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let mut total = user_system.trim().chars().count();
    let project_ctx = project_context_for_chat(conn, chat_id)?;
    if !project_ctx.is_empty() {
        total += project_ctx.chars().count() + 80;
    }
    if let Some(ref raw) = build_context_documents_bundle(conn, context_document_ids)? {
        total += raw.chars().count() + 120;
    }
    let mut stmt = conn
        .prepare("SELECT role, content FROM messages WHERE chat_id = ?1 ORDER BY created_at ASC")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![chat_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;
    for r in rows {
        let (role, content) = r.map_err(|e| e.to_string())?;
        total += content_char_len(&role, &content)?;
    }
    if let Some(d) = draft {
        let t = d.trim();
        if !t.is_empty() {
            total += t.chars().count();
        }
    }
    Ok(total)
}

#[tauri::command]
pub fn get_chat_context_usage(
    state: tauri::State<'_, AppState>,
    chat_id: String,
    draft: Option<String>,
    context_document_ids: Option<Vec<String>>,
) -> Result<ChatContextUsageDto, String> {
    let conn = state.db.lock();
    let limit_tokens = parse_u32_setting(&conn, "n_ctx", 4096);
    let reserved = parse_i32_setting(&conn, "max_tokens", 768).max(1) as u32;
    let ids = context_document_ids.unwrap_or_default();
    let chars = estimate_chat_prompt_chars(&conn, &chat_id, draft.as_deref(), &ids)?;
    let used_tokens = chars_to_tokens_estimate(chars);
    let budget = limit_tokens.saturating_sub(reserved);
    let remaining = budget.saturating_sub(used_tokens);
    let used_percent = if limit_tokens == 0 {
        0.0
    } else {
        ((used_tokens + reserved) as f32 / limit_tokens as f32 * 100.0).clamp(0.0, 100.0)
    };
    Ok(ChatContextUsageDto {
        used_tokens,
        limit_tokens,
        remaining_tokens: remaining,
        reserved_output_tokens: reserved,
        used_percent,
    })
}

/// Load text from context library rows for RAG-style injection into the model prompt.
fn build_context_documents_bundle(
    conn: &rusqlite::Connection,
    ids: &[String],
) -> Result<Option<String>, String> {
    use std::collections::HashSet;
    if ids.is_empty() {
        return Ok(None);
    }
    let mut seen = HashSet::<&str>::new();
    let mut bundle = String::new();
    for id in ids {
        if id.is_empty() || !seen.insert(id.as_str()) {
            continue;
        }
        let row: Result<(String, String), rusqlite::Error> = conn.query_row(
            "SELECT name, stored_path FROM context_documents WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        );
        let (name, path_s) = match row {
            Ok(x) => x,
            Err(rusqlite::Error::QueryReturnedNoRows) => continue,
            Err(e) => return Err(e.to_string()),
        };
        let path = Path::new(&path_s);
        if !path.is_file() {
            continue;
        }
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let mut text = String::from_utf8_lossy(&bytes).to_string();
        text = text.replace('\x00', "");
        if text.chars().count() > MAX_CONTEXT_PER_DOC_CHARS {
            let truncated: String = text.chars().take(MAX_CONTEXT_PER_DOC_CHARS).collect();
            text = format!("{truncated}\n… [truncated]\n");
        }
        let block = format!("=== Reference: {name} (id: {id}) ===\n{text}\n\n");
        if bundle.len() + block.len() > MAX_CONTEXT_BUNDLE_CHARS {
            bundle.push_str("\n… [Additional reference files omitted: size cap]\n");
            break;
        }
        bundle.push_str(&block);
    }
    if bundle.is_empty() {
        Ok(None)
    } else {
        Ok(Some(bundle))
    }
}

fn flatten_api_content_for_engine(v: &Value) -> Result<String, String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Array(parts) => {
            let mut out = String::new();
            for p in parts {
                let Some(obj) = p.as_object() else { continue };
                let typ = obj.get("type").and_then(|x| x.as_str()).unwrap_or("");
                match typ {
                    "text" => {
                        if let Some(t) = obj.get("text").and_then(|x| x.as_str()) {
                            out.push_str(t);
                        }
                    }
                    "image_url" => {
                        out.push_str("\n[Image — vision/mmproj models need the llama-server backend; image omitted in in-process engine]\n");
                    }
                    _ => out.push_str("\n[attachment]\n"),
                }
            }
            Ok(out)
        }
        _ => Err("invalid message content shape".into()),
    }
}

fn selected_model_id_for_chat(
    conn: &rusqlite::Connection,
    chat_id: &str,
    header_model_id: Option<&str>,
) -> Result<String, String> {
    let mid: Option<String> = conn
        .query_row(
            "SELECT model_id FROM chats WHERE id = ?1",
            params![chat_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .flatten()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            header_model_id
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
        });
    mid.ok_or_else(|| "Pick a model for this chat (header dropdown).".to_string())
}

async fn stream_assistant_for_chat(
    _app: &tauri::AppHandle,
    state: &AppState,
    chat_id: &str,
    header_model_id: Option<&str>,
    thinking_enabled: bool,
    web_search_enabled: bool,
    agent_enabled: bool,
    image_generation_enabled: bool,
    context_document_ids: &[String],
    on_token: Channel<String>,
) -> Result<String, String> {
    let (cloud_plan, api_msgs, engine_pairs, gen_params) = {
        let conn = state.db.lock();
        let mid = selected_model_id_for_chat(&conn, chat_id, header_model_id)?;
        let model_row = conn
            .query_row(
                "SELECT COALESCE(weights_format,'gguf'), COALESCE(cloud_provider,''), COALESCE(cloud_api_model,'') FROM models WHERE id = ?1",
                params![mid],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| {
                "Selected model was not found. Choose another model in the header.".to_string()
            })?;
        let (wf, prov, _cloud_api_m) = model_row;

        let user_system: String = conn
            .query_row(
                "SELECT COALESCE(system_prompt, '') FROM chats WHERE id = ?1",
                params![chat_id],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let ctx_block = build_context_documents_bundle(&conn, context_document_ids)?;
        let mut stmt = conn
            .prepare("SELECT role, content FROM messages WHERE chat_id = ?1 ORDER BY created_at ASC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![chat_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut api_msgs: Vec<(String, Value)> = Vec::new();
        let mut engine_pairs: Vec<(String, String)> = Vec::new();
        let utrim = user_system.trim();
        if !utrim.is_empty() {
            api_msgs.push(("system".into(), Value::String(utrim.to_string())));
            engine_pairs.push(("system".into(), utrim.to_string()));
        }
        let project_ctx = project_context_for_chat(&conn, chat_id)?;
        if !project_ctx.is_empty() {
            let sys = format!(
                "Project context (shared across chats in this project). \
Use it when it helps answer; prefer quoting or paraphrasing from it for facts.\n\n{project_ctx}"
            );
            api_msgs.push(("system".into(), Value::String(sys.clone())));
            engine_pairs.push(("system".into(), sys));
        }
        if let Some(ref raw) = ctx_block {
            if !raw.is_empty() {
                let sys = format!(
                    "The user attached reference material (Context library, via @mentions). \
Use it when it helps answer; prefer quoting or paraphrasing from it for facts.\n\n{raw}"
                );
                api_msgs.push(("system".into(), Value::String(sys.clone())));
                engine_pairs.push(("system".into(), sys));
            }
        }
        for r in rows {
            let (role, content): (String, String) = r.map_err(|e| e.to_string())?;
            let api_c = db_content_to_api_content(&role, &content)?;
            api_msgs.push((role.clone(), api_c.clone()));
            engine_pairs.push((role, flatten_api_content_for_engine(&api_c)?));
        }
        let gen_params = GenerationParams {
            n_ctx: parse_u32_setting(&conn, "n_ctx", 4096),
            n_threads: parse_u32_setting(&conn, "n_threads", 0),
            n_threads_batch: 0,
            n_gpu_layers: parse_u32_setting(&conn, "n_gpu_layers", 0),
            temperature: parse_f32_setting(&conn, "temperature", 0.7),
            top_p: parse_f32_setting(&conn, "top_p", 0.9),
            max_tokens: parse_i32_setting(&conn, "max_tokens", 768),
            seed: parse_u32_setting(&conn, "seed", 1234),
        };
        let cloud_plan = if wf == "cloud" {
            let sk = match prov.trim() {
                "openai" => "cloud_openai",
                "anthropic" => "cloud_anthropic",
                "openrouter" => "cloud_openrouter",
                "custom" => "cloud_custom",
                _ => {
                    return Err(format!("Unknown cloud provider: {}", prov));
                }
            };
            let raw = db::get_setting(&conn, sk)
                .map_err(|e| e.to_string())?
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| {
                    "Cloud provider is not configured. Open Models → Cloud providers.".to_string()
                })?;
            let cfg: cloud_infer::CloudProviderStored =
                serde_json::from_str(&raw).map_err(|e| format!("Invalid cloud config: {e}"))?;
            if cfg.model.trim().is_empty() {
                return Err(
                    "Cloud model id is missing. Open Models → Cloud providers.".into(),
                );
            }
            if prov.trim() != "custom" && cfg.api_key.trim().is_empty() {
                return Err(
                    "Cloud API key is missing. Open Models → Cloud providers.".into(),
                );
            }
            if prov.trim() == "custom" {
                let base = cfg.base_url.as_deref().unwrap_or("").trim();
                if base.is_empty() {
                    return Err(
                        "Custom provider base URL is missing. Open Models → Cloud providers.".into(),
                    );
                }
            }
            Some((prov, cfg))
        } else {
            None
        };
        (
            cloud_plan,
            api_msgs,
            engine_pairs,
            gen_params,
        )
    };

    #[cfg(feature = "llama-sidecar")]
    drop(engine_pairs);

    let on_token_blocking = on_token.clone();
    let cancel_arc = state.cancel.clone();

    let context_dir = {
        let conn = state.db.lock();
        resolved_context_dir(&conn, &state.app_data_dir)?
    };
    let image_gen_plan = {
        let conn = state.db.lock();
        let _ = db::sync_cloud_models_from_settings(&conn);
        crate::image_gen::resolve_for_chat_with_model(
            &conn,
            chat_id,
            header_model_id.map(str::trim).filter(|s| !s.is_empty()),
        )
    };
    if image_generation_enabled && image_gen_plan.is_none() {
        return Err(
            "Image generation is on but not configured. Enable it for your cloud provider under Models → Cloud."
                .into(),
        );
    }
    let tool_ctx = chat_tools::ToolContext {
        app_data_dir: &state.app_data_dir,
        models_dir: &state.models_dir,
        context_dir: &context_dir,
        image_gen: image_gen_plan,
    };
    let tool_opts = tool_agent::ToolAgentOptions {
        web_search: web_search_enabled,
        agent: agent_enabled,
        image_generation: image_generation_enabled,
        thinking_enabled,
        gen_params: gen_params.clone(),
        cancel: &cancel_arc,
        tool_ctx,
    };

    #[cfg(feature = "llama-sidecar")]
    let sidecar_url: Option<String> = if cloud_plan.is_none() {
        let g = state.sidecar.lock();
        let s = g.as_ref().ok_or_else(|| {
            "Inference server is not running. Wait for the model to finish loading, or pick a model in the header."
                .to_string()
        })?;
        Some(format!("http://127.0.0.1:{}", s.port))
    } else {
        None
    };

    if web_search_enabled || agent_enabled || image_generation_enabled {
        return tool_agent::run_with_tools(
            api_msgs,
            cloud_plan,
            sidecar_url,
            tool_opts,
            on_token,
        )
        .await;
    }

    if let Some((prov, cfg)) = cloud_plan {
        let _ = thinking_enabled;
        return cloud_infer::stream_by_provider_slug(
            prov.trim(),
            &cfg,
            &api_msgs,
            gen_params.temperature,
            gen_params.max_tokens,
            &cancel_arc,
            |line| on_token_blocking.send(line).map_err(|e| e.to_string()),
        )
        .await;
    }

    #[cfg(feature = "llama-sidecar")]
    {
        let base_url = sidecar_url.ok_or_else(|| {
            "Inference server is not running. Wait for the model to finish loading, or pick a model in the header."
                .to_string()
        })?;
        let send = |part: crate::llama_sidecar::StreamPart, t: &str| {
            let tag = match part {
                crate::llama_sidecar::StreamPart::Reasoning => "r",
                crate::llama_sidecar::StreamPart::Content => "c",
            };
            let line = serde_json::json!({ "t": tag, "s": t }).to_string();
            on_token_blocking.send(line).map_err(|e| e.to_string())
        };
        match crate::llama_sidecar::stream_chat_completion(
            &base_url,
            &api_msgs,
            gen_params.temperature,
            gen_params.top_p,
            gen_params.max_tokens,
            thinking_enabled,
            &cancel_arc,
            send,
        )
        .await
        {
            Ok(text) => Ok(text),
            Err(e) => {
                let _ = on_token.send(format!("\n\n*Error:* {e}"));
                Err(e)
            }
        }
    }
    #[cfg(all(feature = "llama-engine", not(feature = "llama-sidecar")))]
    {
        let loaded_arc = state.loaded.clone();
        let backend_arc = state.backend.clone();
        let join = tokio::task::spawn_blocking(move || {
            engine::with_loaded(loaded_arc.as_ref(), |loaded| {
                engine::generate_chat_reply(
                    loaded,
                    &backend_arc,
                    &engine_pairs,
                    gen_params,
                    &cancel_arc,
                    |t| {
                        let line = serde_json::json!({ "t": "c", "s": t }).to_string();
                        on_token_blocking.send(line).map_err(|e| e.to_string())
                    },
                )
            })
            .map_err(|e| e.to_string())
        });
        match join.await {
            Ok(Ok(text)) => Ok(text),
            Ok(Err(e)) => {
                let _ = on_token.send(format!("\n\n*Error:* {e}"));
                Err(e)
            }
            Err(e) => Err(e.to_string()),
        }
    }
    #[cfg(all(not(feature = "llama-engine"), not(feature = "llama-sidecar")))]
    {
        let loaded_arc = state.loaded.clone();
        let join = tokio::task::spawn_blocking(move || {
            engine::with_loaded(loaded_arc.as_ref(), |loaded| {
                engine::generate_chat_reply(
                    loaded,
                    &(),
                    &engine_pairs,
                    gen_params,
                    &cancel_arc,
                    |t| {
                        let line = serde_json::json!({ "t": "c", "s": t }).to_string();
                        on_token_blocking.send(line).map_err(|e| e.to_string())
                    },
                )
            })
            .map_err(|e| e.to_string())
        });
        match join.await {
            Ok(Ok(text)) => Ok(text),
            Ok(Err(e)) => {
                let _ = on_token.send(format!("\n\n*Error:* {e}"));
                Err(e)
            }
            Err(e) => Err(e.to_string()),
        }
    }
}

fn chat_uses_cloud_llm(conn: &rusqlite::Connection, chat_id: &str) -> Result<bool, String> {
    let mid: Option<String> = conn
        .query_row(
            "SELECT model_id FROM chats WHERE id = ?1",
            params![chat_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .flatten();
    let Some(mid) = mid.filter(|s| !s.trim().is_empty()) else {
        return Ok(false);
    };
    let wf: Option<String> = conn
        .query_row(
            "SELECT COALESCE(weights_format,'gguf') FROM models WHERE id = ?1",
            params![mid],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(matches!(wf.as_deref(), Some("cloud")))
}

fn skip_llama_sidecar_for_chat(
    conn: &rusqlite::Connection,
    chat_id: &str,
    _header_model_id: Option<&str>,
) -> Result<bool, String> {
    chat_uses_cloud_llm(conn, chat_id)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendResult {
    pub assistant_message_id: String,
    pub cancelled: bool,
}

#[tauri::command]
pub async fn send_chat_message(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    on_token: Channel<String>,
    chat_id: String,
    content: String,
    thinking_enabled: bool,
    web_search_enabled: bool,
    agent_enabled: bool,
    image_generation_enabled: bool,
    context_document_ids: Vec<String>,
    header_model_id: Option<String>,
) -> Result<SendResult, String> {
    state.cancel.store(false, Ordering::SeqCst);
    let user_msg_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let content_trim = content.trim().to_string();
    if content_trim.is_empty() {
        return Err("Message is empty".into());
    }

    {
        let conn = state.db.lock();
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chats WHERE id = ?1",
                params![chat_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if exists == 0 {
            return Err("Chat not found".into());
        }
        conn.execute(
            "INSERT INTO messages (id, chat_id, role, content, created_at) VALUES (?1, ?2, 'user', ?3, ?4)",
            params![user_msg_id, chat_id, content_trim, now],
        )
        .map_err(|e| e.to_string())?;
        let title: String = conn
            .query_row(
                "SELECT title FROM chats WHERE id = ?1",
                params![chat_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if title == "New chat" {
            let snippet = user_title_snippet(&content_trim);
            conn.execute(
                "UPDATE chats SET title = ?1, updated_at = ?2 WHERE id = ?3",
                params![snippet, now, chat_id],
            )
            .map_err(|e| e.to_string())?;
        } else {
            conn.execute(
                "UPDATE chats SET updated_at = ?1 WHERE id = ?2",
                params![now, chat_id],
            )
            .map_err(|e| e.to_string())?;
        }
    }

    // Frontend drops the optimistic "sending…" bubble once this is received, so user and
    // assistant states are not shown at the same time during streaming.
    let _ = on_token.send("LMPHASE:user_saved".into());

    #[cfg(feature = "llama-sidecar")]
    let local_runtime_model_id: Option<String> = if {
        let conn = state.db.lock();
        !skip_llama_sidecar_for_chat(&conn, &chat_id, header_model_id.as_deref())?
    } {
        let mid = {
            let conn = state.db.lock();
            selected_model_id_for_chat(&conn, &chat_id, header_model_id.as_deref())?
        };
        llama_runtime::ensure_ready(&app, &state, &mid).await?;
        Some(mid)
    } else {
        None
    };

    #[cfg(feature = "llama-sidecar")]
    let assistant_body = if let Some(mid) = local_runtime_model_id {
        llama_runtime::with_generation(&state, &mid, stream_assistant_for_chat(
            &app,
            &state,
            &chat_id,
            header_model_id.as_deref(),
            thinking_enabled,
            web_search_enabled,
            agent_enabled,
            image_generation_enabled,
            &context_document_ids,
            on_token.clone(),
        ))
        .await?
    } else {
        stream_assistant_for_chat(
            &app,
            &state,
            &chat_id,
            header_model_id.as_deref(),
            thinking_enabled,
            web_search_enabled,
            agent_enabled,
            image_generation_enabled,
            &context_document_ids,
            on_token.clone(),
        )
        .await?
    };

    #[cfg(not(feature = "llama-sidecar"))]
    let assistant_body = stream_assistant_for_chat(
        &app,
        &state,
        &chat_id,
        header_model_id.as_deref(),
        thinking_enabled,
        web_search_enabled,
        agent_enabled,
        image_generation_enabled,
        &context_document_ids,
        on_token.clone(),
    )
    .await?;

    let cancelled = state.cancel.load(Ordering::SeqCst);
    let asst_id = Uuid::new_v4().to_string();
    let now2 = Utc::now().to_rfc3339();
    {
        let conn = state.db.lock();
        conn.execute(
            "INSERT INTO messages (id, chat_id, role, content, created_at) VALUES (?1, ?2, 'assistant', ?3, ?4)",
            params![asst_id, chat_id, assistant_body, now2],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE chats SET updated_at = ?1 WHERE id = ?2",
            params![now2, chat_id],
        )
        .map_err(|e| e.to_string())?;
    }

    Ok(SendResult {
        assistant_message_id: asst_id,
        cancelled,
    })
}

#[tauri::command]
pub async fn regenerate_assistant_message(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    on_token: Channel<String>,
    chat_id: String,
    assistant_message_id: String,
    thinking_enabled: bool,
    web_search_enabled: bool,
    agent_enabled: bool,
    image_generation_enabled: bool,
    header_model_id: Option<String>,
) -> Result<SendResult, String> {
    state.cancel.store(false, Ordering::SeqCst);
    let role: String = {
        let conn = state.db.lock();
        conn.query_row(
            "SELECT role FROM messages WHERE id = ?1 AND chat_id = ?2",
            params![assistant_message_id, chat_id],
            |r| r.get(0),
        )
        .map_err(|_| "Message not found.".to_string())?
    };
    if role != "assistant" {
        return Err("Only assistant replies can be regenerated.".into());
    }
    {
        let conn = state.db.lock();
        conn.execute(
            "DELETE FROM messages WHERE id = ?1",
            params![assistant_message_id],
        )
        .map_err(|e| e.to_string())?;
    }

    #[cfg(feature = "llama-sidecar")]
    let local_runtime_model_id: Option<String> = if {
        let conn = state.db.lock();
        !skip_llama_sidecar_for_chat(&conn, &chat_id, header_model_id.as_deref())?
    } {
        let mid = {
            let conn = state.db.lock();
            selected_model_id_for_chat(&conn, &chat_id, header_model_id.as_deref())?
        };
        llama_runtime::ensure_ready(&app, &state, &mid).await?;
        Some(mid)
    } else {
        None
    };

    #[cfg(feature = "llama-sidecar")]
    let assistant_body = if let Some(mid) = local_runtime_model_id {
        llama_runtime::with_generation(&state, &mid, stream_assistant_for_chat(
            &app,
            &state,
            &chat_id,
            header_model_id.as_deref(),
            thinking_enabled,
            web_search_enabled,
            agent_enabled,
            image_generation_enabled,
            &[],
            on_token.clone(),
        ))
        .await?
    } else {
        stream_assistant_for_chat(
            &app,
            &state,
            &chat_id,
            header_model_id.as_deref(),
            thinking_enabled,
            web_search_enabled,
            agent_enabled,
            image_generation_enabled,
            &[],
            on_token.clone(),
        )
        .await?
    };

    #[cfg(not(feature = "llama-sidecar"))]
    let assistant_body = stream_assistant_for_chat(
        &app,
        &state,
        &chat_id,
        header_model_id.as_deref(),
        thinking_enabled,
        web_search_enabled,
        agent_enabled,
        image_generation_enabled,
        &[],
        on_token.clone(),
    )
    .await?;

    let cancelled = state.cancel.load(Ordering::SeqCst);
    let asst_id = Uuid::new_v4().to_string();
    let now2 = Utc::now().to_rfc3339();
    {
        let conn = state.db.lock();
        conn.execute(
            "INSERT INTO messages (id, chat_id, role, content, created_at) VALUES (?1, ?2, 'assistant', ?3, ?4)",
            params![asst_id, chat_id, assistant_body, now2],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE chats SET updated_at = ?1 WHERE id = ?2",
            params![now2, chat_id],
        )
        .map_err(|e| e.to_string())?;
    }

    Ok(SendResult {
        assistant_message_id: asst_id,
        cancelled,
    })
}

#[tauri::command]
pub fn delete_message(state: tauri::State<'_, AppState>, message_id: String) -> Result<(), String> {
    let now = Utc::now().to_rfc3339();
    let conn = state.db.lock();
    let (chat_id, created_at): (String, String) = conn
        .query_row(
            "SELECT chat_id, created_at FROM messages WHERE id = ?1",
            params![message_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| "Message not found.".to_string())?;
    conn.execute(
        "DELETE FROM messages WHERE chat_id = ?1 AND created_at >= ?2",
        params![chat_id, created_at],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE chats SET updated_at = ?1 WHERE id = ?2",
        params![now, chat_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn update_message(
    state: tauri::State<'_, AppState>,
    message_id: String,
    content: String,
) -> Result<(), String> {
    let trimmed = content.trim().to_string();
    if trimmed.is_empty() {
        return Err("Message cannot be empty.".into());
    }
    let now = Utc::now().to_rfc3339();
    let conn = state.db.lock();
    let (chat_id, created_at): (String, String) = conn
        .query_row(
            "SELECT chat_id, created_at FROM messages WHERE id = ?1",
            params![message_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| "Message not found.".to_string())?;
    conn.execute(
        "UPDATE messages SET content = ?1 WHERE id = ?2",
        params![trimmed, message_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM messages WHERE chat_id = ?1 AND created_at > ?2",
        params![chat_id, created_at],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE chats SET updated_at = ?1 WHERE id = ?2",
        params![now, chat_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextDocDto {
    pub id: String,
    pub name: String,
    pub source: String,
    pub kind: String,
    pub stored_path: String,
    pub size_bytes: Option<i64>,
    pub chunks: i64,
    pub status: String,
    pub created_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextTextEditDto {
    pub id: String,
    pub name: String,
    pub description: String,
    pub content: String,
}

fn split_stored_context_body(raw: &str) -> (String, String) {
    if let Some(pos) = raw.find("\n\n") {
        (raw[..pos].to_string(), raw[pos + 2..].to_string())
    } else {
        (String::new(), raw.to_string())
    }
}

#[tauri::command]
pub fn list_context_documents(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ContextDocDto>, String> {
    let conn = state.db.lock();
    let mut stmt = conn
        .prepare("SELECT id, name, source, kind, stored_path, size_bytes, chunks, status, created_at FROM context_documents ORDER BY created_at DESC")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ContextDocDto {
                id: row.get(0)?,
                name: row.get(1)?,
                source: row.get(2)?,
                kind: row.get(3)?,
                stored_path: row.get(4)?,
                size_bytes: row.get(5)?,
                chunks: row.get(6)?,
                status: row.get(7)?,
                created_at: row.get(8)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn get_context_text_for_edit(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<ContextTextEditDto, String> {
    let conn = state.db.lock();
    let (name, source, stored_path): (String, String, String) = conn
        .query_row(
            "SELECT name, source, stored_path FROM context_documents WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| "Reference item not found.".to_string())?;
    if source != "text" {
        return Err("Only manually added text references can be edited here.".into());
    }
    let bytes = std::fs::read(Path::new(&stored_path)).map_err(|e| e.to_string())?;
    let raw = String::from_utf8_lossy(&bytes).to_string();
    let (description, content) = split_stored_context_body(&raw);
    Ok(ContextTextEditDto {
        id,
        name,
        description,
        content,
    })
}

#[tauri::command]
pub fn add_context_from_path(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<ContextDocDto, String> {
    let src = PathBuf::from(&path);
    if !src.is_file() {
        return Err("Not a file or file missing.".into());
    }
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let orig_name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let kind = src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin")
        .to_ascii_lowercase();
    let meta = std::fs::metadata(&src).map_err(|e| e.to_string())?;
    let size = meta.len() as i64;

    let dest_dir = {
        let conn = state.db.lock();
        resolved_context_dir(&conn, &state.app_data_dir)?
    };
    let dest_name = format!("{}_{}", id, sanitize_filename(&orig_name));
    let dest = dest_dir.join(&dest_name);
    std::fs::copy(&src, &dest).map_err(|e| e.to_string())?;

    let stored = dest.to_string_lossy().into_owned();
    {
        let conn = state.db.lock();
        conn.execute(
            "INSERT INTO context_documents (id, name, source, kind, stored_path, size_bytes, chunks, status, created_at) VALUES (?1, ?2, 'file', ?3, ?4, ?5, 0, 'ready', ?6)",
            params![id, orig_name.clone(), kind, stored, size, now],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(ContextDocDto {
        id,
        name: orig_name,
        source: "file".into(),
        kind,
        stored_path: dest.to_string_lossy().into_owned(),
        size_bytes: Some(size),
        chunks: 0,
        status: "ready".into(),
        created_at: now,
    })
}

#[tauri::command]
pub fn add_context_text(
    state: tauri::State<'_, AppState>,
    title: String,
    description: String,
    content: String,
) -> Result<ContextDocDto, String> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let c = content.trim();
    if c.is_empty() {
        return Err("Enter the main text for this item.".into());
    }
    let desc = description.trim();
    let mut body = String::new();
    if !desc.is_empty() {
        body.push_str(desc);
        body.push_str("\n\n");
    }
    body.push_str(c);

    let base = sanitize_filename(&title);
    let dest_dir = {
        let conn = state.db.lock();
        resolved_context_dir(&conn, &state.app_data_dir)?
    };
    let dest_name = format!("{}_{}.txt", id, base);
    let dest = dest_dir.join(&dest_name);
    std::fs::write(&dest, body.as_bytes()).map_err(|e| e.to_string())?;
    let meta = std::fs::metadata(&dest).map_err(|e| e.to_string())?;
    let size = meta.len() as i64;
    let stored = dest.to_string_lossy().into_owned();
    let display_name = {
        let t = title.trim();
        if t.is_empty() {
            format!("{base}.txt")
        } else {
            t.to_string()
        }
    };
    {
        let conn = state.db.lock();
        conn.execute(
            "INSERT INTO context_documents (id, name, source, kind, stored_path, size_bytes, chunks, status, created_at) VALUES (?1, ?2, 'text', 'txt', ?3, ?4, 0, 'ready', ?5)",
            params![id, display_name.clone(), stored, size, now],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(ContextDocDto {
        id,
        name: display_name,
        source: "text".into(),
        kind: "txt".into(),
        stored_path: dest.to_string_lossy().into_owned(),
        size_bytes: Some(size),
        chunks: 0,
        status: "ready".into(),
        created_at: now,
    })
}

#[tauri::command]
pub fn update_context_text(
    state: tauri::State<'_, AppState>,
    id: String,
    title: String,
    description: String,
    content: String,
) -> Result<ContextDocDto, String> {
    let c = content.trim();
    if c.is_empty() {
        return Err("Enter the main text for this item.".into());
    }
    let (stored_path, created_at): (String, String) = {
        let conn = state.db.lock();
        conn.query_row(
            "SELECT stored_path, created_at FROM context_documents WHERE id = ?1 AND source = 'text'",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| "Text reference not found or not editable.".to_string())?
    };
    let desc = description.trim();
    let mut body = String::new();
    if !desc.is_empty() {
        body.push_str(desc);
        body.push_str("\n\n");
    }
    body.push_str(c);
    std::fs::write(Path::new(&stored_path), body.as_bytes()).map_err(|e| e.to_string())?;
    let meta = std::fs::metadata(Path::new(&stored_path)).map_err(|e| e.to_string())?;
    let size = meta.len() as i64;
    let display_name = {
        let t = title.trim();
        let base = sanitize_filename(t);
        if t.is_empty() {
            format!("{base}.txt")
        } else {
            t.to_string()
        }
    };
    {
        let conn = state.db.lock();
        conn.execute(
            "UPDATE context_documents SET name = ?1, size_bytes = ?2, status = 'ready' WHERE id = ?3 AND source = 'text'",
            params![display_name.clone(), size, id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(ContextDocDto {
        id,
        name: display_name,
        source: "text".into(),
        kind: "txt".into(),
        stored_path,
        size_bytes: Some(size),
        chunks: 0,
        status: "ready".into(),
        created_at,
    })
}

#[tauri::command]
pub fn delete_context_document(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let path: String = {
        let conn = state.db.lock();
        match conn.query_row(
            "SELECT stored_path FROM context_documents WHERE id = ?1",
            params![id],
            |row| row.get(0),
        ) {
            Ok(p) => p,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Err("Context item not found.".into()),
            Err(e) => return Err(e.to_string()),
        }
    };
    let _ = std::fs::remove_file(Path::new(&path));
    let conn = state.db.lock();
    conn.execute(
        "DELETE FROM context_documents WHERE id = ?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn finish_hf_job(dm: &DownloadManager, job_id: &str, result: Result<String, String>) {
    match result {
        Ok(model_id) => dm.mark_completed(job_id, model_id),
        Err(e) if e == "Cancelled." => dm.mark_cancelled(job_id),
        Err(e) => dm.mark_failed(job_id, e),
    }
}

async fn run_hf_download_auto_job(
    dm: Arc<DownloadManager>,
    db: Arc<parking_lot::Mutex<rusqlite::Connection>>,
    models_dir_default: PathBuf,
    job_id: String,
    repo_input: String,
) {
    dm.set_running(&job_id);

    let repo_id = match huggingface::normalize_repo_id(repo_input.trim()) {
        Ok(r) => r,
        Err(e) => {
            dm.mark_failed(&job_id, e);
            return;
        }
    };
    dm.update_job(&job_id, |d| d.title = repo_id.clone());

    let token = huggingface::resolve_hf_token();
    let client = match huggingface::hf_client(token) {
        Ok(c) => c,
        Err(e) => {
            dm.mark_failed(&job_id, e);
            return;
        }
    };

    let models_dir_str = {
        let conn = db.lock();
        db::get_setting(&conn, "models_dir")
            .ok()
            .flatten()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| models_dir_default.to_string_lossy().into_owned())
    };
    let models_dir = PathBuf::from(&models_dir_str);
    if let Err(e) = std::fs::create_dir_all(&models_dir) {
        dm.mark_failed(&job_id, e.to_string());
        return;
    }

    dm.update_job(&job_id, |d| d.message = "Listing…".into());

    let (files, rev) = match huggingface::list_weights_resolved(&client, &repo_id, None).await {
        Ok(x) => x,
        Err(e) => {
            dm.mark_failed(&job_id, e);
            return;
        }
    };
    let plan = match huggingface::pick_auto_download_plan(&files) {
        Ok(p) => p,
        Err(e) => {
            dm.mark_failed(&job_id, e);
            return;
        }
    };

    let mut pause_cancel = {
        let dm = dm.clone();
        let jid = job_id.clone();
        move || dm.job_pause_and_cancel(&jid).unwrap_or((false, true))
    };

    let short_rev: String = rev.chars().take(10).collect();

    let register_out: Result<String, String> = match plan {
        huggingface::AutoDownloadPlan::Single {
            repo_path,
            kind,
        } => {
            let file_path_s = repo_path.as_str();
            let leaf = match Path::new(file_path_s)
                .file_name()
                .and_then(|n| n.to_str())
            {
                Some(l) => l,
                None => {
                    dm.mark_failed(&job_id, "Invalid file path.".into());
                    return;
                }
            };
            let dest = models_dir.join(leaf);
            if dest.exists() {
                dm.mark_failed(
                    &job_id,
                    format!("Already on disk: {}", dest.display()),
                );
                return;
            }
            dm.update_job(&job_id, |d| {
                d.file_count = 1;
                d.file_index = 1;
                d.message = format!("{} @ {}", leaf, short_rev);
            });
            dm.register_partial(&job_id, dest.clone());
            let file_label = leaf.to_string();
            let job_id_pb = job_id.clone();
            let dm_pb = dm.clone();
            let dl = huggingface::download_hf_file(
                &client,
                &repo_id,
                &rev,
                file_path_s,
                &dest,
                &mut pause_cancel,
                move |msg, bd, bt| {
                    let part = match bt {
                        Some(t) if t > 0 => ((bd as f64 / t as f64).min(1.0)) * 100.0,
                        _ => 0.0,
                    };
                    dm_pb.update_job(&job_id_pb, |d| {
                        d.message = msg.clone();
                        d.bytes_downloaded = bd;
                        d.bytes_total = bt;
                        d.progress = part.min(99.9);
                        d.file_index = 1;
                        d.file_count = 1;
                        d.current_file = Some(file_label.clone());
                    });
                },
            )
            .await;

            match dl {
                Ok(_) => {
                    dm.release_partial(&job_id, &dest);
                    let wf = match kind {
                        HfWeightKind::Gguf => "gguf",
                        HfWeightKind::Safetensors => "safetensors",
                    };
                    let conn = db.lock();
                    register_weights_inner(&conn, &dest, wf, None, None).map(|m| m.id)
                }
                Err(e) => Err(e),
            }
        }
        huggingface::AutoDownloadPlan::ShardedSafetensors { repo_paths } => {
            let n = repo_paths.len() as u32;
            dm.update_job(&job_id, |d| d.file_count = n);
            let mut stopped: Option<String> = None;
            for (i, rp) in repo_paths.iter().enumerate() {
                let idx = (i + 1) as u32;
                let leaf = match Path::new(rp).file_name().and_then(|x| x.to_str()) {
                    Some(l) => l,
                    None => {
                        stopped = Some("Invalid shard path.".into());
                        break;
                    }
                };
                let dest = models_dir.join(leaf);
                if dest.exists() {
                    stopped = Some(format!("Already on disk: {}", dest.display()));
                    break;
                }
                dm.register_partial(&job_id, dest.clone());
                let file_label = leaf.to_string();
                let job_id_pb = job_id.clone();
                let dm_pb = dm.clone();
                let r = huggingface::download_hf_file(
                    &client,
                    &repo_id,
                    &rev,
                    rp.as_str(),
                    &dest,
                    &mut pause_cancel,
                    move |msg, bd, bt| {
                        let base = (idx.saturating_sub(1) as f64 / n as f64) * 100.0;
                        let part = match bt {
                            Some(t) if t > 0 && n > 0 => {
                                ((bd as f64 / t as f64).min(1.0)) * (100.0 / n as f64)
                            }
                            _ => 0.0,
                        };
                        dm_pb.update_job(&job_id_pb, |d| {
                            d.message = msg.clone();
                            d.bytes_downloaded = bd;
                            d.bytes_total = bt;
                            d.progress = (base + part).min(99.9);
                            d.file_index = idx;
                            d.file_count = n;
                            d.current_file = Some(file_label.clone());
                        });
                    },
                )
                .await;
                match r {
                    Ok(_) => dm.release_partial(&job_id, &dest),
                    Err(e) => {
                        stopped = Some(e);
                        break;
                    }
                }
            }
            match stopped {
                Some(e) => Err(e),
                None => {
                    let first_rp = match repo_paths.first() {
                        Some(p) => p,
                        None => {
                            dm.mark_failed(&job_id, "No shards.".into());
                            return;
                        }
                    };
                    let first_leaf = match Path::new(first_rp)
                        .file_name()
                        .and_then(|x| x.to_str())
                    {
                        Some(l) => l,
                        None => {
                            dm.mark_failed(&job_id, "Invalid shard path.".into());
                            return;
                        }
                    };
                    let first_dest = models_dir.join(first_leaf);
                    let conn = db.lock();
                    register_weights_inner(
                        &conn,
                        &first_dest,
                        "safetensors",
                        Some(1),
                        Some(repo_paths.len() as i32),
                    )
                    .map(|m| m.id)
                }
            }
        }
    };

    finish_hf_job(&dm, &job_id, register_out);
}

async fn run_hf_download_manual_job(
    dm: Arc<DownloadManager>,
    db: Arc<parking_lot::Mutex<rusqlite::Connection>>,
    models_dir_default: PathBuf,
    job_id: String,
    repo_input: String,
    file_path: String,
    revision: Option<String>,
) {
    dm.set_running(&job_id);

    let repo_id = match huggingface::normalize_repo_id(&repo_input) {
        Ok(r) => r,
        Err(e) => {
            dm.mark_failed(&job_id, e);
            return;
        }
    };

    let token = huggingface::resolve_hf_token();
    let client = match huggingface::hf_client(token) {
        Ok(c) => c,
        Err(e) => {
            dm.mark_failed(&job_id, e);
            return;
        }
    };

    let models_dir_str = {
        let conn = db.lock();
        db::get_setting(&conn, "models_dir")
            .ok()
            .flatten()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| models_dir_default.to_string_lossy().into_owned())
    };
    let models_dir = PathBuf::from(&models_dir_str);
    if let Err(e) = std::fs::create_dir_all(&models_dir) {
        dm.mark_failed(&job_id, e.to_string());
        return;
    }

    let leaf = match Path::new(&file_path).file_name().and_then(|n| n.to_str()) {
        Some(l) => l,
        None => {
            dm.mark_failed(&job_id, "Invalid path.".into());
            return;
        }
    };
    let dest = models_dir.join(leaf);
    if dest.exists() {
        dm.mark_failed(
            &job_id,
            format!("Already on disk: {}", dest.display()),
        );
        return;
    }

    let rev = match revision
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        Some(r) => r.to_string(),
        None => match huggingface::resolve_default_revision(&client, &repo_id).await {
            Ok(r) => r,
            Err(e) => {
                dm.mark_failed(&job_id, e);
                return;
            }
        },
    };

    let mut pause_cancel = {
        let dm = dm.clone();
        let jid = job_id.clone();
        move || dm.job_pause_and_cancel(&jid).unwrap_or((false, true))
    };

    dm.update_job(&job_id, |d| {
        d.file_count = 1;
        d.file_index = 1;
        d.title = repo_id.clone();
    });
    dm.register_partial(&job_id, dest.clone());
    let file_label = leaf.to_string();
    let job_id_pb = job_id.clone();
    let dm_pb = dm.clone();
    let dl = huggingface::download_hf_file(
        &client,
        &repo_id,
        &rev,
        &file_path,
        &dest,
        &mut pause_cancel,
        move |msg, bd, bt| {
            let part = match bt {
                Some(t) if t > 0 => ((bd as f64 / t as f64).min(1.0)) * 100.0,
                _ => 0.0,
            };
            dm_pb.update_job(&job_id_pb, |d| {
                d.message = msg.clone();
                d.bytes_downloaded = bd;
                d.bytes_total = bt;
                d.progress = part.min(99.9);
                d.current_file = Some(file_label.clone());
            });
        },
    )
    .await;

    let out = match dl {
        Ok(_) => {
            dm.release_partial(&job_id, &dest);
            let ext = Path::new(&file_path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let wf = match ext.as_str() {
                "gguf" => "gguf",
                "safetensors" => "safetensors",
                _ => {
                    dm.mark_failed(&job_id, "Only .gguf or .safetensors.".into());
                    return;
                }
            };
            let conn = db.lock();
            register_weights_inner(&conn, &dest, wf, None, None).map(|m| m.id)
        }
        Err(e) => Err(e),
    };

    finish_hf_job(&dm, &job_id, out);
}
