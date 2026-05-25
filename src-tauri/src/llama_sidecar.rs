//! Runs GGUF inference via a bundled `llama-server` binary (OpenAI-compatible HTTP API).
//! Avoids LLVM/libclang at compile time. Spawns the process with `std::process::Command` so we
//! can resolve `llama-server.exe` or the target-triple name reliably (Tauri’s shell sidecar was
//! picking up a 0-byte placeholder when both files exist).

use crate::state::{AppState, SidecarRuntime};
use serde_json::Value;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::Ordering;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

const MIN_LLAMA_EXE_BYTES: u64 = 4096;

fn looks_like_real_exe(path: &Path) -> bool {
    path.metadata()
        .map(|m| m.is_file() && m.len() >= MIN_LLAMA_EXE_BYTES)
        .unwrap_or(false)
}

/// Any `llama-server` / `llama-server-*` in `dir` that looks like a real executable (not a stub).
fn best_llama_server_in_dir(dir: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let rd = std::fs::read_dir(dir).ok()?;
    for e in rd.flatten() {
        let p = e.path();
        if !p.is_file() {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_lowercase();
        let good = if cfg!(target_os = "windows") {
            name == "llama-server.exe" || (name.starts_with("llama-server-") && name.ends_with(".exe"))
        } else {
            name == "llama-server" || name.starts_with("llama-server-")
        };
        if good && looks_like_real_exe(&p) {
            candidates.push(p);
        }
    }
    // Prefer triple-suffixed names over plain `llama-server.exe`.
    candidates.sort_by(|a, b| {
        let la = a.file_name().map(|s| s.len()).unwrap_or(0);
        let lb = b.file_name().map(|s| s.len()).unwrap_or(0);
        lb.cmp(&la)
    });
    candidates.into_iter().next()
}

/// Locates `llama-server`. Prefer `binaries/llama-runtime/` (full zip extract) so every `.dll` sits next to the exe — required on Windows.
pub fn resolve_llama_server_path(app: &AppHandle) -> Result<PathBuf, String> {
    #[cfg(debug_assertions)]
    {
        let manifest_bin = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries");
        let rt = manifest_bin.join("llama-runtime");
        if rt.is_dir() {
            if let Some(p) = best_llama_server_in_dir(&rt) {
                return Ok(std::fs::canonicalize(&p).unwrap_or(p));
            }
        }
        if let Some(p) = best_llama_server_in_dir(&manifest_bin) {
            return Ok(std::fs::canonicalize(&p).unwrap_or(p));
        }
    }

    if let Ok(res_root) = app.path().resource_dir() {
        let rt = res_root.join("binaries").join("llama-runtime");
        if rt.is_dir() {
            if let Some(p) = best_llama_server_in_dir(&rt) {
                return Ok(p);
            }
        }
        let flat = res_root.join("binaries");
        if flat.is_dir() {
            if let Some(p) = best_llama_server_in_dir(&flat) {
                return Ok(p);
            }
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let rt = parent.join("binaries").join("llama-runtime");
            if rt.is_dir() {
                if let Some(p) = best_llama_server_in_dir(&rt) {
                    return Ok(p);
                }
            }
            if let Some(p) = best_llama_server_in_dir(parent) {
                return Ok(p);
            }
        }
    }

    let dev_hint = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("binaries")
        .join("llama-runtime");
    Err(format!(
        "Could not find llama-server (expected at least {} bytes). \
Extract the **full** Windows zip (exe + **all** .dll files) into:\n  {}\n\
Or place exe + DLLs together under src-tauri/binaries/. See binaries/llama-runtime/README.md.",
        MIN_LLAMA_EXE_BYTES,
        dev_hint.display()
    ))
}

pub fn resolve_llama_server_path_headless(runtime_dir: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(dir) = runtime_dir {
        if dir.is_dir() {
            if let Some(p) = best_llama_server_in_dir(dir) {
                return Ok(std::fs::canonicalize(&p).unwrap_or(p));
            }
        }
        if dir.is_file() && looks_like_real_exe(dir) {
            return Ok(std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf()));
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let rt = dir.join("llama-runtime");
            if rt.is_dir() {
                if let Some(p) = best_llama_server_in_dir(&rt) {
                    return Ok(std::fs::canonicalize(&p).unwrap_or(p));
                }
            }
            if let Some(p) = best_llama_server_in_dir(dir) {
                return Ok(std::fs::canonicalize(&p).unwrap_or(p));
            }
        }
    }

    #[cfg(debug_assertions)]
    {
        let manifest_bin = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries");
        let rt = manifest_bin.join("llama-runtime");
        if rt.is_dir() {
            if let Some(p) = best_llama_server_in_dir(&rt) {
                return Ok(std::fs::canonicalize(&p).unwrap_or(p));
            }
        }
        if let Some(p) = best_llama_server_in_dir(&manifest_bin) {
            return Ok(std::fs::canonicalize(&p).unwrap_or(p));
        }
    }

    Err("Could not find llama-server. Put llama-server.exe and its DLLs next to localmod-server, in a llama-runtime folder next to it, or pass --runtime-dir.".into())
}

