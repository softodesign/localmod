//! Multi-turn tool loop for local and cloud chat backends.

use crate::chat_tools::{self, ToolContext};
use crate::cloud_infer;
use crate::engine::GenerationParams;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::ipc::Channel;

const MAX_TOOL_ROUNDS: usize = 6;

pub struct ToolAgentOptions<'a> {
    pub web_search: bool,
    pub agent: bool,
    pub image_generation: bool,
    pub thinking_enabled: bool,
    pub gen_params: GenerationParams,
    pub cancel: &'a AtomicBool,
    pub tool_ctx: ToolContext<'a>,
}

fn push_system(api_msgs: &mut Vec<(String, Value)>, text: String) {
    if text.trim().is_empty() {
        return;
    }
    api_msgs.push(("system".into(), Value::String(text)));
}

fn emit_tool_status(on_token: &Channel<String>, label: &str) {
    let _ = on_token.send(format!("LMPHASE:tool:{label}"));
}

fn emit_content(on_token: &Channel<String>, chunk: &str) -> Result<(), String> {
    let line = json!({ "t": "c", "s": chunk }).to_string();
    on_token.send(line).map_err(|e| e.to_string())
}

#[cfg(feature = "llama-sidecar")]
async fn complete_sidecar(
    base_url: &str,
    api_msgs: &[(String, Value)],
    gen_params: &GenerationParams,
    thinking_enabled: bool,
    cancel: &AtomicBool,
) -> Result<String, String> {
    crate::llama_sidecar::complete_chat_completion(
        base_url,
        api_msgs,
        gen_params.temperature,
        gen_params.top_p,
        gen_params.max_tokens,
        thinking_enabled,
        cancel,
    )
    .await
}

#[cfg(feature = "llama-sidecar")]
async fn stream_sidecar_final(
    base_url: &str,
    api_msgs: &[(String, Value)],
    gen_params: &GenerationParams,
    thinking_enabled: bool,
    cancel: &AtomicBool,
    on_token: &Channel<String>,
) -> Result<String, String> {
    let on_token_blocking = on_token.clone();
    let send = |part: crate::llama_sidecar::StreamPart, t: &str| {
        let tag = match part {
            crate::llama_sidecar::StreamPart::Reasoning => "r",
            crate::llama_sidecar::StreamPart::Content => "c",
        };
        let line = json!({ "t": tag, "s": t }).to_string();
        on_token_blocking.send(line).map_err(|e| e.to_string())
    };
    crate::llama_sidecar::stream_chat_completion(
        base_url,
        api_msgs,
        gen_params.temperature,
        gen_params.top_p,
        gen_params.max_tokens,
        thinking_enabled,
        cancel,
        send,
    )
    .await
}

async fn complete_cloud(
    prov: &str,
    cfg: &cloud_infer::CloudProviderStored,
    api_msgs: &[(String, Value)],
    gen_params: &GenerationParams,
    cancel: &AtomicBool,
) -> Result<String, String> {
    cloud_infer::complete_by_provider_slug(
        prov,
        cfg,
        api_msgs,
        gen_params.temperature,
        gen_params.max_tokens,
        cancel,
    )
    .await
}

async fn stream_cloud_final(
    prov: &str,
    cfg: &cloud_infer::CloudProviderStored,
    api_msgs: &[(String, Value)],
    gen_params: &GenerationParams,
    cancel: &AtomicBool,
    on_token: &Channel<String>,
) -> Result<String, String> {
    let on_token_blocking = on_token.clone();
    cloud_infer::stream_by_provider_slug(
        prov,
        cfg,
        api_msgs,
        gen_params.temperature,
        gen_params.max_tokens,
        cancel,
        |line| on_token_blocking.send(line).map_err(|e| e.to_string()),
    )
    .await
}

