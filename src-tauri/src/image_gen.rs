//! Cloud text-to-image generation (OpenAI-compatible APIs).

use crate::cloud_infer::CloudProviderStored;
use rusqlite::{params, Connection};
use rusqlite::OptionalExtension;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub enum ImageGenBackend {
    /// OpenAI `/v1/images/generations` or compatible.
    OpenAiCompatible {
        url: String,
        api_key: String,
        model: String,
    },
}

#[derive(Clone, Debug)]
pub struct ImageGenPlan {
    pub backend: ImageGenBackend,
}

pub struct GeneratedImage {
    pub filename: String,
    pub markdown: String,
}

pub fn generated_dir(app_data: &Path) -> PathBuf {
    app_data.join("generated")
}

fn chat_model_id(conn: &Connection, chat_id: &str) -> Option<String> {
    conn.query_row(
        "SELECT model_id FROM chats WHERE id = ?1",
        params![chat_id.trim()],
        |r| r.get::<_, Option<String>>(0),
    )
    .optional()
    .ok()
    .flatten()
    .flatten()
    .filter(|s| !s.trim().is_empty())
}

fn cloud_setting_key(slug: &str) -> Option<&'static str> {
    match slug {
        "openai" => Some("cloud_openai"),
        "openrouter" => Some("cloud_openrouter"),
        "custom" => Some("cloud_custom"),
        _ => None,
    }
}

fn cloud_slug_for_model(model_id: &str, cloud_provider: Option<&str>) -> Option<String> {
    if let Some(p) = cloud_provider.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(p.to_string());
    }
    model_id
        .strip_prefix("lm-cloud-")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn resolve_cloud_image_plan(conn: &Connection, slug: &str) -> Option<ImageGenPlan> {
    let sk = cloud_setting_key(slug)?;
    let raw = crate::db::get_setting(conn, sk).ok().flatten()?;
    let cfg: CloudProviderStored = serde_json::from_str(&raw).ok()?;
    if !cfg.image_generation_enabled {
        return None;
    }
    let model = cfg
        .image_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    let url = match slug {
        "openai" => "https://api.openai.com/v1/images/generations".to_string(),
        "openrouter" => "https://openrouter.ai/api/v1/images/generations".to_string(),
        "custom" => {
            let base = cfg.base_url.as_deref().unwrap_or("").trim();
            if base.is_empty() {
                return None;
            }
            if base.ends_with("/images/generations") {
                base.to_string()
            } else if base.ends_with("/v1") {
                format!("{base}/images/generations")
            } else {
                format!("{}/images/generations", base.trim_end_matches('/'))
            }
        }
        _ => return None,
    };
    Some(ImageGenPlan {
        backend: ImageGenBackend::OpenAiCompatible {
            url,
            api_key: cfg.api_key,
            model,
        },
    })
}

/// Resolve image generation for a chat, optionally overriding the chat's stored model id.
pub fn resolve_for_chat_with_model(
    conn: &Connection,
    chat_id: &str,
    model_id_override: Option<&str>,
) -> Option<ImageGenPlan> {
    if let Some(mid) = model_id_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Some(plan) = resolve_for_model(conn, mid) {
            return Some(plan);
        }
    }
    if let Some(mid) = chat_model_id(conn, chat_id) {
        if let Some(plan) = resolve_for_model(conn, &mid) {
            return Some(plan);
        }
    }
    if let Some(loaded) = crate::db::get_setting(conn, "loaded_model_id")
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
    {
        if let Some(plan) = resolve_for_model(conn, loaded.trim()) {
            return Some(plan);
        }
    }
    None
}

/// Resolve cloud image generation for a specific model library row id.
pub fn resolve_for_model(conn: &Connection, model_id: &str) -> Option<ImageGenPlan> {
    let mid = model_id.trim();
    if mid.is_empty() {
        return None;
    }

    let row: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT COALESCE(weights_format,'gguf'), cloud_provider FROM models WHERE id = ?1",
            params![mid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .ok()
        .flatten();

    let Some((wf, prov)) = row else {
        return None;
    };

    if wf != "cloud" {
        return None;
    }

    if let Some(slug) = cloud_slug_for_model(mid, prov.as_deref()) {
        return resolve_cloud_image_plan(conn, slug.trim());
    }

    None
}

pub async fn generate(
    app_data: &Path,
    plan: &ImageGenPlan,
    prompt: &str,
) -> Result<GeneratedImage, String> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("Image prompt is empty.".into());
    }
    let out_dir = generated_dir(app_data);
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let filename = format!("{}.png", Uuid::new_v4());
    let out_path = out_dir.join(&filename);

    match &plan.backend {
        ImageGenBackend::OpenAiCompatible { url, api_key, model } => {
            generate_openai_compatible(url, api_key, model, prompt, &out_path).await?;
        }
    }

    if !out_path.is_file() {
        return Err("Image generation did not produce an output file.".into());
    }

    let markdown = format!("![Generated image](localmod-gen:{filename})");
    Ok(GeneratedImage { filename, markdown })
}

