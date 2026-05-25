use serde::Serialize;
use std::thread;
use sysinfo::{
    CpuRefreshKind, Disks, MemoryRefreshKind, RefreshKind, System, MINIMUM_CPU_UPDATE_INTERVAL,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuMetrics {
    pub name: String,
    pub usage_pct: Option<f32>,
    pub vram_used_mb: Option<u64>,
    pub vram_total_mb: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskMountMetrics {
    pub name: String,
    pub mount: String,
    pub total_gb: f64,
    pub free_gb: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSnapshot {
    pub host_name: Option<String>,
    pub os_version: Option<String>,
    pub cpu_name: String,
    pub cpu_cores: usize,
    pub cpu_usage_pct: f32,
    pub ram_used_gb: f64,
    pub ram_total_gb: f64,
    pub swap_used_gb: f64,
    pub swap_total_gb: f64,
    pub gpus: Vec<GpuMetrics>,
    pub disks: Vec<DiskMountMetrics>,
}

pub fn capture_system_snapshot() -> SystemSnapshot {
    let mut sys = System::new_with_specifics(
        RefreshKind::new()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything()),
    );
    thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_cpu_usage();
    sys.refresh_memory();

    let cpu_usage_pct = sys.global_cpu_usage();
    let cpu_name = sys
        .cpus()
        .first()
        .map(|c| c.brand().trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "CPU".into());
    let cpu_cores = sys.cpus().len().max(1);

    let ram_total = sys.total_memory();
    let ram_used = sys.used_memory();
    let swap_total = sys.total_swap();
    let swap_used = sys.used_swap();

    let gb = |b: u64| b as f64 / (1024.0 * 1024.0 * 1024.0);

    let mut disks_out = Vec::new();
    let disks = Disks::new_with_refreshed_list();
    for disk in disks.list() {
        let total = disk.total_space();
        let avail = disk.available_space();
        if total == 0 {
            continue;
        }
        let name = disk.name().to_string_lossy().to_string();
        let mount = disk.mount_point().to_string_lossy().to_string();
        disks_out.push(DiskMountMetrics {
            name,
            mount,
            total_gb: total as f64 / (1024.0 * 1024.0 * 1024.0),
            free_gb: avail as f64 / (1024.0 * 1024.0 * 1024.0),
        });
    }

    SystemSnapshot {
        host_name: System::host_name(),
        os_version: System::long_os_version(),
        cpu_name,
        cpu_cores,
        cpu_usage_pct,
        ram_used_gb: gb(ram_used),
        ram_total_gb: gb(ram_total),
        swap_used_gb: gb(swap_used),
        swap_total_gb: gb(swap_total),
        gpus: crate::gpu_probe::probe_gpus()
            .into_iter()
            .map(|g| GpuMetrics {
                name: g.name,
                usage_pct: g.usage_pct,
                vram_used_mb: g.vram_used_mb,
                vram_total_mb: g.vram_total_mb,
            })
            .collect(),
        disks: disks_out,
    }
}