fn pick_free_port() -> Result<u16, String> {
    TcpListener::bind("127.0.0.1:0")
        .map_err(|e| e.to_string())?
        .local_addr()
        .map(|a| a.port())
        .map_err(|e| e.to_string())
}

/// Thread count passed to `-t`. Some builds misbehave with `-t 0`; 0 means "pick a default".
fn effective_llama_threads(n_threads: u32) -> u32 {
    if n_threads == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get().max(2) as u32)
            .unwrap_or(4)
    } else {
        n_threads
    }
}

#[cfg(feature = "llama-sidecar")]
pub fn sidecar_inference_matches(
    rt: &SidecarRuntime,
    db_n_ctx: u32,
    db_n_threads_setting: u32,
    db_ngl: u32,
) -> bool {
    let eff = effective_llama_threads(db_n_threads_setting);
    rt.n_ctx == db_n_ctx && rt.n_threads == eff && rt.n_gpu_layers == db_ngl
}

#[cfg(windows)]
fn hint_for_llama_exit_status(status: &std::process::ExitStatus) -> Option<&'static str> {
    let code = status.code()? as u32;
    match code {
        // STATUS_DLL_NOT_FOUND
        0xC0000135 => Some(
            "Almost always means a DLL next to llama-server is missing. Official zips ship **llama-server.exe plus many .dll files** — extract the **entire** zip into `src-tauri/binaries/llama-runtime/` (see README there). Copying only the exe causes this. \
Also install [Visual C++ Redistributable x64](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist). \
CUDA builds need CUDA/cuDNN; prefer a **CPU / win-x64** zip if you are not using CUDA.",
        ),
        // STATUS_INVALID_IMAGE_FORMAT — wrong arch
        0xC000007B => Some(
            "The llama-server.exe architecture does not match Windows (wrong build). Download the **x64** Windows binary from llama.cpp releases, not Arm32 or an incompatible variant.",
        ),
        // STATUS_ACCESS_VIOLATION
        0xC0000005 => Some(
            "Access violation in llama-server (native crash). Often tied to **prompt cache / KV** or a **buggy build**. LocalMOD disables prompt cache by default (`--cache-ram 0`); update **llama.cpp** to a newer release build, try a **smaller quant** or lower **Context (n_ctx)** in Settings, and ensure you aren’t mixing an old **llama-server.exe** with a different GGUF.",
        ),
        _ => None,
    }
}

#[cfg(not(windows))]
fn hint_for_llama_exit_status(_: &std::process::ExitStatus) -> Option<&'static str> {
    None
}

