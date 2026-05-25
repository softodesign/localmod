//! Short notes for a weight file path (GGUF heuristics + mmproj; safetensors = not runnable here).

use serde::Serialize;
use std::path::Path;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModelKnowledgeDto {
    pub display_name: String,
    pub supports_images: bool,
    pub vision_explanation: String,
    pub summary: String,
    pub capabilities: Vec<String>,
}

fn lower_name(path: &Path) -> String {
    path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
        .to_lowercase()
}

fn stem_lower(path: &Path) -> String {
    path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
        .to_lowercase()
}

fn name_suggests_vision_family(s: &str) -> bool {
    s.contains("llava")
        || s.contains("qwen2-vl")
        || s.contains("qwen3-vl")
        || s.contains("qwen-vl")
        || s.contains("minicpm-v")
        || s.contains("internvl")
        || s.contains("moondream")
        || s.contains("bakllava")
        || s.contains("cogvlm")
        || s.contains("phi-3.5-vision")
        || s.contains("phi3.5-vision")
        || s.contains("pixtral")
        || s.contains("florence-2")
}

fn name_suggests_gpt_oss(s: &str) -> bool {
    s.contains("gpt-oss") || s.contains("gpt_oss")
}

pub fn knowledge_for_weights_path(path: &Path) -> ModelKnowledgeDto {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let display_name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Model".into());

    if ext == "safetensors" {
        return ModelKnowledgeDto {
            display_name,
            supports_images: false,
            vision_explanation: "—".into(),
            summary: "Safetensors on disk only. The chat runner uses GGUF.".into(),
            capabilities: vec![
                "Stored under your models folder".into(),
                "Use a GGUF build of the same model to load it here".into(),
            ],
        };
    }

    let name = lower_name(path);
    let stem = stem_lower(path);
    let has_mmproj = crate::mmproj_detect::auto_discover_mmproj(path).is_some();
    let vision_name = name_suggests_vision_family(&name);
    let supports_images = has_mmproj;

    let vision_explanation = if supports_images {
        "mmproj next to this GGUF: image tensor path enabled.".into()
    } else if vision_name {
        "Name suggests vision weights; no mmproj beside this file.".into()
    } else {
        "Text weights only unless you add a vision GGUF + mmproj pair.".into()
    };

    let (summary, capabilities) = if name_suggests_gpt_oss(&stem) || name_suggests_gpt_oss(&name) {
        (
            "gpt-oss-20B-class GGUF: text/chat oriented; public releases are not vision models.".into(),
            vec![
                "Text chat".into(),
                "Attachments as extracted text (PDF etc.)".into(),
                "No image tensors without a separate vision+mmproj stack".into(),
            ],
        )
    } else if vision_name && !has_mmproj {
        (
            "Likely vision family: needs mmproj beside main GGUF for pixels.".into(),
            vec![
                "Add mmproj from the same HF release, same folder".into(),
                "Reload model after files are in place".into(),
            ],
        )
    } else if has_mmproj {
        (
            "GGUF + mmproj: multimodal stack for this path.".into(),
            vec!["Text".into(), "Images (within app limits)".into()],
        )
    } else {
        (
            "GGUF for local text inference.".into(),
            vec!["Text chat".into(), "Files as text".into()],
        )
    };

    ModelKnowledgeDto {
        display_name,
        supports_images,
        vision_explanation,
        summary,
        capabilities,
    }
}
