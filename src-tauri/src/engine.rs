//! Model handle + generation parameters. Inference runs either in-process (`llama-engine`, needs LLVM on Windows)
//! or via the default bundled `llama-server` sidecar (`llama-sidecar`).

use anyhow::Result;
#[cfg(any(
    feature = "llama-engine",
    all(not(feature = "llama-engine"), not(feature = "llama-sidecar"))
))]
use anyhow::anyhow;
use parking_lot::Mutex;
use std::path::PathBuf;
#[cfg(any(
    feature = "llama-engine",
    all(not(feature = "llama-engine"), not(feature = "llama-sidecar"))
))]
use std::sync::atomic::AtomicBool;
#[cfg(feature = "llama-engine")]
use std::sync::Arc;

#[derive(Clone)]
#[allow(dead_code)]
pub struct GenerationParams {
    pub n_ctx: u32,
    pub n_threads: u32,
    pub n_threads_batch: u32,
    pub n_gpu_layers: u32,
    pub temperature: f32,
    pub top_p: f32,
    pub max_tokens: i32,
    pub seed: u32,
}

pub struct LoadedModel {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    #[cfg(feature = "llama-engine")]
    pub model: llama_cpp_2::model::LlamaModel,
}

pub type LoadedSlot = Mutex<Option<LoadedModel>>;

pub fn new_loaded_slot() -> LoadedSlot {
    Mutex::new(None)
}

#[cfg(all(not(feature = "llama-engine"), not(feature = "llama-sidecar")))]
pub const LLAMA_BUILD_HINT: &str = "This copy of LocalMOD was built in UI-only mode (no AI backend). Use a normal build with bundled llama-server, or enable an inference feature in Cargo.";

#[cfg(feature = "llama-engine")]
pub fn backend_arc() -> Result<Arc<llama_cpp_2::llama_backend::LlamaBackend>> {
    Ok(Arc::new(
        llama_cpp_2::llama_backend::LlamaBackend::init()
            .map_err(|e| anyhow!("llama backend init: {e:?}"))?,
    ))
}

#[cfg(feature = "llama-engine")]
pub fn load_model_file(
    backend: &llama_cpp_2::llama_backend::LlamaBackend,
    id: String,
    name: String,
    path: PathBuf,
    n_gpu_layers: u32,
) -> Result<LoadedModel> {
    use anyhow::Context;
    use llama_cpp_2::model::params::LlamaModelParams;
    use llama_cpp_2::model::LlamaModel;
    use std::path::Path;

    let path_ref: &Path = path.as_ref();
    let mut params = LlamaModelParams::default();
    if n_gpu_layers > 0 {
        params = params.with_n_gpu_layers(n_gpu_layers);
    }
    let model = LlamaModel::load_from_file(backend, path_ref, &params)
        .with_context(|| format!("failed to load GGUF {}", path.display()))?;
    Ok(LoadedModel {
        id,
        name,
        path,
        model,
    })
}

#[cfg(not(feature = "llama-engine"))]
pub fn load_model_file(
    id: String,
    name: String,
    path: PathBuf,
    _n_gpu_layers: u32,
) -> Result<LoadedModel> {
    Ok(LoadedModel { id, name, path })
}