/// Wait until HTTP `/health` returns 200. While the GGUF is mmap'd, llama.cpp returns **503**.
async fn wait_until_llama_ready(port: u16, child: &mut Child) -> Result<(), String> {
    let base = format!("http://127.0.0.1:{port}");
    let urls = [
        format!("{base}/health"),
        format!("{base}/v1/health"),
    ];
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    // Poll quickly so “ready” is detected soon after load completes (~25 min max wait).
    const POLL_MS: u64 = 150;
    const MAX_ATTEMPTS: u32 = 10_000;

    for attempt in 0..MAX_ATTEMPTS {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut msg = format!("llama-server exited while starting (status: {status}).");
                if let Some(hint) = hint_for_llama_exit_status(&status) {
                    msg.push(' ');
                    msg.push_str(hint);
                } else {
                    msg.push_str(
                        " Check the terminal for errors from llama-server (bad GGUF path, missing DLL, unsupported file, or a failed buffer allocation during load — try a smaller GGUF/quant, lower context length, or free system RAM).",
                    );
                }
                return Err(msg);
            }
            Ok(None) => {}
            Err(e) => return Err(format!("could not wait on llama-server: {e}")),
        }

        for url in &urls {
            match client.get(url).send().await {
                Ok(resp) => {
                    let s = resp.status();
                    if s.is_success() {
                        return Ok(());
                    }
                    if s.as_u16() == 503 {
                        // Model still loading — keep polling.
                        break;
                    }
                }
                Err(_) => continue,
            }
        }

        if attempt == 200 || attempt == 1_000 || (attempt > 0 && attempt % 2_000 == 0) {
            eprintln!(
                "[LocalMOD] Still waiting for llama-server on port {port}… \
(if the model is huge this can take several minutes; 503 = still loading) attempt {attempt}/{MAX_ATTEMPTS}"
            );
        }

        tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
    }

    Err(format!(
        "llama-server did not report ready within ~{} minutes (port {port}). \
Try a smaller GGUF, move the file to a faster disk, or run llama-server manually to see errors.",
        (MAX_ATTEMPTS as u64 * POLL_MS) / 60_000
    ))
}

/// Stop llama-server and clear the sidecar slot only (keeps `loaded` — same model, new process).
pub async fn kill_sidecar_process_async(state: &AppState) -> Result<(), String> {
    let child_opt = {
        let mut g = state.sidecar.lock();
        if let Some(mut rt) = g.take() {
            let _ = rt.child.kill();
            Some(rt.child)
        } else {
            None
        }
    };
    if let Some(mut child) = child_opt {
        tokio::task::spawn_blocking(move || {
            let _ = child.wait();
        })
        .await
        .map_err(|e| format!("sidecar reap: {e}"))?;
    }
    Ok(())
}

pub async fn kill_sidecar_slot_async(state: &AppState) -> Result<(), String> {
    *state.loaded.lock() = None;
    kill_sidecar_process_async(state).await
}

pub async fn restart_server(
    app: &AppHandle,
    state: &AppState,
    model_path: &Path,
    n_ctx: u32,
    n_threads: u32,
    n_gpu_layers: u32,
    mmproj: Option<PathBuf>,
) -> Result<(), String> {
    kill_sidecar_process_async(state).await?;

    // Let the OS release the old process / listener before spawning again (Windows).
    tokio::time::sleep(Duration::from_millis(120)).await;

    let exe_path = resolve_llama_server_path(app)?;
    let abs_model = std::fs::canonicalize(model_path).map_err(|e| e.to_string())?;
    let path_str = abs_model.to_string_lossy().into_owned();

    let port = pick_free_port()?;

    let n_ctx_s = n_ctx.to_string();
    let threads_resolved = effective_llama_threads(n_threads);
    let n_threads_s = threads_resolved.to_string();
    let ngl_s = n_gpu_layers.to_string();
    let port_s = port.to_string();

    let work_dir = exe_path
        .parent()
        .ok_or_else(|| "llama-server path has no parent directory".to_string())?
        .to_path_buf();

    // Default llama.cpp enables weight repack on CPU, which can request a multi-GB
    // `CPU_REPACK` buffer and fail on typical free-RAM + fragmentation even when
    // `--fit` projects a smaller footprint. `--no-repack` avoids that allocation
    // (slightly slower inference, much more likely to load).
    let mut cmd = Command::new(&exe_path);
    cmd.current_dir(&work_dir)
        .arg("--no-repack")
        .arg("-m")
        .arg(&path_str)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(&port_s)
        .arg("-c")
        .arg(&n_ctx_s)
        .arg("-t")
        .arg(&n_threads_s)
        .arg("-ngl")
        .arg(&ngl_s);
    // Prompt cache (see ggml-org/llama.cpp#16391) can destabilize some Windows builds/models
    // (e.g. access violations after load). Disable unless we add an explicit user toggle later.
    cmd.arg("--cache-ram").arg("0");
    cmd.arg("--no-warmup");
    // Single slot for one chat client — avoids default auto n_parallel=4 and lowers KV/RAM use.
    cmd.arg("-np").arg("1");

    if let Some(ref proj) = mmproj {
        if proj.is_file() {
            let proj_str = proj.to_string_lossy().into_owned();
            cmd.arg("--mmproj").arg(proj_str);
        }
    }

    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| {
            format!(
                "Failed to start llama-server from {}: {e}",
                exe_path.display()
            )
        })?;

    if let Err(e) = wait_until_llama_ready(port, &mut child).await {
        let _ = child.kill();
        return Err(e);
    }

    {
        let mut g = state.sidecar.lock();
        *g = Some(SidecarRuntime {
            port,
            child,
            model_path: abs_model,
            n_ctx,
            n_threads: threads_resolved,
            n_gpu_layers,
        });
    }
    Ok(())
}

