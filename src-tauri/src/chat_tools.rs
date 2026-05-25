//! Web search, image generation, and local agent tools (filesystem, shell, system info).

use crate::image_gen::{self, ImageGenPlan};
use crate::system_metrics;
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

const MAX_FILE_BYTES: usize = 256 * 1024;
const MAX_CMD_OUTPUT: usize = 32 * 1024;
const CMD_TIMEOUT_SECS: u64 = 30;

pub struct ToolContext<'a> {
    pub app_data_dir: &'a Path,
    pub models_dir: &'a Path,
    pub context_dir: &'a Path,
    pub image_gen: Option<ImageGenPlan>,
}

#[derive(Debug, Clone)]
pub struct ParsedToolCall {
    pub name: String,
    pub arguments: Value,
}

/// Extract `<tool_call>{json}</tool_call>` blocks from model output.
pub fn extract_tool_calls(text: &str) -> Vec<ParsedToolCall> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("<tool_call>") {
        let after_open = &rest[start + "<tool_call>".len()..];
        let Some(end) = after_open.find("</tool_call>") else {
            break;
        };
        let inner = after_open[..end].trim();
        if let Ok(v) = serde_json::from_str::<Value>(inner) {
            if let (Some(name), args) = (
                v.get("name").and_then(|n| n.as_str()).map(String::from),
                v.get("arguments").cloned().unwrap_or(json!({})),
            ) {
                out.push(ParsedToolCall { name, arguments: args });
            }
        }
        rest = &after_open[end + "</tool_call>".len()..];
    }
    out
}

pub fn strip_tool_call_blocks(text: &str) -> String {
    let mut s = text.to_string();
    while let Some(start) = s.find("<tool_call>") {
        let Some(end) = s[start..].find("</tool_call>") else {
            break;
        };
        let end_idx = start + end + "</tool_call>".len();
        s.replace_range(start..end_idx, "");
    }
    s.trim().to_string()
}

/// Pull `![…](localmod-gen:…)` from a successful generate_image tool result.
pub fn extract_generated_image_markdown(tool_result: &str) -> Option<String> {
    for line in tool_result.lines() {
        let t = line.trim();
        if t.starts_with("![") && t.contains("localmod-gen:") {
            return Some(t.to_string());
        }
    }
    let open = tool_result.find("![")?;
    let close = tool_result[open..].find(')')? + open + 1;
    let candidate = tool_result[open..close].trim();
    if candidate.contains("localmod-gen:") {
        Some(candidate.to_string())
    } else {
        None
    }
}

/// Append generated image markdown when the model forgot to include it.
pub fn ensure_generated_images(text: &str, images: &[String]) -> String {
    if images.is_empty() {
        return text.to_string();
    }
    let mut out = text.trim().to_string();
    for md in images {
        let md = md.trim();
        if md.is_empty() {
            continue;
        }
        if out.contains("localmod-gen:") {
            if let Some(start) = md.find("localmod-gen:") {
                let rest = &md[start..];
                let end = rest.find(')').unwrap_or(rest.len());
                let token = &rest[..end];
                if out.contains(token) {
                    continue;
                }
            }
        }
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(md);
    }
    out
}

pub fn build_tools_system_prompt(web_search: bool, agent: bool, image_generation: bool) -> String {
    let mut tools: Vec<&str> = Vec::new();
    if web_search {
        tools.push(
            "- web_search(query): Search the web for current information. Argument: query (string).",
        );
    }
    if image_generation {
        tools.push(
            "- generate_image(prompt): Create an image from a text description. Argument: prompt (string). \
Include the returned markdown image in your final reply when showing the picture to the user.",
        );
    }
    if agent {
        tools.push("- read_file(path): Read a text file. Argument: path (string).");
        tools.push("- list_dir(path): List directory entries. Argument: path (string, optional — defaults to app data).");
        tools.push("- write_file(path, content): Create or replace a UTF-8 text file. Arguments: path (string), content (string).");
        tools.push("- edit_file(path, old_text, new_text): Replace one exact text span in a UTF-8 file. Arguments: path, old_text, new_text (strings).");
        tools.push("- create_folder(path): Create a folder, including parent folders if needed. Argument: path (string).");
        tools.push("- run_command(command): Run a shell command and return stdout/stderr. Argument: command (string).");
        tools.push("- install_package(command): Install packages with a package-manager command such as npm, pnpm, yarn, cargo, pip, uv, bun, or go. Argument: command (string).");
        tools.push("- debug(command): Run a diagnostic/debug command and return stdout/stderr. Argument: command (string).");
        tools.push("- system_info(): Return CPU, RAM, GPU, and disk metrics. No arguments.");
    }
    if tools.is_empty() {
        return String::new();
    }
    format!(
        "You can use tools to gather information before answering.\n\n\
When you need a tool, respond with ONLY one or more tool call blocks (no other text):\n\
<tool_call>\n\
{{\"name\": \"tool_name\", \"arguments\": {{\"key\": \"value\"}}}}\n\
</tool_call>\n\n\
After tool results are provided, either call more tools or give your final answer in normal prose \
(without tool_call blocks).\n\n\
Available tools:\n{}\n",
        tools.join("\n")
    )
}

