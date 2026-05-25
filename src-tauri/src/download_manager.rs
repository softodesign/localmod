use crate::huggingface::HfDownloadControl;
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DownloadJobStatus {
    Queued,
    Running,
    Paused,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DownloadJobDto {
    pub id: String,
    pub title: String,
    pub status: DownloadJobStatus,
    pub message: String,
    /// 0–100 overall
    pub progress: f64,
    pub bytes_downloaded: u64,
    pub bytes_total: Option<u64>,
    pub current_file: Option<String>,
    pub file_index: u32,
    pub file_count: u32,
    pub registered_model_id: Option<String>,
    pub error: Option<String>,
}

struct DownloadJobInner {
    dto: DownloadJobDto,
    control: HfDownloadControl,
    /// Part-written files to delete on cancel / failure
    partial_paths: Mutex<Vec<PathBuf>>,
}

pub struct DownloadManager {
    jobs: Mutex<HashMap<String, DownloadJobInner>>,
}

impl DownloadManager {
    pub fn new() -> Self {
        Self {
            jobs: Mutex::new(HashMap::new()),
        }
    }

    pub fn create_job(&self, title: String) -> String {
        let id = Uuid::new_v4().to_string();
        let inner = DownloadJobInner {
            dto: DownloadJobDto {
                id: id.clone(),
                title,
                status: DownloadJobStatus::Queued,
                message: String::new(),
                progress: 0.0,
                bytes_downloaded: 0,
                bytes_total: None,
                current_file: None,
                file_index: 0,
                file_count: 1,
                registered_model_id: None,
                error: None,
            },
            control: HfDownloadControl::new(),
            partial_paths: Mutex::new(Vec::new()),
        };
        self.jobs.lock().insert(id.clone(), inner);
        id
    }

    pub fn set_running(&self, id: &str) {
        self.update_job(id, |d| {
            d.status = DownloadJobStatus::Running;
        });
    }

    pub fn register_partial(&self, id: &str, path: PathBuf) {
        if let Some(j) = self.jobs.lock().get_mut(id) {
            j.partial_paths.lock().push(path);
        }
    }

    /// Stop tracking paths that finished successfully (so cleanup won’t delete them).
    pub fn release_partial(&self, id: &str, path: &std::path::Path) {
        if let Some(j) = self.jobs.lock().get_mut(id) {
            j.partial_paths.lock().retain(|p| p.as_path() != path);
        }
    }

    pub fn cleanup_partials(&self, id: &str) {
        let paths: Vec<PathBuf> = {
            let mut m = self.jobs.lock();
            if let Some(j) = m.get_mut(id) {
                std::mem::take(&mut *j.partial_paths.lock())
            } else {
                vec![]
            }
        };
        for p in paths {
            let _ = std::fs::remove_file(&p);
        }
    }

    pub fn update_job(&self, id: &str, f: impl FnOnce(&mut DownloadJobDto)) {
        if let Some(j) = self.jobs.lock().get_mut(id) {
            f(&mut j.dto);
        }
    }

    /// Latest pause/cancel flags stored on the job (same atomics as `pause` / `resume` / `cancel`).
    pub fn job_pause_and_cancel(&self, id: &str) -> Option<(bool, bool)> {
        self.jobs.lock().get(id).map(|j| {
            (
                j.control.pause.load(Ordering::SeqCst),
                j.control.cancel.load(Ordering::SeqCst),
            )
        })
    }

    pub fn list_jobs(&self) -> Vec<DownloadJobDto> {
        let m = self.jobs.lock();
        let mut v: Vec<_> = m.values().map(|j| j.dto.clone()).collect();
        v.sort_by(|a, b| b.id.cmp(&a.id));
        v
    }

    pub fn pause(&self, id: &str) -> Result<(), String> {
        let mut m = self.jobs.lock();
        let j = m.get_mut(id).ok_or_else(|| "Job not found.".to_string())?;
        match j.dto.status {
            DownloadJobStatus::Running => {}
            _ => return Err("Only an active download can pause.".into()),
        }
        j.control.pause.store(true, Ordering::SeqCst);
        j.dto.status = DownloadJobStatus::Paused;
        j.dto.message = "Paused".into();
        Ok(())
    }

    pub fn resume(&self, id: &str) -> Result<(), String> {
        let mut m = self.jobs.lock();
        let j = m.get_mut(id).ok_or_else(|| "Job not found.".to_string())?;
        if j.dto.status != DownloadJobStatus::Paused {
            return Err("Not paused.".into());
        }
        j.control.pause.store(false, Ordering::SeqCst);
        j.dto.status = DownloadJobStatus::Running;
        if j.dto.message == "Paused" {
            j.dto.message = "Resuming…".into();
        }
        Ok(())
    }

    pub fn cancel(&self, id: &str) -> Result<(), String> {
        let mut m = self.jobs.lock();
        let j = m.get_mut(id).ok_or_else(|| "Job not found.".to_string())?;
        match j.dto.status {
            DownloadJobStatus::Completed | DownloadJobStatus::Cancelled | DownloadJobStatus::Failed => {
                return Err("Already finished.".into());
            }
            _ => {}
        }
        j.control.cancel.store(true, Ordering::SeqCst);
        j.control.pause.store(false, Ordering::SeqCst);
        Ok(())
    }

    pub fn mark_completed(&self, id: &str, registered_model_id: String) {
        self.cleanup_partials(id);
        self.update_job(id, |d| {
            d.status = DownloadJobStatus::Completed;
            d.progress = 100.0;
            d.registered_model_id = Some(registered_model_id);
            d.message = "Done".into();
            d.bytes_total = d.bytes_total.or(Some(d.bytes_downloaded));
        });
    }

    pub fn mark_failed(&self, id: &str, err: String) {
        self.cleanup_partials(id);
        self.update_job(id, |d| {
            d.status = DownloadJobStatus::Failed;
            d.error = Some(err.clone());
            d.message = err;
        });
    }

    pub fn mark_cancelled(&self, id: &str) {
        self.cleanup_partials(id);
        self.update_job(id, |d| {
            d.status = DownloadJobStatus::Cancelled;
            d.message = "Cancelled".into();
            d.error = None;
        });
    }

    /// Remove a terminal job from the list (UI dismiss).
    pub fn dismiss(&self, id: &str) -> Result<(), String> {
        let mut m = self.jobs.lock();
        let j = m.get(id).ok_or_else(|| "Job not found.".to_string())?;
        match j.dto.status {
            DownloadJobStatus::Completed | DownloadJobStatus::Cancelled | DownloadJobStatus::Failed => {}
            _ => return Err("Can only dismiss finished jobs.".into()),
        }
        m.remove(id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::DownloadJobStatus;

    #[test]
    fn download_job_status_json_matches_frontend() {
        assert_eq!(
            serde_json::to_string(&DownloadJobStatus::Paused).unwrap(),
            "\"paused\""
        );
        assert_eq!(
            serde_json::to_string(&DownloadJobStatus::Running).unwrap(),
            "\"running\""
        );
    }
}