pub async fn restart_server_with_exe(
    state: &AppState,
    exe_path: PathBuf,
    model_path: &Path,
    n_ctx: u32,
    n_threads: u32,
    n_gpu_layers: u32,
    mmproj: Option<PathBuf>,
) -> Result<(), String> {
    kill_sidecar_process_async(state).await?;

    tokio::time::sleep(Duration::from_millis(120)).await;

    let abs_model = std::fs::canonicalize(model_path).map_err(|e| e.to_string())?;
    let path_str = abs_model.to_string_lossy().into_owned();
    let port = pick_free_port()?;
    let n_ctx_s = n_ctx.to_string();
    let threads_resolved = effective_llama_threads(n_threads);
    let n_threads_s = threads_resolved.to_string();
    let ngl_s = n_gpu_layers.to_string();
    let port_s = port.to_string();
    let work_dir = exe_path
        .parent()
        .ok_or_else(|| "llama-server path has no parent directory".to_string())?
        .to_path_buf();

    let mut cmd = Command::new(&exe_path);
    cmd.current_dir(&work_dir)
        .arg("--no-repack")
        .arg("-m")
        .arg(&path_str)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(&port_s)
        .arg("-c")
        .arg(&n_ctx_s)
        .arg("-t")
        .arg(&n_threads_s)
        .arg("-ngl")
        .arg(&ngl_s)
        .arg("--cache-ram")
        .arg("0")
        .arg("--no-warmup")
        .arg("-np")
        .arg("1");

    if let Some(ref proj) = mmproj {
        if proj.is_file() {
            cmd.arg("--mmproj").arg(proj.to_string_lossy().into_owned());
        }
    }

    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| {
            format!(
                "Failed to start llama-server from {}: {e}",
                exe_path.display()
            )
        })?;

    if let Err(e) = wait_until_llama_ready(port, &mut child).await {
        let _ = child.kill();
        return Err(e);
    }

    {
        let mut g = state.sidecar.lock();
        *g = Some(SidecarRuntime {
            port,
            child,
            model_path: abs_model,
            n_ctx,
            n_threads: threads_resolved,
            n_gpu_layers,
        });
    }
    Ok(())
}

/// Token stream partition for models that expose chain-of-thought separately (OpenAI-style deltas).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamPart {
    Reasoning,
    Content,
}

