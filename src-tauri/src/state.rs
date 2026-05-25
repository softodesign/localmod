use anyhow::Result;
use parking_lot::Mutex;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::oneshot;
#[cfg(feature = "llama-engine")]
use llama_cpp_2::llama_backend::LlamaBackend;
#[cfg(feature = "llama-sidecar")]
use std::process::Child as LlamaChild;
#[cfg(feature = "llama-sidecar")]
use tokio::sync::Mutex as TokioMutex;

#[cfg(feature = "llama-sidecar")]
pub struct SidecarRuntime {
    pub port: u16,
    pub child: LlamaChild,
    pub model_path: PathBuf,
    /// Values actually passed to llama-server (threads after `effective_llama_threads`).
    pub n_ctx: u32,
    pub n_threads: u32,
    pub n_gpu_layers: u32,
}

pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
    pub db_path: PathBuf,
    /// Effective data directory for this run (database, default models, context, etc.).
    pub app_data_dir: PathBuf,
    /// Tauri’s default app data folder — holds `data_root.json` when using a custom data directory.
    pub bootstrap_dir: PathBuf,
    pub models_dir: PathBuf,
    #[cfg(feature = "llama-engine")]
    pub backend: Arc<LlamaBackend>,
    #[cfg(feature = "llama-sidecar")]
    pub sidecar: Arc<Mutex<Option<SidecarRuntime>>>,
    /// Ensures only one sidecar restart runs at a time (avoids overlapping kills / stale ports).
    #[cfg(feature = "llama-sidecar")]
    pub load_llm_sidecar_lock: Arc<TokioMutex<()>>,
    #[cfg(feature = "llama-sidecar")]
    pub llama_runtime: Arc<crate::llama_runtime::LlamaRuntimeState>,
    #[cfg(feature = "llama-sidecar")]
    pub llama_runtime_dir: Arc<Mutex<Option<PathBuf>>>,
    pub loaded: Arc<crate::engine::LoadedSlot>,
    pub cancel: Arc<AtomicBool>,
    pub downloads: Arc<crate::download_manager::DownloadManager>,
    pub api_server: Arc<Mutex<Option<ApiServerRuntime>>>,
}

pub struct ApiServerRuntime {
    pub host: String,
    pub port: u16,
    pub auth_mode: String,
    pub shutdown_tx: Option<oneshot::Sender<()>>,
}

impl Clone for AppState {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            db_path: self.db_path.clone(),
            app_data_dir: self.app_data_dir.clone(),
            bootstrap_dir: self.bootstrap_dir.clone(),
            models_dir: self.models_dir.clone(),
            #[cfg(feature = "llama-engine")]
            backend: self.backend.clone(),
            #[cfg(feature = "llama-sidecar")]
            sidecar: self.sidecar.clone(),
            #[cfg(feature = "llama-sidecar")]
            load_llm_sidecar_lock: self.load_llm_sidecar_lock.clone(),
            #[cfg(feature = "llama-sidecar")]
            llama_runtime: self.llama_runtime.clone(),
            #[cfg(feature = "llama-sidecar")]
            llama_runtime_dir: self.llama_runtime_dir.clone(),
            loaded: self.loaded.clone(),
            cancel: self.cancel.clone(),
            downloads: self.downloads.clone(),
            api_server: self.api_server.clone(),
        }
    }
}

impl AppState {
    pub fn new(
        mut conn: Connection,
        db_path: PathBuf,
        models_dir: PathBuf,
        app_data_dir: PathBuf,
        bootstrap_dir: PathBuf,
    ) -> Result<Self> {
        crate::db::migrate(&mut conn)?;
        crate::db::ensure_defaults(&conn, &models_dir)?;
        #[cfg(feature = "llama-engine")]
        let backend = crate::engine::backend_arc()?;
        #[cfg(feature = "llama-sidecar")]
        let sidecar = Arc::new(Mutex::new(None));
        #[cfg(feature = "llama-sidecar")]
        let load_llm_sidecar_lock = Arc::new(TokioMutex::new(()));
        #[cfg(feature = "llama-sidecar")]
        let llama_runtime = Arc::new(crate::llama_runtime::LlamaRuntimeState::new());
        #[cfg(feature = "llama-sidecar")]
        let llama_runtime_dir = Arc::new(Mutex::new(None));
        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
            db_path,
            app_data_dir,
            bootstrap_dir,
            models_dir,
            #[cfg(feature = "llama-engine")]
            backend,
            #[cfg(feature = "llama-sidecar")]
            sidecar,
            #[cfg(feature = "llama-sidecar")]
            load_llm_sidecar_lock,
            #[cfg(feature = "llama-sidecar")]
            llama_runtime,
            #[cfg(feature = "llama-sidecar")]
            llama_runtime_dir,
            loaded: Arc::new(crate::engine::new_loaded_slot()),
            cancel: Arc::new(AtomicBool::new(false)),
            downloads: Arc::new(crate::download_manager::DownloadManager::new()),
            api_server: Arc::new(Mutex::new(None)),
        })
    }
}