#[cfg(feature = "llama-engine")]
#[allow(clippy::too_many_arguments)]
pub fn generate_chat_reply(
    loaded: &LoadedModel,
    backend: &llama_cpp_2::llama_backend::LlamaBackend,
    messages: &[(String, String)],
    params: GenerationParams,
    cancel: &AtomicBool,
    token_tx: impl Fn(&str) -> Result<(), String>,
) -> Result<String> {
    use anyhow::Context;
    use encoding_rs::UTF_8;
    use llama_cpp_2::context::params::LlamaContextParams;
    use llama_cpp_2::llama_batch::LlamaBatch;
    use llama_cpp_2::model::AddBos;
    use llama_cpp_2::model::LlamaChatMessage;
    use llama_cpp_2::sampling::LlamaSampler;
    use std::num::NonZeroU32;

    let chat_msgs: Vec<LlamaChatMessage> = messages
        .iter()
        .map(|(r, c)| LlamaChatMessage::new(r.clone(), c.clone()))
        .collect::<Result<_, _>>()
        .map_err(|e| anyhow!("chat message: {e:?}"))?;

    let tmpl = loaded
        .model
        .chat_template(None)
        .map_err(|e| anyhow!("chat template: {e:?}"))?;

    let prompt = loaded
        .model
        .apply_chat_template(&tmpl, &chat_msgs, true)
        .map_err(|e| anyhow!("apply template: {e:?}"))?;

    let n_ctx = NonZeroU32::new(params.n_ctx.max(512)).unwrap();
    let mut ctx_params = LlamaContextParams::default().with_n_ctx(Some(n_ctx));

    if params.n_threads > 0 {
        ctx_params = ctx_params.with_n_threads(params.n_threads as i32);
    }
    if params.n_threads_batch > 0 {
        ctx_params = ctx_params.with_n_threads_batch(params.n_threads_batch as i32);
    } else if params.n_threads > 0 {
        ctx_params = ctx_params.with_n_threads_batch(params.n_threads as i32);
    }

    let mut ctx = loaded
        .model
        .new_context(backend, ctx_params)
        .map_err(|e| anyhow!("context: {e:?}"))?;

    let tokens_list = loaded
        .model
        .str_to_token(&prompt, AddBos::False)
        .map_err(|e| anyhow!("tokenize: {e:?}"))?;

    if tokens_list.is_empty() {
        return Err(anyhow!("Prompt tokenized to zero tokens"));
    }

    let n_cxt = ctx.n_ctx() as i32;
    let n_kv_req = tokens_list.len() as i32 + params.max_tokens;
    if n_kv_req > n_cxt {
        return Err(anyhow!(
            "Context too small: need ~{n_kv_req} tokens, n_ctx is {n_cxt}. Increase n_ctx in settings."
        ));
    }

    let mut batch = LlamaBatch::new((tokens_list.len() + params.max_tokens as usize).max(512), 1);
    let last_index: i32 = (tokens_list.len() - 1) as i32;
    for (i, token) in (0_i32..).zip(tokens_list.into_iter()) {
        let is_last = i == last_index;
        batch
            .add(token, i, &[0], is_last)
            .map_err(|e| anyhow!("batch add: {e:?}"))?;
    }

    ctx.decode(&mut batch)
        .map_err(|e| anyhow!("decode prompt: {e:?}"))?;

    let mut n_cur = batch.n_tokens();
    let mut sampler = LlamaSampler::chain_simple([
        LlamaSampler::top_p(params.top_p, 1),
        LlamaSampler::temp(params.temperature),
        LlamaSampler::dist(params.seed),
        LlamaSampler::greedy(),
    ]);

    let mut decoder = UTF_8.new_decoder();
    let mut acc = String::new();

    let mut n_generated = 0;
    while n_generated < params.max_tokens {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }

        let token = sampler.sample(&ctx, batch.n_tokens() - 1);
        sampler.accept(token);

        if loaded.model.is_eog_token(token) {
            break;
        }

        let piece = loaded
            .model
            .token_to_piece(token, &mut decoder, true, None)
            .map_err(|e| anyhow!("token piece: {e:?}"))?;

        acc.push_str(&piece);
        token_tx(&piece).map_err(|e| anyhow!(e))?;

        batch.clear();
        batch
            .add(token, n_cur, &[0], true)
            .map_err(|e| anyhow!("batch: {e:?}"))?;

        n_cur += 1;
        n_generated += 1;

        ctx.decode(&mut batch)
            .map_err(|e| anyhow!("decode: {e:?}"))?;
    }

    Ok(acc)
}

#[cfg(all(not(feature = "llama-engine"), not(feature = "llama-sidecar")))]
pub fn generate_chat_reply(
    _loaded: &LoadedModel,
    _backend: &(),
    _messages: &[(String, String)],
    _params: GenerationParams,
    _cancel: &AtomicBool,
    _token_tx: impl Fn(&str) -> Result<(), String>,
) -> Result<String> {
    Err(anyhow!(LLAMA_BUILD_HINT))
}

#[cfg(any(feature = "llama-engine", not(feature = "llama-sidecar")))]
pub fn with_loaded<T>(
    slot: &LoadedSlot,
    f: impl FnOnce(&LoadedModel) -> Result<T>,
) -> Result<T> {
    let guard = slot.lock();
    let m = guard
        .as_ref()
        .ok_or_else(|| anyhow!("No model loaded. Add a GGUF in Models, then click Load."))?;
    f(m)
}
