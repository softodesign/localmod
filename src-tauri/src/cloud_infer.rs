//! Remote chat completion (OpenAI-compatible + Anthropic Messages API).

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudProviderStored {
    pub api_key: String,
    pub model: String,
    /// OpenAI-compatible base URL (e.g. `https://host/v1`). Used by the custom provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// When true, chat tools may call `generate_image` using `image_model`.
    #[serde(default)]
    pub image_generation_enabled: bool,
    /// Image model id (e.g. `dall-e-3`) for `/v1/images/generations`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_model: Option<String>,
}

fn chat_completions_url(base_url: &str) -> Result<String, String> {
    let u = base_url.trim().trim_end_matches('/').to_string();
    if u.is_empty() {
        return Err("Custom provider base URL is required.".into());
    }
    if u.ends_with("/chat/completions") {
        Ok(u)
    } else {
        Ok(format!("{u}/chat/completions"))
    }
}

fn flatten_content(v: &Value) -> Result<String, String> {
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
                    "image_url" => out.push_str("\n[Image omitted for cloud models]\n"),
                    _ => out.push_str("\n[attachment]\n"),
                }
            }
            Ok(out)
        }
        _ => Err("invalid message content shape".into()),
    }
}

fn openai_messages(api_msgs: &[(String, Value)]) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for (role, content) in api_msgs {
        let r = match role.as_str() {
            "system" | "user" | "assistant" => role.clone(),
            _ => "user".into(),
        };
        let c = match content {
            Value::String(s) => s.clone(),
            _ => flatten_content(content)?,
        };
        out.push(json!({ "role": r, "content": c }));
    }
    Ok(out)
}

fn anthropic_system_and_messages(
    api_msgs: &[(String, Value)],
) -> Result<(String, Vec<Value>), String> {
    let mut system_parts: Vec<String> = Vec::new();
    let mut turns: Vec<Value> = Vec::new();
    for (role, content) in api_msgs {
        let text = match content {
            Value::String(s) => s.clone(),
            _ => flatten_content(content)?,
        };
        match role.as_str() {
            "system" => system_parts.push(text),
            "user" | "assistant" => {
                let rr = if role == "user" { "user" } else { "assistant" };
                turns.push(json!({ "role": rr, "content": text }));
            }
            _ => turns.push(json!({ "role": "user", "content": text })),
        }
    }
    let mut merged: Vec<Value> = Vec::new();
    for m in turns {
        if let Some(last) = merged.last_mut() {
            if last["role"] == m["role"] {
                let a = last["content"].as_str().unwrap_or("").to_string();
                let b = m["content"].as_str().unwrap_or("");
                let role = last["role"].clone();
                *last = json!({ "role": role, "content": format!("{a}\n\n{b}") });
                continue;
            }
        }
        merged.push(m);
    }
    Ok((system_parts.join("\n\n"), merged))
}

fn emit_chunk(
    on_token: &impl Fn(String) -> Result<(), String>,
    acc: &mut String,
    chunk: &str,
) -> Result<(), String> {
    acc.push_str(chunk);
    let line = serde_json::json!({ "t": "c", "s": chunk }).to_string();
    on_token(line)
}

async fn stream_openai_sse(
    url: &str,
    api_key: &str,
    extra_headers: &[(&str, &str)],
    body: Value,
    cancel: &AtomicBool,
    on_token: &impl Fn(String) -> Result<(), String>,
) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;

    let mut req = client
        .post(url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body);
    for (k, v) in extra_headers {
        req = req.header(*k, *v);
    }
    let res = req.send().await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        let txt = res.text().await.unwrap_or_default();
        return Err(format!("Cloud API error: {}", txt.chars().take(800).collect::<String>()));
    }

    let mut acc = String::new();
    let mut stream = res.bytes_stream();
    let mut buf = String::new();
    while let Some(item) = stream.next().await {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let chunk = item.map_err(|e| e.to_string())?;
        buf.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buf.find('\n') {
            let raw_line = buf[..pos].to_string();
            buf = buf[pos + 1..].to_string();
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with(':') {
                continue;
            }
            let data = line.strip_prefix("data:").map(str::trim).unwrap_or("");
            if data.is_empty() {
                continue;
            }
            if data == "[DONE]" {
                return Ok(acc);
            }
            let Ok(v) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            if let Some(content) = v["choices"][0]["delta"]["content"].as_str() {
                if !content.is_empty() {
                    emit_chunk(on_token, &mut acc, content)?;
                }
            }
        }
    }
    Ok(acc)
}