pub async fn execute_tool(name: &str, args: &Value, ctx: &ToolContext<'_>) -> Result<String, String> {
    match name {
        "web_search" => {
            let q = args
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if q.is_empty() {
                return Err("web_search requires a non-empty query".into());
            }
            web_search(q).await
        }
        "generate_image" => {
            let p = args
                .get("prompt")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if p.is_empty() {
                return Err("generate_image requires a non-empty prompt".into());
            }
            let plan = ctx
                .image_gen
                .as_ref()
                .ok_or_else(|| {
                    "Image generation is not configured for this chat model.".to_string()
                })?;
            let img = image_gen::generate(ctx.app_data_dir, plan, p).await?;
            Ok(format!(
                "Image generated successfully.\n\nUse this markdown in your reply:\n\n{}\n\nSaved as: {}",
                img.markdown,
                img.filename
            ))
        }
        "read_file" => {
            let p = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "read_file requires path".to_string())?;
            read_file_allowed(p, ctx)
        }
        "list_dir" => {
            let p = args
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            list_dir_allowed(p, ctx)
        }
        "write_file" => {
            let p = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "write_file requires path".to_string())?;
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "write_file requires content".to_string())?;
            write_file_allowed(p, content, ctx)
        }
        "edit_file" => {
            let p = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "edit_file requires path".to_string())?;
            let old_text = args
                .get("old_text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "edit_file requires old_text".to_string())?;
            let new_text = args
                .get("new_text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "edit_file requires new_text".to_string())?;
            edit_file_allowed(p, old_text, new_text, ctx)
        }
        "create_folder" => {
            let p = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "create_folder requires path".to_string())?;
            create_folder_allowed(p, ctx)
        }
        "run_command" => {
            let cmd = args
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "run_command requires command".to_string())?;
            run_command_safe(cmd)
        }
        "install_package" => {
            let cmd = args
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "install_package requires command".to_string())?;
            run_install_command_safe(cmd)
        }
        "debug" => {
            let cmd = args
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "debug requires command".to_string())?;
            run_command_safe(cmd)
        }
        "system_info" => Ok(system_info_json()),
        _ => Err(format!("Unknown tool: {name}")),
    }
}

async fn web_search(query: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("LocalMOD/0.1 (+https://github.com/localmod)")
        .build()
        .map_err(|e| e.to_string())?;

    let url = format!(
        "https://html.duckduckgo.com/html/?q={}",
        urlencoding::encode(query)
    );
    let html = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Web search request failed: {e}"))?
        .text()
        .await
        .map_err(|e| format!("Web search response failed: {e}"))?;

    let results = parse_ddg_html(&html);
    if results.is_empty() {
        return Ok(format!("No web results found for: {query}"));
    }
    let mut out = format!("Web search results for \"{query}\":\n\n");
    for (i, (title, snippet, link)) in results.iter().take(8).enumerate() {
        out.push_str(&format!(
            "{}. {}\n   {}\n   {}\n\n",
            i + 1,
            title,
            snippet,
            link
        ));
    }
    Ok(out.trim().to_string())
}

fn parse_ddg_html(html: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(a) = rest.find("class=\"result__a\"") {
        let chunk = &rest[a..];
        let href = extract_attr(chunk, "href").unwrap_or_default();
        let title = extract_tag_text(chunk, "a").unwrap_or_default();
        let snippet = chunk
            .split("class=\"result__snippet\"")
            .nth(1)
            .and_then(|c| extract_tag_text(c, "a"))
            .or_else(|| {
                chunk
                    .split("class=\"result__snippet\"")
                    .nth(1)
                    .and_then(|c| extract_tag_text(c, "span"))
            })
            .unwrap_or_default();
        if !title.is_empty() {
            out.push((decode_html(&title), decode_html(&snippet), href));
        }
        rest = &rest[a + 10..];
    }
    out
}

fn extract_attr(s: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=\"");
    let start = s.find(&needle)? + needle.len();
    let end = s[start..].find('"')? + start;
    Some(s[start..end].to_string())
}