async fn generate_openai_compatible(
    url: &str,
    api_key: &str,
    model: &str,
    prompt: &str,
    out_path: &Path,
) -> Result<(), String> {
    if looks_like_chat_only_model(model) {
        return Err(format!(
            "\"{model}\" is a chat model, not an image model. Set an image model (e.g. dall-e-3 or gpt-image-1) under Models → Cloud."
        ));
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|e| e.to_string())?;

    let variants = openai_image_request_variants(model, prompt);
    let mut last_err = String::from("Image API request failed.");

    for body in variants {
        let mut req = client.post(url).json(&body);
        if !api_key.trim().is_empty() {
            req = req.header("Authorization", format!("Bearer {}", api_key.trim()));
        }
        let res = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                last_err = format!("Image API request failed: {e}");
                continue;
            }
        };
        if !res.status().is_success() {
            last_err = format_api_error(res).await;
            continue;
        }
        let v: Value = match res.json().await {
            Ok(v) => v,
            Err(e) => {
                last_err = format!("Invalid image API response: {e}");
                continue;
            }
        };
        if save_image_from_json(&client, &v, out_path).await.is_ok() {
            return Ok(());
        }
        last_err = "Image API returned no image data.".into();
    }

    Err(last_err)
}

fn looks_like_chat_only_model(model: &str) -> bool {
    let m = model.trim().to_lowercase();
    if m.starts_with("gpt-image") {
        return false;
    }
    if m.contains("dall-e") || m.contains("dalle") {
        return false;
    }
    (m.starts_with("gpt-") && !m.starts_with("gpt-image"))
        || m.starts_with("o1")
        || m.starts_with("o3")
        || m.starts_with("o4")
        || m.contains("claude")
        || m.contains("gemini")
        || m.contains("llama")
        || m.contains("mistral")
}

fn is_gpt_image_model(model: &str) -> bool {
    model.trim().to_lowercase().starts_with("gpt-image")
}

fn is_dalle3(model: &str) -> bool {
    let m = model.trim().to_lowercase();
    m.contains("dall-e-3") || m == "dalle-3"
}

fn openai_image_request_variants(model: &str, prompt: &str) -> Vec<Value> {
    if is_gpt_image_model(model) {
        return vec![
            json!({ "model": model, "prompt": prompt, "n": 1, "size": "1024x1024" }),
            json!({ "model": model, "prompt": prompt, "n": 1, "size": "auto" }),
            json!({ "model": model, "prompt": prompt, "n": 1, "size": "1536x1024" }),
        ];
    }
    if is_dalle3(model) {
        return vec![
            json!({
                "model": model,
                "prompt": prompt,
                "n": 1,
                "size": "1024x1024",
                "response_format": "b64_json",
            }),
            json!({
                "model": model,
                "prompt": prompt,
                "n": 1,
                "size": "1024x1024",
                "response_format": "url",
            }),
        ];
    }
    vec![
        json!({
            "model": model,
            "prompt": prompt,
            "n": 1,
            "size": "1024x1024",
            "response_format": "b64_json",
        }),
        json!({
            "model": model,
            "prompt": prompt,
            "n": 1,
            "size": "512x512",
            "response_format": "b64_json",
        }),
        json!({
            "model": model,
            "prompt": prompt,
            "n": 1,
            "size": "1024x1024",
            "response_format": "url",
        }),
        json!({ "model": model, "prompt": prompt, "n": 1, "size": "1024x1024" }),
    ]
}

async fn format_api_error(res: reqwest::Response) -> String {
    let status = res.status();
    let txt = res.text().await.unwrap_or_default();
    if let Ok(v) = serde_json::from_str::<Value>(&txt) {
        if let Some(msg) = v["error"]["message"].as_str() {
            return format!("Image API error ({status}): {msg}");
        }
    }
    format!(
        "Image API error ({status}): {}",
        txt.chars().take(800).collect::<String>()
    )
}

async fn save_image_from_json(
    client: &reqwest::Client,
    v: &Value,
    out_path: &Path,
) -> Result<(), String> {
    if let Some(b64) = v["data"][0]["b64_json"].as_str() {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| format!("Invalid base64 image data: {e}"))?;
        std::fs::write(out_path, bytes).map_err(|e| e.to_string())?;
        return Ok(());
    }
    if let Some(img_url) = v["data"][0]["url"].as_str() {
        let bytes = client
            .get(img_url)
            .send()
            .await
            .map_err(|e| format!("Failed to download image: {e}"))?
            .bytes()
            .await
            .map_err(|e| e.to_string())?;
        std::fs::write(out_path, bytes).map_err(|e| e.to_string())?;
        return Ok(());
    }
    Err("Image API returned no image data.".into())
}
