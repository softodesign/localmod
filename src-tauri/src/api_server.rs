use crate::cloud_infer;
use crate::db;
use crate::engine::GenerationParams;
#[cfg(feature = "llama-sidecar")]
use crate::llama_runtime;
use crate::state::{ApiServerRuntime, AppState};
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;

#[derive(Clone)]
struct ServerConfig {
    auth_mode: String,
    api_key: String,
}

#[derive(Clone)]
struct ApiHttpState {
    app: Option<tauri::AppHandle>,
    state: Arc<AppState>,
    config: ServerConfig,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiServerStatusDto {
    pub running: bool,
    pub host: String,
    pub port: u16,
    pub auth_mode: String,
    pub base_url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiServerSettingsDto {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub auth_mode: String,
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize)]
struct OpenAiModelList {
    object: &'static str,
    data: Vec<OpenAiModel>,
}

#[derive(Debug, Clone, Serialize)]
struct OpenAiModel {
    id: String,
    object: &'static str,
    created: i64,
    owned_by: &'static str,
}

#[derive(Debug, Clone, Deserialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<OpenAiChatMessage>,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    top_p: Option<f32>,
    #[serde(default)]
    max_tokens: Option<i32>,
    #[serde(default)]
    stream: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct OpenAiChatMessage {
    role: String,
    content: Value,
}

#[derive(Clone)]
struct ModelRoute {
    id: String,
    name: String,
    weights_format: String,
    cloud_provider: Option<String>,
}

pub fn settings_from_db(state: &AppState) -> ApiServerSettingsDto {
    let conn = state.db.lock();
    let enabled = db::get_setting(&conn, "api_server_enabled")
        .ok()
        .flatten()
        .map(|s| s == "true" || s == "1")
        .unwrap_or(false);
    let host = db::get_setting(&conn, "api_server_host")
        .ok()
        .flatten()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "127.0.0.1".into());
    let port = db::get_setting(&conn, "api_server_port")
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|p| *p > 0)
        .unwrap_or(11435);
    let auth_mode = db::get_setting(&conn, "api_server_auth_mode")
        .ok()
        .flatten()
        .filter(|s| s == "bearer")
        .unwrap_or_else(|| "none".into());
    let api_key = db::get_setting(&conn, "api_server_key")
        .ok()
        .flatten()
        .unwrap_or_default();
    ApiServerSettingsDto {
        enabled,
        host,
        port,
        auth_mode,
        api_key,
    }
}

pub fn status(state: &AppState) -> ApiServerStatusDto {
    if let Some(rt) = state.api_server.lock().as_ref() {
        return ApiServerStatusDto {
            running: true,
            host: rt.host.clone(),
            port: rt.port,
            auth_mode: rt.auth_mode.clone(),
            base_url: format!("http://{}:{}/v1", rt.host, rt.port),
        };
    }
    let settings = settings_from_db(state);
    ApiServerStatusDto {
        running: false,
        host: settings.host.clone(),
        port: settings.port,
        auth_mode: settings.auth_mode.clone(),
        base_url: format!("http://{}:{}/v1", settings.host, settings.port),
    }
}

pub async fn start(
    app: tauri::AppHandle,
    state: &AppState,
    host: String,
    port: u16,
    auth_mode: String,
    api_key: String,
) -> Result<ApiServerStatusDto, String> {
    stop(state).await?;
    let host = host.trim().to_string();
    if host.is_empty() {
        return Err("API server host cannot be empty.".into());
    }
    if port == 0 {
        return Err("API server port must be between 1 and 65535.".into());
    }
    let auth_mode = if auth_mode == "bearer" { "bearer" } else { "none" }.to_string();
    if auth_mode == "bearer" && api_key.trim().is_empty() {
        return Err("Bearer auth requires an API key.".into());
    }
    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .map_err(|e| format!("Invalid API server address: {e}"))?;
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| format!("Could not bind API server on {host}:{port}: {e}"))?;
    let bound = listener.local_addr().map_err(|e| e.to_string())?;
    let config = ServerConfig {
        auth_mode: auth_mode.clone(),
        api_key,
    };
    let http_state = ApiHttpState {
        app: Some(app),
        state: Arc::new(state.clone()),
        config: config.clone(),
    };
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let router = Router::new()
        .route("/v1/models", get(list_models))
        .route("/models", get(list_models))
        .route("/v1/chat/completions", post(chat_completions))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
                .allow_headers([AUTHORIZATION, CONTENT_TYPE]),
        )
        .with_state(http_state);

    tokio::spawn(async move {
        let server = axum::serve(listener, router).with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
        });
        if let Err(e) = server.await {
            eprintln!("[api-server] stopped with error: {e}");
        }
    });

    *state.api_server.lock() = Some(ApiServerRuntime {
        host,
        port: bound.port(),
        auth_mode,
        shutdown_tx: Some(shutdown_tx),
    });
    Ok(status(state))
}