fn extract_tag_text(s: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let start = s.find(&open)?;
    let after = &s[start..];
    let gt = after.find('>')? + 1;
    let inner = &after[gt..];
    let close = format!("</{tag}>");
    let end = inner.find(&close)?;
    Some(inner[..end].trim().to_string())
}

fn decode_html(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn resolve_allowed_path(raw: &str, ctx: &ToolContext<'_>) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    let path = if trimmed.is_empty() {
        ctx.app_data_dir.to_path_buf()
    } else if path_has_parent_traversal(Path::new(trimmed)) {
        return Err("Path cannot contain parent traversal (`..`)".into());
    } else {
        PathBuf::from(trimmed)
    };
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| ctx.app_data_dir.to_path_buf())
            .join(path)
    };
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("Invalid path {}: {e}", path.display()))?;
    if !path_is_under_allowed_root(&canonical, ctx) {
        return Err(format!(
            "Path not allowed (must be under the app data folder, models folder, context folder, or current workspace): {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

fn allowed_roots(ctx: &ToolContext<'_>) -> Vec<PathBuf> {
    let mut roots = vec![
        ctx.app_data_dir.to_path_buf(),
        ctx.models_dir.to_path_buf(),
        ctx.context_dir.to_path_buf(),
    ];
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    roots
}

fn path_is_under_allowed_root(path: &Path, ctx: &ToolContext<'_>) -> bool {
    allowed_roots(ctx).iter().any(|root| {
        root.canonicalize()
            .ok()
            .map(|r| path.starts_with(&r))
            .unwrap_or(false)
    })
}

fn resolve_writable_path(raw: &str, ctx: &ToolContext<'_>) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Path is required".into());
    }
    let input = PathBuf::from(trimmed);
    if path_has_parent_traversal(&input) {
        return Err("Path cannot contain parent traversal (`..`)".into());
    }
    let path = if input.is_absolute() {
        input
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| ctx.app_data_dir.to_path_buf())
            .join(input)
    };
    let parent = path
        .parent()
        .ok_or_else(|| format!("Path has no parent: {}", path.display()))?;
    let parent_canonical = parent
        .canonicalize()
        .map_err(|e| format!("Invalid parent directory {}: {e}", parent.display()))?;
    if !path_is_under_allowed_root(&parent_canonical, ctx) {
        return Err(format!(
            "Path not allowed (must be under the app data folder, models folder, context folder, or current workspace): {}",
            path.display()
        ));
    }
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("Path has no file/folder name: {}", path.display()))?;
    Ok(parent_canonical.join(file_name))
}

fn read_file_allowed(raw: &str, ctx: &ToolContext<'_>) -> Result<String, String> {
    let path = resolve_allowed_path(raw, ctx)?;
    if !path.is_file() {
        return Err(format!("Not a file: {}", path.display()));
    }
    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if meta.len() as usize > MAX_FILE_BYTES {
        return Err(format!(
            "File too large (max {} KB): {}",
            MAX_FILE_BYTES / 1024,
            path.display()
        ));
    }
    std::fs::read_to_string(&path).map_err(|e| format!("Read failed: {e}"))
}

fn list_dir_allowed(raw: &str, ctx: &ToolContext<'_>) -> Result<String, String> {
    let path = resolve_allowed_path(raw, ctx)?;
    if !path.is_dir() {
        return Err(format!("Not a directory: {}", path.display()));
    }
    let mut entries: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        let kind = if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            "dir"
        } else {
            "file"
        };
        entries.push(format!("{name} ({kind})"));
    }
    entries.sort();
    Ok(format!(
        "Directory {} ({} entries):\n{}",
        path.display(),
        entries.len(),
        entries.join("\n")
    ))
}

fn write_file_allowed(raw: &str, content: &str, ctx: &ToolContext<'_>) -> Result<String, String> {
    let path = resolve_writable_path(raw, ctx)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Create parent failed: {e}"))?;
    }
    std::fs::write(&path, content).map_err(|e| format!("Write failed: {e}"))?;
    Ok(format!(
        "Wrote {} bytes to {}",
        content.len(),
        path.display()
    ))
}

fn edit_file_allowed(
    raw: &str,
    old_text: &str,
    new_text: &str,
    ctx: &ToolContext<'_>,
) -> Result<String, String> {
    if old_text.is_empty() {
        return Err("old_text cannot be empty".into());
    }
    let path = resolve_allowed_path(raw, ctx)?;
    if !path.is_file() {
        return Err(format!("Not a file: {}", path.display()));
    }
    if !path_is_under_allowed_root(&path, ctx) {
        return Err(format!("Path not allowed: {}", path.display()));
    }
    let original = std::fs::read_to_string(&path).map_err(|e| format!("Read failed: {e}"))?;
    let count = original.matches(old_text).count();
    if count == 0 {
        return Err("old_text was not found in file".into());
    }
    if count > 1 {
        return Err("old_text matched more than once; include more surrounding context".into());
    }
    let updated = original.replacen(old_text, new_text, 1);
    std::fs::write(&path, updated).map_err(|e| format!("Write failed: {e}"))?;
    Ok(format!("Edited {}", path.display()))
}