/// Newer OpenAI chat models reject `max_tokens` and require `max_completion_tokens`.
/// Many of the same models only allow the default sampling temperature (1); custom values error.
fn openai_model_uses_max_completion_tokens(model: &str) -> bool {
    let m = model.trim().to_ascii_lowercase();
    m.contains("gpt-5")
        || m.starts_with("o1")
        || m.starts_with("o3")
        || m.starts_with("o4")
}

/// Stream assistant text using SSE (`data:` lines, OpenAI-style deltas).
pub async fn stream_openai_chat(
    api_key: &str,
    model: &str,
    api_msgs: &[(String, Value)],
    temperature: f32,
    max_tokens: i32,
    cancel: &AtomicBool,
    on_token_line: impl Fn(String) -> Result<(), String>,
) -> Result<String, String> {
    let messages = openai_messages(api_msgs)?;
    let cap = max_tokens.max(1).min(16_384);
    let mut body = serde_json::Map::new();
    body.insert("model".to_string(), json!(model));
    body.insert("messages".to_string(), json!(messages));
    body.insert("stream".to_string(), json!(true));
    let restricted = openai_model_uses_max_completion_tokens(model);
    if !restricted {
        body.insert("temperature".to_string(), json!(temperature));
    }
    if restricted {
        body.insert("max_completion_tokens".to_string(), json!(cap));
    } else {
        body.insert("max_tokens".to_string(), json!(cap));
    }
    let body = Value::Object(body);
    stream_openai_sse(
        "https://api.openai.com/v1/chat/completions",
        api_key,
        &[],
        body,
        cancel,
        &on_token_line,
    )
    .await
}

/// OpenRouter uses OpenAI-compatible chat completions.
pub async fn stream_openrouter_chat(
    api_key: &str,
    model: &str,
    api_msgs: &[(String, Value)],
    temperature: f32,
    max_tokens: i32,
    cancel: &AtomicBool,
    on_token_line: impl Fn(String) -> Result<(), String>,
) -> Result<String, String> {
    let messages = openai_messages(api_msgs)?;
    let body = json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "temperature": temperature,
        "max_tokens": max_tokens.max(1).min(8192),
    });
    let referer = std::env::var("OPENROUTER_REFERER")
        .unwrap_or_else(|_| "https://github.com/localmod".into());
    let title = std::env::var("OPENROUTER_TITLE").unwrap_or_else(|_| "LocalMOD".into());
    let extra: [(&str, &str); 2] = [
        ("HTTP-Referer", referer.as_str()),
        ("X-Title", title.as_str()),
    ];
    stream_openai_sse(
        "https://openrouter.ai/api/v1/chat/completions",
        api_key,
        &extra,
        body,
        cancel,
        &on_token_line,
    )
    .await
}

/// Any OpenAI-compatible `/chat/completions` endpoint (LM Studio, Ollama, Groq, etc.).
pub async fn stream_custom_openai_chat(
    base_url: &str,
    api_key: &str,
    model: &str,
    api_msgs: &[(String, Value)],
    temperature: f32,
    max_tokens: i32,
    cancel: &AtomicBool,
    on_token_line: impl Fn(String) -> Result<(), String>,
) -> Result<String, String> {
    let url = chat_completions_url(base_url)?;
    let messages = openai_messages(api_msgs)?;
    let body = json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "temperature": temperature,
        "max_tokens": max_tokens.max(1).min(8192),
    });
    stream_openai_sse(&url, api_key, &[], body, cancel, &on_token_line).await
}