pub async fn serve_headless(
    state: AppState,
    host: String,
    port: u16,
    auth_mode: String,
    api_key: String,
) -> Result<(), String> {
    let host = host.trim().to_string();
    if host.is_empty() {
        return Err("API server host cannot be empty.".into());
    }
    if port == 0 {
        return Err("API server port must be between 1 and 65535.".into());
    }
    let auth_mode = if auth_mode == "bearer" { "bearer" } else { "none" }.to_string();
    if auth_mode == "bearer" && api_key.trim().is_empty() {
        return Err("Bearer auth requires an API key.".into());
    }
    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .map_err(|e| format!("Invalid API server address: {e}"))?;
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| format!("Could not bind API server on {host}:{port}: {e}"))?;
    let bound = listener.local_addr().map_err(|e| e.to_string())?;
    let http_state = ApiHttpState {
        app: None,
        state: Arc::new(state),
        config: ServerConfig { auth_mode, api_key },
    };
    let router = Router::new()
        .route("/v1/models", get(list_models))
        .route("/models", get(list_models))
        .route("/v1/chat/completions", post(chat_completions))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
                .allow_headers([AUTHORIZATION, CONTENT_TYPE]),
        )
        .with_state(http_state);
    println!("[localmod-server] listening on http://{}:{}/v1", bound.ip(), bound.port());
    axum::serve(listener, router)
        .await
        .map_err(|e| format!("API server stopped with error: {e}"))
}

pub async fn stop(state: &AppState) -> Result<(), String> {
    let runtime = state.api_server.lock().take();
    if let Some(mut rt) = runtime {
        if let Some(tx) = rt.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
    Ok(())
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "error": {
                "message": "Unauthorized",
                "type": "invalid_request_error"
            }
        })),
    )
        .into_response()
}

fn json_error(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(json!({
            "error": {
                "message": message.into(),
                "type": "invalid_request_error"
            }
        })),
    )
        .into_response()
}

fn require_auth(headers: &HeaderMap, cfg: &ServerConfig) -> Result<(), Response> {
    if cfg.auth_mode != "bearer" {
        return Ok(());
    }
    let expected = format!("Bearer {}", cfg.api_key.trim());
    let got = headers
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if got == expected {
        Ok(())
    } else {
        Err(unauthorized())
    }
}

async fn list_models(
    State(st): State<ApiHttpState>,
    headers: HeaderMap,
) -> Result<Json<OpenAiModelList>, Response> {
    require_auth(&headers, &st.config)?;
    let rows = {
        let conn = st.state.db.lock();
        let mut stmt = conn
            .prepare("SELECT id, name FROM models WHERE COALESCE(weights_format,'gguf') IN ('gguf','cloud') ORDER BY name")
            .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let mapped = stmt
            .query_map([], |r| Ok(r.get::<_, String>(1)?))
            .map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let mut out = Vec::new();
        for row in mapped {
            out.push(row.map_err(|e| json_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?);
        }
        out
    };
    let created = Utc::now().timestamp();
    Ok(Json(OpenAiModelList {
        object: "list",
        data: rows
            .into_iter()
            .map(|name| {
                OpenAiModel {
                    id: name,
                    object: "model",
                    created,
                    owned_by: "localmod",
                }
            })
            .collect(),
    }))
}

async fn chat_completions(
    State(st): State<ApiHttpState>,
    headers: HeaderMap,
    Json(req): Json<ChatCompletionRequest>,
) -> Response {
    if let Err(res) = require_auth(&headers, &st.config) {
        return res;
    }
    if req.messages.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "messages cannot be empty");
    }
    let route = match resolve_model(&st.state, &req.model) {
        Ok(r) => r,
            Err(e) => return json_error(StatusCode::BAD_REQUEST, e),
    };
    if req.stream {
        stream_completion(st, route, req).await
    } else {
        match complete_once(st.app.as_ref(), &st.state, &route, &req, None).await {
            Ok(text) => Json(openai_completion_response(&route.name, text, &req)).into_response(),
            Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, e),
        }
    }
}