fn create_folder_allowed(raw: &str, ctx: &ToolContext<'_>) -> Result<String, String> {
    let path = resolve_writable_path(raw, ctx)?;
    std::fs::create_dir_all(&path).map_err(|e| format!("Create folder failed: {e}"))?;
    Ok(format!("Created folder {}", path.display()))
}

fn command_is_blocked(cmd: &str) -> Option<&'static str> {
    let lower = cmd.to_lowercase();
    const BLOCKED: &[&str] = &[
        "rm -rf",
        "rm -r",
        "del /f",
        "format ",
        "shutdown",
        "reboot",
        "reg delete",
        "diskpart",
        ":(){",
        "mkfs",
        "dd if=",
        "curl | sh",
        "wget | sh",
        "invoke-expression",
        "iex ",
    ];
    for b in BLOCKED {
        if lower.contains(b) {
            return Some(b);
        }
    }
    None
}

fn install_command_allowed(cmd: &str) -> bool {
    let lower = cmd.trim().to_lowercase();
    const PREFIXES: &[&str] = &[
        "npm install",
        "npm i",
        "pnpm add",
        "pnpm install",
        "yarn add",
        "yarn install",
        "bun add",
        "bun install",
        "cargo add",
        "pip install",
        "python -m pip install",
        "py -m pip install",
        "uv add",
        "uv pip install",
        "go get",
        "dotnet add package",
    ];
    PREFIXES.iter().any(|p| lower.starts_with(p))
}

fn run_install_command_safe(command: &str) -> Result<String, String> {
    let cmd = command.trim();
    if !install_command_allowed(cmd) {
        return Err(
            "install_package only accepts package-manager install commands (npm/pnpm/yarn/bun/cargo/pip/uv/go/dotnet)."
                .into(),
        );
    }
    run_command_safe(cmd)
}

fn run_command_safe(command: &str) -> Result<String, String> {
    let cmd = command.trim();
    if cmd.is_empty() {
        return Err("Empty command".into());
    }
    if let Some(b) = command_is_blocked(cmd) {
        return Err(format!("Command blocked for safety (matched: {b})"));
    }
    for c in cmd.chars() {
        if c.is_control() && c != '\n' && c != '\t' {
            return Err("Command contains invalid control characters".into());
        }
    }

    #[cfg(windows)]
    let mut child = std::process::Command::new("cmd")
        .args(["/C", cmd])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run command: {e}"))?;

    #[cfg(not(windows))]
    let mut child = std::process::Command::new("sh")
        .args(["-c", cmd])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run command: {e}"))?;

    let deadline = Duration::from_secs(CMD_TIMEOUT_SECS);
    let start = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            let mut stdout = String::new();
            let mut stderr = String::new();
            if let Some(mut out) = child.stdout.take() {
                use std::io::Read;
                let _ = out.read_to_string(&mut stdout);
            }
            if let Some(mut err) = child.stderr.take() {
                use std::io::Read;
                let _ = err.read_to_string(&mut stderr);
            }
            let mut combined = String::new();
            if !stdout.is_empty() {
                combined.push_str(&stdout);
            }
            if !stderr.is_empty() {
                if !combined.is_empty() {
                    combined.push_str("\n--- stderr ---\n");
                }
                combined.push_str(&stderr);
            }
            if combined.len() > MAX_CMD_OUTPUT {
                combined.truncate(MAX_CMD_OUTPUT);
                combined.push_str("\n… (output truncated)");
            }
            return Ok(format!(
                "Exit code: {}\n{}",
                status.code().unwrap_or(-1),
                if combined.is_empty() {
                    "(no output)".to_string()
                } else {
                    combined
                }
            ));
        }
        if start.elapsed() > deadline {
            let _ = child.kill();
            return Err(format!("Command timed out after {CMD_TIMEOUT_SECS}s"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn system_info_json() -> String {
    let snap = system_metrics::capture_system_snapshot();
    serde_json::to_string_pretty(&snap).unwrap_or_else(|_| "Failed to serialize system info".into())
}

/// Reject path traversal in raw paths before canonicalize.
#[allow(dead_code)]
fn path_has_parent_traversal(path: &Path) -> bool {
    path.components().any(|c| matches!(c, Component::ParentDir))
}
