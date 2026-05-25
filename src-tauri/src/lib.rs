pub mod chat_tools;
pub mod api_server;
pub mod cloud_infer;
pub mod commands;
pub mod data_root;
pub mod db;
pub mod download_manager;
pub mod engine;
pub mod fs_util;
pub mod mmproj_detect;
pub mod model_knowledge;
pub mod huggingface;
pub mod state;
pub mod gpu_probe;
pub mod image_gen;
#[cfg(feature = "llama-sidecar")]
pub mod llama_runtime;
pub mod system_metrics;
pub mod tool_agent;

#[cfg(all(feature = "llama-sidecar", feature = "llama-engine"))]
compile_error!(
    "Choose one inference backend: default `llama-sidecar`, or `--no-default-features --features llama-engine` \
     (in-process; needs LLVM on Windows)."
);

#[cfg(feature = "llama-sidecar")]
pub mod llama_sidecar;

use state::AppState;
use std::path::PathBuf;
use tauri::Manager;

pub struct HeadlessServerOptions {
    pub host: String,
    pub port: u16,
    pub data_dir: PathBuf,
    pub models_dir: Option<PathBuf>,
    pub auth_mode: String,
    pub api_key: String,
    #[cfg(feature = "llama-sidecar")]
    pub runtime_dir: Option<PathBuf>,
}

pub async fn run_headless_server(options: HeadlessServerOptions) -> Result<(), String> {
    std::fs::create_dir_all(&options.data_dir).map_err(|e| e.to_string())?;
    let models_dir = options
        .models_dir
        .clone()
        .unwrap_or_else(|| options.data_dir.join("models"));
    std::fs::create_dir_all(&models_dir).map_err(|e| e.to_string())?;
    let db_path = options.data_dir.join("localmod.sqlite");
    let conn = db::open(&db_path).map_err(|e| e.to_string())?;
    let state = AppState::new(
        conn,
        db_path,
        models_dir,
        options.data_dir.clone(),
        options.data_dir.clone(),
    )
    .map_err(|e| e.to_string())?;

    #[cfg(feature = "llama-sidecar")]
    {
        *state.llama_runtime_dir.lock() = options.runtime_dir.clone();
        match llama_runtime::runtime_validation_headless(&state) {
            Ok(path) => println!("[localmod-server] llama runtime: {path}"),
            Err(e) => eprintln!("[localmod-server] warning: {e}"),
        }
    }

    println!(
        "[localmod-server] data dir: {}",
        options.data_dir.to_string_lossy()
    );
    println!(
        "[localmod-server] models dir: {}",
        state.models_dir.to_string_lossy()
    );

    api_server::serve_headless(
        state,
        options.host,
        options.port,
        options.auth_mode,
        options.api_key,
    )
    .await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Loads `.env` from the current working directory (e.g. project root in dev) so maintainers can set HUGGINGFACE_TOKEN without exposing it in the UI.
    let _ = dotenvy::dotenv();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let resolver = app.path();
            let bootstrap_dir = resolver.app_data_dir().map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&bootstrap_dir).map_err(|e| e.to_string())?;
            let app_data_dir = data_root::resolve_work_dir(&bootstrap_dir)?;
            let models_dir = app_data_dir.join("models");
            std::fs::create_dir_all(&models_dir).map_err(|e| e.to_string())?;
            let db_path = app_data_dir.join("localmod.sqlite");
            let conn = db::open(&db_path).map_err(|e| e.to_string())?;

            let st = AppState::new(
                conn,
                db_path,
                models_dir,
                app_data_dir,
                bootstrap_dir,
            )
            .map_err(|e| e.to_string())?;
            let auto_start_api = api_server::settings_from_db(&st);
            app.manage(st);
            if auto_start_api.enabled {
                let handle = app.handle().clone();
                let state = app.state::<AppState>().inner().clone();
                tauri::async_runtime::spawn(async move {
                    let _ = api_server::start(
                        handle,
                        &state,
                        auto_start_api.host,
                        auto_start_api.port,
                        auto_start_api.auth_mode,
                        auto_start_api.api_key,
                    )
                    .await;
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::set_setting,
            commands::get_api_server_settings,
            commands::get_api_server_status,
            commands::get_headless_server_status,
            commands::get_llm_runtime_status,
            commands::validate_llama_runtime,
            commands::start_api_server,
            commands::stop_api_server,
            commands::start_headless_server,
            commands::stop_headless_server,
            commands::get_dashboard,
            commands::get_system_snapshot,
            commands::get_paths,
            commands::list_models,
            commands::get_cloud_provider_configs,
            commands::set_cloud_provider_config,
            commands::chat_image_gen_available,
            commands::read_generated_image_data_url,
            commands::export_generated_image,
            commands::register_model,
            commands::update_model,
            commands::list_huggingface_gguf_files,
            commands::hf_download_start_auto,
            commands::hf_download_start_manual,
            commands::hf_download_list,
            commands::hf_download_pause,
            commands::hf_download_resume,
            commands::hf_download_cancel,
            commands::hf_download_dismiss,
            commands::get_model_knowledge,
            commands::delete_model,
            commands::list_chats,
            commands::create_chat,
            commands::list_projects,
            commands::create_project,
            commands::update_project,
            commands::delete_project,
            commands::get_chat_context_usage,
            commands::rename_chat,
            commands::set_chat_model,
            commands::set_chat_system_prompt,
            commands::delete_chat,
            commands::list_messages,
            commands::load_llm,
            commands::run_model_benchmark,
            commands::unload_llm,
            commands::get_loaded_llm,
            commands::send_chat_message,
            commands::regenerate_assistant_message,
            commands::delete_message,
            commands::update_message,
            commands::stop_generation,
            commands::list_context_documents,
            commands::get_context_text_for_edit,
            commands::update_context_text,
            commands::add_context_from_path,
            commands::add_context_text,
            commands::delete_context_document,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