fn resolve_model(state: &AppState, model: &str) -> Result<ModelRoute, String> {
    let conn = state.db.lock();
    conn.query_row(
        "SELECT id, name, COALESCE(weights_format,'gguf'), cloud_provider FROM models WHERE id = ?1 OR name = ?1 ORDER BY CASE WHEN id = ?1 THEN 0 ELSE 1 END LIMIT 1",
        params![model],
        |r| {
            Ok(ModelRoute {
                id: r.get(0)?,
                name: r.get(1)?,
                weights_format: r.get(2)?,
                cloud_provider: r.get(3)?,
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("Model not found: {model}"))
}

fn api_messages(req: &ChatCompletionRequest) -> Vec<(String, Value)> {
    req.messages
        .iter()
        .map(|m| {
            let role = match m.role.as_str() {
                "system" | "user" | "assistant" => m.role.clone(),
                _ => "user".into(),
            };
            (role, m.content.clone())
        })
        .collect()
}

fn estimate_tokens(text: &str) -> usize {
    ((text.chars().count() as f64) / 4.0).ceil().max(1.0) as usize
}

fn gen_params(state: &AppState, req: &ChatCompletionRequest) -> GenerationParams {
    let conn = state.db.lock();
    let get_u32 = |key: &str, default: u32| {
        db::get_setting(&conn, key)
            .ok()
            .flatten()
            .and_then(|s| s.parse().ok())
            .unwrap_or(default)
    };
    let get_f32 = |key: &str, default: f32| {
        db::get_setting(&conn, key)
            .ok()
            .flatten()
            .and_then(|s| s.parse().ok())
            .unwrap_or(default)
    };
    let get_i32 = |key: &str, default: i32| {
        db::get_setting(&conn, key)
            .ok()
            .flatten()
            .and_then(|s| s.parse().ok())
            .unwrap_or(default)
    };
    GenerationParams {
        n_ctx: get_u32("n_ctx", 4096),
        n_threads: get_u32("n_threads", 0),
        n_threads_batch: 0,
        n_gpu_layers: get_u32("n_gpu_layers", 0),
        temperature: req.temperature.unwrap_or_else(|| get_f32("temperature", 0.7)),
        top_p: req.top_p.unwrap_or_else(|| get_f32("top_p", 0.9)),
        max_tokens: req.max_tokens.unwrap_or_else(|| get_i32("max_tokens", 768)),
        seed: get_u32("seed", 1234),
    }
}

async fn complete_once(
    app: Option<&tauri::AppHandle>,
    state: &AppState,
    route: &ModelRoute,
    req: &ChatCompletionRequest,
    stream_tx: Option<mpsc::Sender<Result<Event, Infallible>>>,
) -> Result<String, String> {
    let params = gen_params(state, req);
    let messages = api_messages(req);
    state.cancel.store(false, Ordering::SeqCst);
    if route.weights_format == "cloud" {
        let (slug, cfg) = resolve_cloud_config(state, route)?;
        if let Some(tx) = stream_tx {
            return cloud_infer::stream_by_provider_slug(
                &slug,
                &cfg,
                &messages,
                params.temperature,
                params.max_tokens,
                &state.cancel,
                |line| {
                    let text = parse_localmod_stream_line(&line);
                    if text.is_empty() {
                        return Ok(());
                    }
                    let chunk = openai_stream_chunk(&route.name, &text, false);
                    let _ = tx.try_send(Ok(Event::default().data(chunk)));
                    Ok(())
                },
            )
            .await;
        }
        return cloud_infer::complete_by_provider_slug(
            &slug,
            &cfg,
            &messages,
            params.temperature,
            params.max_tokens,
            &state.cancel,
        )
        .await;
    }

    if route.weights_format != "gguf" {
        return Err("Only GGUF and cloud models are available through the API server.".into());
    }

    #[cfg(feature = "llama-sidecar")]
    {
        if let Some(app) = app {
            llama_runtime::ensure_ready(app, state, &route.id).await?;
        } else {
            llama_runtime::ensure_ready_headless(state, &route.id).await?;
        }
        let base_url = {
            let g = state.sidecar.lock();
            let s = g
                .as_ref()
                .ok_or_else(|| "Local inference server is not running.".to_string())?;
            format!("http://127.0.0.1:{}", s.port)
        };
        if let Some(tx) = stream_tx {
            return llama_runtime::with_generation(state, &route.id, crate::llama_sidecar::stream_chat_completion(
                &base_url,
                &messages,
                params.temperature,
                params.top_p,
                params.max_tokens,
                false,
                &state.cancel,
                |part, text| {
                    if matches!(part, crate::llama_sidecar::StreamPart::Content) && !text.is_empty()
                    {
                        let chunk = openai_stream_chunk(&route.name, text, false);
                        let _ = tx.try_send(Ok(Event::default().data(chunk)));
                    }
                    Ok(())
                },
            ))
            .await;
        }
        llama_runtime::with_generation(state, &route.id, crate::llama_sidecar::complete_chat_completion(
            &base_url,
            &messages,
            params.temperature,
            params.top_p,
            params.max_tokens,
            false,
            &state.cancel,
        ))
        .await
    }
    #[cfg(not(feature = "llama-sidecar"))]
    {
        let _ = (app, stream_tx);
        Err("Local API server requires the llama-sidecar backend.".into())
    }
}

fn resolve_cloud_config(
    state: &AppState,
    route: &ModelRoute,
) -> Result<(String, cloud_infer::CloudProviderStored), String> {
    let slug = route
        .cloud_provider
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| route.id.strip_prefix("lm-cloud-"))
        .ok_or_else(|| "Cloud provider is missing for this model.".to_string())?
        .to_string();
    let key = match slug.as_str() {
        "openai" => "cloud_openai",
        "anthropic" => "cloud_anthropic",
        "openrouter" => "cloud_openrouter",
        "custom" => "cloud_custom",
        _ => return Err(format!("Unknown cloud provider: {slug}")),
    };
    let raw = {
        let conn = state.db.lock();
        db::get_setting(&conn, key)
            .map_err(|e| e.to_string())?
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| "Cloud provider is not configured.".to_string())?
    };
    let cfg = serde_json::from_str(&raw).map_err(|e| format!("Invalid cloud config: {e}"))?;
    Ok((slug, cfg))
}

fn openai_completion_response(model: &str, text: String, req: &ChatCompletionRequest) -> Value {
    let prompt_tokens = estimate_tokens(&serde_json::to_string(&req.messages).unwrap_or_default());
    let completion_tokens = if text.trim().is_empty() { 0 } else { estimate_tokens(&text) };
    json!({
        "id": format!("chatcmpl-{}", Uuid::new_v4()),
        "object": "chat.completion",
        "created": Utc::now().timestamp(),
        "model": model,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": text },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": prompt_tokens + completion_tokens
        }
    })
}