pub async fn run_with_tools(
    mut api_msgs: Vec<(String, Value)>,
    cloud_plan: Option<(String, cloud_infer::CloudProviderStored)>,
    sidecar_base: Option<String>,
    opts: ToolAgentOptions<'_>,
    on_token: Channel<String>,
) -> Result<String, String> {
    if !opts.web_search && !opts.agent && !opts.image_generation {
        return run_plain(api_msgs, cloud_plan, sidecar_base, &opts, on_token).await;
    }

    let tools_prompt = chat_tools::build_tools_system_prompt(
        opts.web_search,
        opts.agent,
        opts.image_generation,
    );
    push_system(&mut api_msgs, tools_prompt);

    let mut generated_images: Vec<String> = Vec::new();

    for _round in 0..MAX_TOOL_ROUNDS {
        if opts.cancel.load(Ordering::SeqCst) {
            break;
        }

        let reply = if let Some((ref prov, ref cfg)) = cloud_plan {
            complete_cloud(prov, cfg, &api_msgs, &opts.gen_params, opts.cancel).await?
        } else {
            #[cfg(feature = "llama-sidecar")]
            {
                let base = sidecar_base.as_ref().ok_or_else(|| {
                    "Inference server is not running.".to_string()
                })?;
                complete_sidecar(
                    base,
                    &api_msgs,
                    &opts.gen_params,
                    opts.thinking_enabled,
                    opts.cancel,
                )
                .await?
            }
            #[cfg(not(feature = "llama-sidecar"))]
            {
                return Err("Tool agent requires llama-sidecar or cloud model.".into());
            }
        };

        let calls = chat_tools::extract_tool_calls(&reply);
        if calls.is_empty() {
            let visible = chat_tools::ensure_generated_images(
                &chat_tools::strip_tool_call_blocks(&reply),
                &generated_images,
            );
            for ch in visible.chars().collect::<Vec<_>>().chunks(32) {
                if opts.cancel.load(Ordering::SeqCst) {
                    break;
                }
                let s: String = ch.iter().collect();
                emit_content(&on_token, &s)?;
            }
            return Ok(visible);
        }

        api_msgs.push(("assistant".into(), Value::String(reply.clone())));

        for call in calls {
            if opts.cancel.load(Ordering::SeqCst) {
                break;
            }
            emit_tool_status(&on_token, &call.name);
            let result = chat_tools::execute_tool(&call.name, &call.arguments, &opts.tool_ctx).await;
            let body = match &result {
                Ok(r) => {
                    if call.name == "generate_image" {
                        if let Some(md) = chat_tools::extract_generated_image_markdown(r) {
                            generated_images.push(md);
                        }
                    }
                    r.clone()
                }
                Err(e) => format!("Tool error: {e}"),
            };
            let tool_msg = format!(
                "Tool result for `{}`:\n\n{body}",
                call.name
            );
            api_msgs.push(("user".into(), Value::String(tool_msg)));
        }
    }

    if opts.cancel.load(Ordering::SeqCst) {
        return Ok(String::new());
    }

    let final_reply = if let Some((ref prov, ref cfg)) = cloud_plan {
        stream_cloud_final(prov, cfg, &api_msgs, &opts.gen_params, opts.cancel, &on_token).await?
    } else {
        #[cfg(feature = "llama-sidecar")]
        {
            let base = sidecar_base.as_ref().ok_or_else(|| {
                "Inference server is not running.".to_string()
            })?;
            stream_sidecar_final(
                base,
                &api_msgs,
                &opts.gen_params,
                opts.thinking_enabled,
                opts.cancel,
                &on_token,
            )
            .await?
        }
        #[cfg(not(feature = "llama-sidecar"))]
        {
            return Err("Tool agent requires llama-sidecar or cloud model.".into());
        }
    };

    Ok(chat_tools::ensure_generated_images(
        &final_reply,
        &generated_images,
    ))
}

async fn run_plain(
    api_msgs: Vec<(String, Value)>,
    cloud_plan: Option<(String, cloud_infer::CloudProviderStored)>,
    sidecar_base: Option<String>,
    opts: &ToolAgentOptions<'_>,
    on_token: Channel<String>,
) -> Result<String, String> {
    if let Some((ref prov, ref cfg)) = cloud_plan {
        stream_cloud_final(prov, cfg, &api_msgs, &opts.gen_params, opts.cancel, &on_token).await
    } else {
        #[cfg(feature = "llama-sidecar")]
        {
            let base = sidecar_base.as_ref().ok_or_else(|| {
                "Inference server is not running.".to_string()
            })?;
            stream_sidecar_final(
                base,
                &api_msgs,
                &opts.gen_params,
                opts.thinking_enabled,
                opts.cancel,
                &on_token,
            )
            .await
        }
        #[cfg(not(feature = "llama-sidecar"))]
        {
            let _ = (api_msgs, sidecar_base, on_token);
            Err("Inference backend not available.".into())
        }
    }
}
