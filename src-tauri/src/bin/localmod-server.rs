use localmod_lib::HeadlessServerOptions;
use std::path::PathBuf;

#[derive(Debug)]
struct ServerArgs {
    host: String,
    port: u16,
    data_dir: PathBuf,
    models_dir: Option<PathBuf>,
    auth_mode: String,
    api_key: String,
    runtime_dir: Option<PathBuf>,
}

fn print_help() {
    println!(
        "LocalMOD headless OpenAI-compatible API server\n\n\
Usage:\n  localmod-server [options]\n\n\
Options:\n  --host <host>            Bind host (default: 127.0.0.1; use 0.0.0.0 for server/LAN)\n  --port <port>            Bind port (default: 11435)\n  --data-dir <path>        Data directory for localmod.sqlite, models, context\n  --models-dir <path>      Override models directory (default: <data-dir>/models)\n  --auth none|bearer       Auth mode (default: none)\n  --api-key <key>          Bearer token when --auth bearer\n  --runtime-dir <path>     Folder containing llama-server.exe and DLLs\n  --help                   Show this help\n\n\
Examples:\n  localmod-server --host 0.0.0.0 --port 11435 --data-dir D:\\LocalMOD --auth bearer --api-key secret\n  localmod-server --data-dir ./localmod-data --runtime-dir ./llama-runtime\n"
    );
}

fn default_data_dir() -> Result<PathBuf, String> {
    if let Ok(v) = std::env::var("LOCALMOD_DATA_DIR") {
        let p = PathBuf::from(v);
        if !p.as_os_str().is_empty() {
            return Ok(p);
        }
    }
    let base = std::env::current_dir().map_err(|e| e.to_string())?;
    Ok(base.join("localmod-data"))
}

fn parse_args() -> Result<Option<ServerArgs>, String> {
    let mut args = std::env::args().skip(1);
    let mut out = ServerArgs {
        host: std::env::var("LOCALMOD_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
        port: std::env::var("LOCALMOD_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(11435),
        data_dir: default_data_dir()?,
        models_dir: std::env::var("LOCALMOD_MODELS_DIR").ok().map(PathBuf::from),
        auth_mode: std::env::var("LOCALMOD_AUTH").unwrap_or_else(|_| "none".into()),
        api_key: std::env::var("LOCALMOD_API_KEY").unwrap_or_default(),
        runtime_dir: std::env::var("LOCALMOD_RUNTIME_DIR").ok().map(PathBuf::from),
    };

    while let Some(arg) = args.next() {
        let mut next_value = || {
            args.next()
                .ok_or_else(|| format!("Missing value after {arg}"))
        };
        match arg.as_str() {
            "--help" | "-h" => return Ok(None),
            "--host" => out.host = next_value()?,
            "--port" => {
                out.port = next_value()?
                    .parse()
                    .map_err(|_| "--port must be a number between 1 and 65535".to_string())?;
            }
            "--data-dir" => out.data_dir = PathBuf::from(next_value()?),
            "--models-dir" => out.models_dir = Some(PathBuf::from(next_value()?)),
            "--auth" => out.auth_mode = next_value()?,
            "--api-key" => out.api_key = next_value()?,
            "--runtime-dir" => out.runtime_dir = Some(PathBuf::from(next_value()?)),
            other => return Err(format!("Unknown argument: {other}. Use --help.")),
        }
    }

    out.auth_mode = if out.auth_mode == "bearer" {
        "bearer".into()
    } else {
        "none".into()
    };
    if out.auth_mode == "bearer" && out.api_key.trim().is_empty() {
        return Err("--auth bearer requires --api-key or LOCALMOD_API_KEY.".into());
    }
    Ok(Some(out))
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("[localmod-server] {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let _ = dotenvy::dotenv();
    let Some(args) = parse_args()? else {
        print_help();
        return Ok(());
    };

    localmod_lib::run_headless_server(HeadlessServerOptions {
        host: args.host,
        port: args.port,
        data_dir: args.data_dir,
        models_dir: args.models_dir,
        auth_mode: args.auth_mode,
        api_key: args.api_key,
        #[cfg(feature = "llama-sidecar")]
        runtime_dir: args.runtime_dir,
    })
    .await
}