fn openai_stream_chunk(model: &str, text: &str, done: bool) -> String {
    if done {
        return json!({
            "id": format!("chatcmpl-{}", Uuid::new_v4()),
            "object": "chat.completion.chunk",
            "created": Utc::now().timestamp(),
            "model": model,
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": "stop"
            }]
        })
        .to_string();
    }
    json!({
        "id": format!("chatcmpl-{}", Uuid::new_v4()),
        "object": "chat.completion.chunk",
        "created": Utc::now().timestamp(),
        "model": model,
        "choices": [{
            "index": 0,
            "delta": { "content": text },
            "finish_reason": null
        }]
    })
    .to_string()
}

fn parse_localmod_stream_line(line: &str) -> String {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|v| v.get("s").and_then(|s| s.as_str()).map(str::to_string))
        .unwrap_or_else(|| line.to_string())
}

async fn stream_completion(
    st: ApiHttpState,
    route: ModelRoute,
    req: ChatCompletionRequest,
) -> Response {
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(64);
    let app = st.app.clone();
    let state = st.state.clone();
    let route_for_task = route.clone();
    tokio::spawn(async move {
        let result = complete_once(app.as_ref(), &state, &route_for_task, &req, Some(tx.clone())).await;
        match result {
            Ok(_) => {
                let done_chunk = openai_stream_chunk(&route_for_task.name, "", true);
                let _ = tx.send(Ok(Event::default().data(done_chunk))).await;
                let _ = tx.send(Ok(Event::default().data("[DONE]"))).await;
            }
            Err(e) => {
                let _ = tx
                    .send(Ok(Event::default().data(
                        json!({ "error": { "message": e, "type": "server_error" } }).to_string(),
                    )))
                    .await;
                let _ = tx.send(Ok(Event::default().data("[DONE]"))).await;
            }
        }
    });
    Sse::new(ReceiverStream::new(rx)).into_response()
}