/// Anthropic Messages API (non-streaming); UI still gets streamed-style chunks.
pub async fn run_anthropic_chat(
    api_key: &str,
    model: &str,
    api_msgs: &[(String, Value)],
    temperature: f32,
    max_tokens: i32,
    cancel: &AtomicBool,
    on_token_line: impl Fn(String) -> Result<(), String>,
) -> Result<String, String> {
    let (system, messages) = anthropic_system_and_messages(api_msgs)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;
    let mt = max_tokens.clamp(256, 8192);
    let body = json!({
        "model": model,
        "max_tokens": mt,
        "temperature": temperature as f64,
        "system": system,
        "messages": messages,
    });
    let res = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if cancel.load(Ordering::SeqCst) {
        return Ok(String::new());
    }
    if !res.status().is_success() {
        let txt = res.text().await.unwrap_or_default();
        return Err(format!(
            "Anthropic API error: {}",
            txt.chars().take(800).collect::<String>()
        ));
    }
    let v: Value = res.json().await.map_err(|e| e.to_string())?;
    let text = v["content"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|b| {
                    if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                        b.get("text").and_then(|t| t.as_str()).map(String::from)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();

    let mut acc = String::new();
    for ch_chunk in text.chars().collect::<Vec<char>>().chunks(48) {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let s: String = ch_chunk.iter().collect();
        emit_chunk(&on_token_line, &mut acc, &s)?;
    }
    Ok(acc)
}

pub async fn stream_by_provider_slug(
    slug: &str,
    stored: &CloudProviderStored,
    api_msgs: &[(String, Value)],
    temperature: f32,
    max_tokens: i32,
    cancel: &AtomicBool,
    on_token_line: impl Fn(String) -> Result<(), String>,
) -> Result<String, String> {
    let model = stored.model.trim();
    let key = stored.api_key.trim();
    match slug {
        "openai" => {
            stream_openai_chat(key, model, api_msgs, temperature, max_tokens, cancel, on_token_line)
                .await
        }
        "anthropic" => {
            run_anthropic_chat(key, model, api_msgs, temperature, max_tokens, cancel, on_token_line).await
        }
        "openrouter" => {
            stream_openrouter_chat(key, model, api_msgs, temperature, max_tokens, cancel, on_token_line).await
        }
        "custom" => {
            let base = stored
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "Custom provider base URL is missing.".to_string())?;
            stream_custom_openai_chat(
                base,
                key,
                model,
                api_msgs,
                temperature,
                max_tokens,
                cancel,
                on_token_line,
            )
            .await
        }
        _ => Err(format!("Unknown cloud provider: {}", slug)),
    }
}

async fn complete_openai_json(
    url: &str,
    api_key: &str,
    extra_headers: &[(&str, &str)],
    body: Value,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client
        .post(url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body);
    for (k, v) in extra_headers {
        req = req.header(*k, *v);
    }
    let res = req.send().await.map_err(|e| e.to_string())?;
    if cancel.load(Ordering::SeqCst) {
        return Ok(String::new());
    }
    if !res.status().is_success() {
        let txt = res.text().await.unwrap_or_default();
        return Err(format!("Cloud API error: {}", txt.chars().take(800).collect::<String>()));
    }
    let v: Value = res.json().await.map_err(|e| e.to_string())?;
    Ok(v["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string())
}

pub async fn complete_openai_chat(
    api_key: &str,
    model: &str,
    api_msgs: &[(String, Value)],
    temperature: f32,
    max_tokens: i32,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let messages = openai_messages(api_msgs)?;
    let cap = max_tokens.max(1).min(16_384);
    let mut body = serde_json::Map::new();
    body.insert("model".to_string(), json!(model));
    body.insert("messages".to_string(), json!(messages));
    body.insert("stream".to_string(), json!(false));
    let restricted = openai_model_uses_max_completion_tokens(model);
    if !restricted {
        body.insert("temperature".to_string(), json!(temperature));
    }
    if restricted {
        body.insert("max_completion_tokens".to_string(), json!(cap));
    } else {
        body.insert("max_tokens".to_string(), json!(cap));
    }
    complete_openai_json(
        "https://api.openai.com/v1/chat/completions",
        api_key,
        &[],
        Value::Object(body),
        cancel,
    )
    .await
}

pub async fn complete_openrouter_chat(
    api_key: &str,
    model: &str,
    api_msgs: &[(String, Value)],
    temperature: f32,
    max_tokens: i32,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let messages = openai_messages(api_msgs)?;
    let body = json!({
        "model": model,
        "messages": messages,
        "stream": false,
        "temperature": temperature,
        "max_tokens": max_tokens.max(1).min(8192),
    });
    let referer = std::env::var("OPENROUTER_REFERER")
        .unwrap_or_else(|_| "https://github.com/localmod".into());
    let title = std::env::var("OPENROUTER_TITLE").unwrap_or_else(|_| "LocalMOD".into());
    let extra: [(&str, &str); 2] = [
        ("HTTP-Referer", referer.as_str()),
        ("X-Title", title.as_str()),
    ];
    complete_openai_json(
        "https://openrouter.ai/api/v1/chat/completions",
        api_key,
        &extra,
        body,
        cancel,
    )
    .await
}

pub async fn complete_custom_openai_chat(
    base_url: &str,
    api_key: &str,
    model: &str,
    api_msgs: &[(String, Value)],
    temperature: f32,
    max_tokens: i32,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let url = chat_completions_url(base_url)?;
    let messages = openai_messages(api_msgs)?;
    let body = json!({
        "model": model,
        "messages": messages,
        "stream": false,
        "temperature": temperature,
        "max_tokens": max_tokens.max(1).min(8192),
    });
    complete_openai_json(&url, api_key, &[], body, cancel).await
}

pub async fn complete_anthropic_chat(
    api_key: &str,
    model: &str,
    api_msgs: &[(String, Value)],
    temperature: f32,
    max_tokens: i32,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let (system, messages) = anthropic_system_and_messages(api_msgs)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;
    let body = json!({
        "model": model,
        "max_tokens": max_tokens.max(1).min(8192),
        "temperature": temperature,
        "system": system,
        "messages": messages,
    });
    let res = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if cancel.load(Ordering::SeqCst) {
        return Ok(String::new());
    }
    if !res.status().is_success() {
        let txt = res.text().await.unwrap_or_default();
        return Err(format!(
            "Anthropic API error: {}",
            txt.chars().take(800).collect::<String>()
        ));
    }
    let v: Value = res.json().await.map_err(|e| e.to_string())?;
    let text = v["content"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|b| {
                    if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                        b.get("text").and_then(|t| t.as_str()).map(String::from)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    Ok(text)
}

pub async fn complete_by_provider_slug(
    slug: &str,
    stored: &CloudProviderStored,
    api_msgs: &[(String, Value)],
    temperature: f32,
    max_tokens: i32,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let model = stored.model.trim();
    let key = stored.api_key.trim();
    match slug {
        "openai" => {
            complete_openai_chat(key, model, api_msgs, temperature, max_tokens, cancel).await
        }
        "anthropic" => {
            complete_anthropic_chat(key, model, api_msgs, temperature, max_tokens, cancel).await
        }
        "openrouter" => {
            complete_openrouter_chat(key, model, api_msgs, temperature, max_tokens, cancel).await
        }
        "custom" => {
            let base = stored
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "Custom provider base URL is missing.".to_string())?;
            complete_custom_openai_chat(base, key, model, api_msgs, temperature, max_tokens, cancel)
                .await
        }
        _ => Err(format!("Unknown cloud provider: {}", slug)),
    }
}