fn emit_stream_delta<F: Fn(StreamPart, &str) -> Result<(), String>>(
    full_content: &mut String,
    part: StreamPart,
    token_tx: &F,
    c: &Value,
) -> Result<(), String> {
    match c {
        Value::String(s) => {
            if !s.is_empty() {
                if part == StreamPart::Content {
                    full_content.push_str(s);
                }
                token_tx(part, s)?;
            }
        }
        Value::Array(items) => {
            for item in items {
                if let Some(s) = item.get("text").and_then(|x| x.as_str()) {
                    if !s.is_empty() {
                        if part == StreamPart::Content {
                            full_content.push_str(s);
                        }
                        token_tx(part, s)?;
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Sidecar always uses `http://127.0.0.1:PORT` or `http://localhost:PORT`.
fn parse_loopback_http_port(base_url: &str) -> Result<u16, String> {
    let u = base_url.trim_end_matches('/');
    let rest = u.strip_prefix("http://").ok_or_else(|| {
        "chat stream expects an http:// loopback llama-server URL (sidecar)".to_string()
    })?;
    let (host, port_s) = rest
        .rsplit_once(':')
        .ok_or_else(|| "llama-server URL must include an explicit port".to_string())?;
    if !host.eq_ignore_ascii_case("127.0.0.1") && !host.eq_ignore_ascii_case("localhost") {
        return Err(
            "loopback-only: sidecar streaming uses 127.0.0.1 or localhost with a port".into(),
        );
    }
    port_s
        .parse()
        .map_err(|_| format!("invalid TCP port in llama-server URL: {port_s}"))
}

fn trim_crlf_bytes(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

fn trim_ws_bytes(mut s: &[u8]) -> &[u8] {
    while let Some(b) = s.first().copied() {
        if b == b' ' || b == b'\t' {
            s = &s[1..];
        } else {
            break;
        }
    }
    while let Some(b) = s.last().copied() {
        if b == b' ' || b == b'\t' {
            s = &s[..s.len().saturating_sub(1)];
        } else {
            break;
        }
    }
    s
}

/// After each chunked body chunk, the spec requires `\r\n`. Some llama-server builds use `\n`
/// only; `hyper`/`reqwest` then fail with "error decoding response body". We accept either.
async fn skip_chunk_suffix<R: AsyncRead + Unpin>(reader: &mut R) -> Result<(), String> {
    let mut one = [0u8; 1];
    reader
        .read_exact(&mut one)
        .await
        .map_err(|e| format!("chunk suffix: {e}"))?;
    if one[0] == b'\r' {
        reader
            .read_exact(&mut one)
            .await
            .map_err(|e| format!("chunk suffix: {e}"))?;
        if one[0] != b'\n' {
            return Err(format!(
                "chunk framing: expected LF after CR, got byte {}",
                one[0]
            ));
        }
    } else if one[0] != b'\n' {
        return Err(format!(
            "chunk framing: expected CR or LF after chunk data, got byte {}",
            one[0]
        ));
    }
    Ok(())
}

fn apply_sse_json_payload<F: Fn(StreamPart, &str) -> Result<(), String>>(
    payload: &str,
    full_content: &mut String,
    token_tx: &F,
) -> Result<bool, String> {
    if payload == "[DONE]" {
        return Ok(true);
    }
    let v: Value = serde_json::from_str(payload).map_err(|e| {
        format!(
            "invalid JSON in SSE data line ({} bytes): {e}",
            payload.len()
        )
    })?;
    let delta = v
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|ch| ch.get("delta"));
    if let Some(d) = delta {
        for key in ["reasoning_content", "reasoning"] {
            if let Some(r) = d.get(key) {
                emit_stream_delta(full_content, StreamPart::Reasoning, token_tx, r)?;
            }
        }
        if let Some(c) = d.get("content") {
            emit_stream_delta(full_content, StreamPart::Content, token_tx, c)?;
        }
    }
    Ok(false)
}

/// Returns `true` when `[DONE]` was seen.
fn drain_sse_bytes_buffer<F: Fn(StreamPart, &str) -> Result<(), String>>(
    buf: &mut Vec<u8>,
    cancel: &std::sync::atomic::AtomicBool,
    full_content: &mut String,
    token_tx: &F,
) -> Result<bool, String> {
    while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
        if cancel.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let raw: Vec<u8> = buf.drain(..=pos).collect();
        let mut line = &raw[..raw.len().saturating_sub(1)];
        line = line.strip_suffix(b"\r").unwrap_or(line);
        let line = trim_ws_bytes(line);
        if line.is_empty() {
            continue;
        }
        const PREFIX: &[u8] = b"data: ";
        if !line.starts_with(PREFIX) {
            continue;
        }
        let payload_bytes = trim_ws_bytes(&line[PREFIX.len()..]);
        let payload = std::str::from_utf8(payload_bytes)
            .map_err(|e| format!("SSE line is not valid UTF-8: {e}"))?;
        if apply_sse_json_payload(payload, full_content, token_tx)? {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn read_http_body_aggregate<R: AsyncBufRead + AsyncRead + Unpin>(
    reader: &mut R,
    te_chunked: bool,
    content_length: Option<usize>,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    if te_chunked {
        loop {
            let chunk_size = loop {
                let mut size_line = Vec::new();
                let n = reader
                    .read_until(b'\n', &mut size_line)
                    .await
                    .map_err(|e| e.to_string())?;
                if n == 0 {
                    return Err("unexpected EOF inside chunked body (chunk size line)".into());
                }
                let s = trim_crlf_bytes(&size_line);
                if s.is_empty() {
                    continue;
                }
                let hex_part = s
                    .split(|&b| b == b';')
                    .next()
                    .ok_or_else(|| "bad chunk size line".to_string())?;
                let hex_str = std::str::from_utf8(hex_part).map_err(|e| e.to_string())?;
                break usize::from_str_radix(hex_str, 16).map_err(|e| {
                    format!("invalid chunked size {hex_str:?}: {e}")
                })?;
            };

            if chunk_size == 0 {
                loop {
                    let mut trailer = Vec::new();
                    let n = reader
                        .read_until(b'\n', &mut trailer)
                        .await
                        .map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    if trim_crlf_bytes(&trailer).is_empty() {
                        break;
                    }
                }
                break;
            }

            let start = out.len();
            out.resize(start + chunk_size, 0);
            reader
                .read_exact(&mut out[start..])
                .await
                .map_err(|e| format!("chunked body data: {e}"))?;
            skip_chunk_suffix(reader).await?;
        }
        Ok(())
    } else if let Some(len) = content_length {
        let start = out.len();
        out.resize(start + len, 0);
        reader
            .read_exact(&mut out[start..])
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    } else {
        reader.read_to_end(out).await.map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Raw HTTP/1.1 + lenient chunked decode for localhost llama-server. Avoids `reqwest`/`hyper`
/// strict framing issues (and mis-split UTF-8 across TCP chunks) when consuming SSE.
async fn stream_chat_loopback_tcp<F: Fn(StreamPart, &str) -> Result<(), String>>(
    port: u16,
    body_vec: &[u8],
    cancel: &std::sync::atomic::AtomicBool,
    token_tx: F,
) -> Result<String, String> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut tcp = TcpStream::connect(addr)
        .await
        .map_err(|e| format!("connect to llama-server at {addr}: {e}"))?;
    let _ = tcp.set_nodelay(true);

    let head = format!(
        "POST /v1/chat/completions HTTP/1.1\r\n\
Host: 127.0.0.1:{port}\r\n\
Content-Type: application/json\r\n\
Content-Length: {}\r\n\
Accept: text/event-stream\r\n\
Connection: close\r\n\
\r\n",
        body_vec.len()
    );
    tcp.write_all(head.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    tcp.write_all(body_vec)
        .await
        .map_err(|e| e.to_string())?;
    tcp.flush().await.map_err(|e| e.to_string())?;

    let mut reader = BufReader::new(&mut tcp);

    let mut first = true;
    let mut status_u16 = 0u16;
    let mut te_chunked = false;
    let mut content_length: Option<usize> = None;

    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("unexpected EOF before HTTP response headers finished".into());
        }
        if line.trim().is_empty() {
            break;
        }
        if first {
            let mut parts = line.split_whitespace();
            let _ver = parts.next();
            status_u16 = parts
                .next()
                .ok_or_else(|| format!("bad status line: {line:?}"))?
                .parse()
                .map_err(|_| format!("bad status line: {line:?}"))?;
            first = false;
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        if name.eq_ignore_ascii_case("transfer-encoding")
            && value.to_ascii_lowercase().contains("chunked")
        {
            te_chunked = true;
        }
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value.parse().ok();
        }
    }

    if status_u16 != 200 {
        let mut err_body = Vec::new();
        read_http_body_aggregate(&mut reader, te_chunked, content_length, &mut err_body).await?;
        let msg = String::from_utf8_lossy(&err_body);
        return Err(format!("llama-server HTTP {status_u16}: {msg}"));
    }

    let mut full_content = String::new();
    let mut sse_buf: Vec<u8> = Vec::new();

    if te_chunked {
        loop {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let chunk_size = loop {
                let mut size_line = Vec::new();
                let n = reader
                    .read_until(b'\n', &mut size_line)
                    .await
                    .map_err(|e| e.to_string())?;
                if n == 0 {
                    return Err("unexpected EOF in streamed chunked response (size line)".into());
                }
                let s = trim_crlf_bytes(&size_line);
                if s.is_empty() {
                    continue;
                }
                let hex_part = s
                    .split(|&b| b == b';')
                    .next()
                    .ok_or_else(|| "bad chunk size line".to_string())?;
                let hex_str = std::str::from_utf8(hex_part).map_err(|e| e.to_string())?;
                break usize::from_str_radix(hex_str, 16).map_err(|e| {
                    format!("invalid streamed chunk size {hex_str:?}: {e}")
                })?;
            };

            if chunk_size == 0 {
                loop {
                    let mut trailer = Vec::new();
                    let n = reader
                        .read_until(b'\n', &mut trailer)
                        .await
                        .map_err(|e| e.to_string())?;
                    if n == 0 || trim_crlf_bytes(&trailer).is_empty() {
                        break;
                    }
                }
                break;
            }

            let start = sse_buf.len();
            sse_buf.resize(start + chunk_size, 0);
            reader
                .read_exact(&mut sse_buf[start..])
                .await
                .map_err(|e| format!("stream chunk data: {e}"))?;
            skip_chunk_suffix(&mut reader).await?;

            if drain_sse_bytes_buffer(&mut sse_buf, cancel, &mut full_content, &token_tx)? {
                return Ok(full_content);
            }
        }
    } else if let Some(len) = content_length {
        sse_buf.resize(len, 0);
        reader
            .read_exact(&mut sse_buf)
            .await
            .map_err(|e| e.to_string())?;
        if drain_sse_bytes_buffer(&mut sse_buf, cancel, &mut full_content, &token_tx)? {
            return Ok(full_content);
        }
    } else {
        reader
            .read_to_end(&mut sse_buf)
            .await
            .map_err(|e| e.to_string())?;
        if drain_sse_bytes_buffer(&mut sse_buf, cancel, &mut full_content, &token_tx)? {
            return Ok(full_content);
        }
    }

    Ok(full_content)
}

/// `thinking_enabled` maps to Qwen-style `chat_template_kwargs.enable_thinking` (llama-server).
pub async fn stream_chat_completion(
    base_url: &str,
    messages: &[(String, Value)],
    temperature: f32,
    top_p: f32,
    max_tokens: i32,
    thinking_enabled: bool,
    cancel: &std::sync::atomic::AtomicBool,
    token_tx: impl Fn(StreamPart, &str) -> Result<(), String>,
) -> Result<String, String> {
    let openai_messages: Vec<Value> = messages
        .iter()
        .map(|(role, content)| {
            serde_json::json!({
                "role": role,
                "content": content.clone(),
            })
        })
        .collect();

    let body = serde_json::json!({
        "model": "local",
        "messages": openai_messages,
        "temperature": temperature,
        "top_p": top_p,
        "max_tokens": max_tokens,
        "stream": true,
        "chat_template_kwargs": {
            "enable_thinking": thinking_enabled,
        },
    });

    let body_vec = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
    let port = parse_loopback_http_port(base_url)?;
    stream_chat_loopback_tcp(port, &body_vec, cancel, token_tx).await
}

/// Non-streaming completion for tool-agent intermediate turns.
pub async fn complete_chat_completion(
    base_url: &str,
    messages: &[(String, Value)],
    temperature: f32,
    top_p: f32,
    max_tokens: i32,
    thinking_enabled: bool,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<String, String> {
    let openai_messages: Vec<Value> = messages
        .iter()
        .map(|(role, content)| {
            serde_json::json!({
                "role": role,
                "content": content.clone(),
            })
        })
        .collect();

    let body = serde_json::json!({
        "model": "local",
        "messages": openai_messages,
        "temperature": temperature,
        "top_p": top_p,
        "max_tokens": max_tokens,
        "stream": false,
        "chat_template_kwargs": {
            "enable_thinking": thinking_enabled,
        },
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));
    let res = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if cancel.load(Ordering::Relaxed) {
        return Ok(String::new());
    }
    if !res.status().is_success() {
        let txt = res.text().await.unwrap_or_default();
        return Err(format!(
            "llama-server error: {}",
            txt.chars().take(800).collect::<String>()
        ));
    }
    let v: Value = res.json().await.map_err(|e| e.to_string())?;
    let content = v
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|ch| ch.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| match c {
            Value::String(s) => Some(s.clone()),
            Value::Array(parts) => {
                let mut out = String::new();
                for p in parts {
                    if let Some(t) = p.get("text").and_then(|x| x.as_str()) {
                        out.push_str(t);
                    }
                }
                if out.is_empty() { None } else { Some(out) }
            }
            _ => None,
        })
        .unwrap_or_default();
    Ok(content)
}
