use crate::db;
use crate::engine;
use crate::engine::GenerationParams;
use crate::state::AppState;
use parking_lot::Mutex;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use tokio::sync::Mutex as TokioMutex;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlamaRuntimeStatusDto {
    pub phase: String,
    pub model_id: Option<String>,
    pub model_name: Option<String>,
    pub error: Option<String>,
    pub base_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LlamaRuntimeSnapshot {
    pub phase: String,
    pub model_id: Option<String>,
    pub model_name: Option<String>,
    pub error: Option<String>,
}

impl Default for LlamaRuntimeSnapshot {
    fn default() -> Self {
        Self {
            phase: "idle".into(),
            model_id: None,
            model_name: None,
            error: None,
        }
    }
}

pub struct LlamaRuntimeState {
    pub status: Mutex<LlamaRuntimeSnapshot>,
    pub op_lock: TokioMutex<()>,
}

impl LlamaRuntimeState {
    pub fn new() -> Self {
        Self {
            status: Mutex::new(LlamaRuntimeSnapshot::default()),
            op_lock: TokioMutex::new(()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LocalModelRow {
    pub id: String,
    pub name: String,
    pub path: String,
    pub weights_format: String,
}

pub fn status(state: &AppState) -> LlamaRuntimeStatusDto {
    let snap = state.llama_runtime.status.lock().clone();
    let base_url = state
        .sidecar
        .lock()
        .as_ref()
        .map(|rt| format!("http://127.0.0.1:{}", rt.port));
    LlamaRuntimeStatusDto {
        phase: snap.phase,
        model_id: snap.model_id,
        model_name: snap.model_name,
        error: snap.error,
        base_url,
    }
}

pub fn set_status(
    state: &AppState,
    phase: impl Into<String>,
    model_id: Option<String>,
    model_name: Option<String>,
    error: Option<String>,
) {
    *state.llama_runtime.status.lock() = LlamaRuntimeSnapshot {
        phase: phase.into(),
        model_id,
        model_name,
        error,
    };
}

pub fn model_row(state: &AppState, model_id: &str) -> Result<LocalModelRow, String> {
    let conn = state.db.lock();
    conn.query_row(
        "SELECT id, name, path, COALESCE(weights_format, 'gguf') FROM models WHERE id = ?1",
        params![model_id],
        |row| {
            Ok(LocalModelRow {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                weights_format: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("Model not found: {model_id}"))
}

fn parse_u32_setting(conn: &rusqlite::Connection, key: &str, default: u32) -> u32 {
    db::get_setting(conn, key)
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

pub fn generation_params(state: &AppState) -> GenerationParams {
    let conn = state.db.lock();
    GenerationParams {
        n_ctx: parse_u32_setting(&conn, "n_ctx", 4096),
        n_threads: parse_u32_setting(&conn, "n_threads", 0),
        n_threads_batch: 0,
        n_gpu_layers: parse_u32_setting(&conn, "n_gpu_layers", 0),
        temperature: db::get_setting(&conn, "temperature")
            .ok()
            .flatten()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.7),
        top_p: db::get_setting(&conn, "top_p")
            .ok()
            .flatten()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.9),
        max_tokens: db::get_setting(&conn, "max_tokens")
            .ok()
            .flatten()
            .and_then(|s| s.parse().ok())
            .unwrap_or(768),
        seed: parse_u32_setting(&conn, "seed", 1234),
    }
}

fn sidecar_ready_for(state: &AppState, model_path: &Path, params: &GenerationParams) -> bool {
    let canonical = std::fs::canonicalize(model_path).unwrap_or_else(|_| model_path.to_path_buf());
    let g = state.sidecar.lock();
    match g.as_ref() {
        Some(rt) => {
            rt.model_path == canonical
                && crate::llama_sidecar::sidecar_inference_matches(
                    rt,
                    params.n_ctx,
                    params.n_threads,
                    params.n_gpu_layers,
                )
        }
        None => false,
    }
}

#[cfg(feature = "llama-sidecar")]
pub async fn select_model(
    app: AppHandle,
    state: AppState,
    model_id: String,
) -> Result<LocalModelRow, String> {
    let row = model_row(&state, &model_id)?;
    if row.weights_format == "cloud" {
        crate::llama_sidecar::kill_sidecar_slot_async(&state).await.ok();
        db::set_setting(&state.db.lock(), "loaded_model_id", &model_id)
            .map_err(|e| e.to_string())?;
        set_status(&state, "ready", Some(row.id.clone()), Some(row.name.clone()), None);
        return Ok(row);
    }
    if row.weights_format != "gguf" {
        return Err("Only GGUF and cloud models can be selected for chat.".into());
    }
    let p = PathBuf::from(&row.path);
    if !p.exists() {
        return Err(format!("Model file missing: {}", p.display()));
    }
    let n_gpu = {
        let conn = state.db.lock();
        parse_u32_setting(&conn, "n_gpu_layers", 0)
    };
    let loaded = engine::load_model_file(row.id.clone(), row.name.clone(), p.clone(), n_gpu)
        .map_err(|e| e.to_string())?;
    *state.loaded.lock() = Some(loaded);
    db::set_setting(&state.db.lock(), "loaded_model_id", &model_id).map_err(|e| e.to_string())?;
    set_status(&state, "loading", Some(row.id.clone()), Some(row.name.clone()), None);

    let warm_state = state.clone();
    let warm_app = app.clone();
    let warm_model_id = model_id.clone();
    std::thread::Builder::new()
        .name("localmod-llama-warmup".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let row = model_row(&warm_state, &warm_model_id).ok();
                    set_status(
                        &warm_state,
                        "failed",
                        Some(warm_model_id),
                        row.map(|r| r.name),
                        Some(format!("Failed to create runtime warmup task: {e}")),
                    );
                    return;
                }
            };
            rt.block_on(async move {
                if let Err(e) = ensure_ready(&warm_app, &warm_state, &warm_model_id).await {
                    let row = model_row(&warm_state, &warm_model_id).ok();
                    set_status(
                        &warm_state,
                        "failed",
                        Some(warm_model_id),
                        row.map(|r| r.name),
                        Some(e),
                    );
                }
            });
        })
        .map_err(|e| format!("Failed to start runtime warmup task: {e}"))?;
    Ok(row)
}

#[cfg(feature = "llama-sidecar")]
pub async fn ensure_ready(
    app: &AppHandle,
    state: &AppState,
    model_id: &str,
) -> Result<(), String> {
    ensure_ready_with_app(app, state, model_id).await
}

#[cfg(feature = "llama-sidecar")]
pub async fn ensure_ready_headless(state: &AppState, model_id: &str) -> Result<(), String> {
    ensure_ready_inner(None, state, model_id).await
}

#[cfg(feature = "llama-sidecar")]
async fn ensure_ready_with_app(
    app: &AppHandle,
    state: &AppState,
    model_id: &str,
) -> Result<(), String> {
    ensure_ready_inner(Some(app), state, model_id).await
}

#[cfg(feature = "llama-sidecar")]
async fn ensure_ready_inner(
    app: Option<&AppHandle>,
    state: &AppState,
    model_id: &str,
) -> Result<(), String> {
    let row = model_row(state, model_id)?;
    if row.weights_format == "cloud" {
        return Ok(());
    }
    if row.weights_format != "gguf" {
        return Err("Only GGUF rows can run local chat inference.".into());
    }
    let p = PathBuf::from(&row.path);
    if !p.exists() {
        return Err(format!("Model file missing: {}", p.display()));
    }

    let _op = state.llama_runtime.op_lock.lock().await;
    let params = generation_params(state);
    if sidecar_ready_for(state, &p, &params) {
        set_status(state, "ready", Some(row.id), Some(row.name), None);
        return Ok(());
    }

    set_status(
        state,
        "loading",
        Some(row.id.clone()),
        Some(row.name.clone()),
        None,
    );
    let mmproj = crate::mmproj_detect::auto_discover_mmproj(p.as_path())
        .and_then(|x| std::fs::canonicalize(&x).ok().or(Some(x)));
    let restart_result = if let Some(app) = app {
        crate::llama_sidecar::restart_server(
            app,
            state,
            &p,
            params.n_ctx,
            params.n_threads,
            params.n_gpu_layers,
            mmproj,
        )
        .await
    } else {
        let runtime_dir = state.llama_runtime_dir.lock().clone();
        let exe = crate::llama_sidecar::resolve_llama_server_path_headless(runtime_dir.as_deref())?;
        crate::llama_sidecar::restart_server_with_exe(
            state,
            exe,
            &p,
            params.n_ctx,
            params.n_threads,
            params.n_gpu_layers,
            mmproj,
        )
        .await
    };
    match restart_result {
        Ok(()) => {
            let loaded = engine::load_model_file(
                row.id.clone(),
                row.name.clone(),
                p,
                params.n_gpu_layers,
            )
            .map_err(|e| e.to_string())?;
            *state.loaded.lock() = Some(loaded);
            set_status(state, "ready", Some(row.id), Some(row.name), None);
            Ok(())
        }
        Err(e) => {
            set_status(
                state,
                "failed",
                Some(row.id),
                Some(row.name),
                Some(e.clone()),
            );
            Err(e)
        }
    }
}

#[cfg(feature = "llama-sidecar")]
pub async fn with_generation<T>(
    state: &AppState,
    model_id: &str,
    f: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    let _op = state.llama_runtime.op_lock.lock().await;
    let row = model_row(state, model_id).ok();
    set_status(
        state,
        "generating",
        Some(model_id.to_string()),
        row.as_ref().map(|r| r.name.clone()),
        None,
    );
    let result = f.await;
    match &result {
        Ok(_) => set_status(
            state,
            "ready",
            Some(model_id.to_string()),
            row.map(|r| r.name),
            None,
        ),
        Err(e) => set_status(
            state,
            "failed",
            Some(model_id.to_string()),
            row.map(|r| r.name),
            Some(e.clone()),
        ),
    }
    result
}

pub fn runtime_validation(app: &AppHandle) -> Result<String, String> {
    let path = crate::llama_sidecar::resolve_llama_server_path(app)?;
    Ok(path.to_string_lossy().into_owned())
}

pub fn runtime_validation_headless(state: &AppState) -> Result<String, String> {
    let runtime_dir = state.llama_runtime_dir.lock().clone();
    let path = crate::llama_sidecar::resolve_llama_server_path_headless(runtime_dir.as_deref())?;
    Ok(path.to_string_lossy().into_owned())
}

