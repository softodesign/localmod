#[derive(Debug, Clone)]
pub struct GpuProbe {
    pub name: String,
    pub usage_pct: Option<f32>,
    pub vram_used_mb: Option<u64>,
    pub vram_total_mb: Option<u64>,
}

#[cfg(windows)]
mod windows_impl {
    use super::GpuProbe;
    use std::collections::HashSet;
    use std::thread;
    use std::time::Duration;
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIFactory1,
    };
    use windows::Win32::System::Performance::{
        PdhAddCounterW, PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData,
        PdhGetFormattedCounterArrayW, PdhOpenQueryW, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE,
        PDH_MORE_DATA,
    };

    const PDH_CSTATUS_VALID_DATA: u32 = 0;

    struct DxgiAdapter {
        name: String,
        luid_key: String,
        total_vram_bytes: u64,
    }

    fn luid_key(high: u32, low: u32) -> String {
        format!("luid_0x{high:08x}_0x{low:08x}")
    }

    fn pdh_status_ok(status: u32) -> bool {
        status == PDH_CSTATUS_VALID_DATA || status == PDH_MORE_DATA
    }

    fn instance_matches_luid(inst: &str, luid: &str) -> bool {
        let inst_l = inst.to_lowercase();
        let luid_l = luid.to_lowercase();
        inst_l.starts_with(&luid_l) || inst_l.contains(&format!("_{luid_l}_"))
    }

    fn add_pdh_counter(query: isize, path: &str) -> Option<isize> {
        let path_w: Vec<u16> = path.encode_utf16().chain([0]).collect();
        unsafe {
            let mut counter: isize = 0;
            if PdhAddEnglishCounterW(query, PCWSTR(path_w.as_ptr()), 0, &mut counter) == 0 {
                return Some(counter);
            }
            if PdhAddCounterW(query, PCWSTR(path_w.as_ptr()), 0, &mut counter) == 0 {
                return Some(counter);
            }
        }
        None
    }

    /// Read all instances for a wildcard PDH path (e.g. `\GPU Engine(*)\Utilization Percentage`).
    pub(crate) fn pdh_wildcard_values(path: &str, two_samples: bool) -> Vec<(String, f64)> {
        unsafe {
            let mut query: isize = 0;
            if PdhOpenQueryW(None, 0, &mut query) != 0 {
                return Vec::new();
            }
            let Some(counter) = add_pdh_counter(query, path) else {
                let _ = PdhCloseQuery(query);
                return Vec::new();
            };
            if PdhCollectQueryData(query) != 0 {
                let _ = PdhCloseQuery(query);
                return Vec::new();
            }
            if two_samples {
                thread::sleep(Duration::from_millis(400));
                if PdhCollectQueryData(query) != 0 {
                    let _ = PdhCloseQuery(query);
                    return Vec::new();
                }
            }

            let mut buf_size: u32 = 0;
            let mut item_count: u32 = 0;
            let status = PdhGetFormattedCounterArrayW(
                counter,
                PDH_FMT_DOUBLE,
                &mut buf_size,
                &mut item_count,
                None,
            );
            if !pdh_status_ok(status) || buf_size == 0 || item_count == 0 {
                let _ = PdhCloseQuery(query);
                return Vec::new();
            }

            let mut buffer = vec![0u8; buf_size as usize];
            let items = buffer.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            let mut buf_size2 = buf_size;
            let mut item_count2 = item_count;
            if PdhGetFormattedCounterArrayW(
                counter,
                PDH_FMT_DOUBLE,
                &mut buf_size2,
                &mut item_count2,
                Some(items),
            ) != 0
            {
                let _ = PdhCloseQuery(query);
                return Vec::new();
            }

            let mut out = Vec::with_capacity(item_count2 as usize);
            for i in 0..item_count2 as isize {
                let item = &*items.offset(i);
                let name = if item.szName.is_null() {
                    String::new()
                } else {
                    item.szName.to_string().unwrap_or_default()
                };
                out.push((name, item.FmtValue.Anonymous.doubleValue));
            }
            let _ = PdhCloseQuery(query);
            out
        }
    }

    fn max_engine_usage_for_luid(luid: &str, values: &[(String, f64)]) -> Option<f32> {
        let mut max_pct = 0.0f64;
        let mut found = false;
        for (inst, v) in values {
            if instance_matches_luid(inst, luid) && inst.contains("engtype_3D") {
                found = true;
                max_pct = max_pct.max(*v);
            }
        }
        found.then(|| max_pct.clamp(0.0, 100.0) as f32)
    }

    fn memory_bytes_for_luid(
        luid: &str,
        dedicated: &[(String, f64)],
        shared: &[(String, f64)],
    ) -> Option<u64> {
        for list in [dedicated, shared] {
            let matches: Vec<_> = list
                .iter()
                .filter(|(inst, _)| instance_matches_luid(inst, luid))
                .collect();
            if matches.is_empty() {
                continue;
            }
            let phys: Vec<_> = matches
                .iter()
                .filter(|(inst, _)| inst.contains("_phys_"))
                .copied()
                .collect();
            let pick = if phys.is_empty() { matches } else { phys };
            let bytes: u64 = pick.iter().map(|(_, v)| v.round().max(0.0) as u64).sum();
            return Some(bytes);
        }
        None
    }

    fn enumerate_dxgi_adapters() -> Vec<DxgiAdapter> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        unsafe {
            let factory: IDXGIFactory1 = match CreateDXGIFactory1() {
                Ok(f) => f,
                Err(_) => return out,
            };
            let mut index = 0u32;
            loop {
                let adapter = match factory.EnumAdapters1(index) {
                    Ok(a) => a,
                    Err(_) => break,
                };
                index += 1;
                let desc = match adapter.GetDesc1() {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                if (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
                    continue;
                }
                let name = String::from_utf16_lossy(
                    &desc
                        .Description
                        .iter()
                        .take_while(|&&c| c != 0)
                        .copied()
                        .collect::<Vec<_>>(),
                )
                .trim()
                .to_string();
                if name.is_empty() {
                    continue;
                }
                let luid = desc.AdapterLuid;
                let key = luid_key(luid.HighPart as u32, luid.LowPart as u32);
                if !seen.insert(key.clone()) {
                    continue;
                }
                out.push(DxgiAdapter {
                    name,
                    luid_key: key,
                    total_vram_bytes: desc.DedicatedVideoMemory as u64,
                });
            }
        }
        out
    }

    fn engine_usage_values() -> Vec<(String, f64)> {
        const PATHS: &[&str] = &[
            r"\GPU Engine(*engtype_3D*)\Utilization Percentage",
            r"\GPU Engine(*)\Utilization Percentage",
        ];
        for path in PATHS {
            let vals = pdh_wildcard_values(path, true);
            if !vals.is_empty() {
                return vals;
            }
        }
        Vec::new()
    }

    pub fn probe_gpus() -> Vec<GpuProbe> {
        let adapters = enumerate_dxgi_adapters();
        if adapters.is_empty() {
            return super::probe_gpus_nvml();
        }

        let engine_vals = engine_usage_values();
        let mem_dedicated = pdh_wildcard_values(r"\GPU Adapter Memory(*)\Dedicated Usage", false);
        let mem_shared = pdh_wildcard_values(r"\GPU Adapter Memory(*)\Shared Usage", false);

        let mut gpus: Vec<GpuProbe> = adapters
            .into_iter()
            .map(|a| {
                let usage_pct = max_engine_usage_for_luid(&a.luid_key, &engine_vals);
                let used_bytes = memory_bytes_for_luid(&a.luid_key, &mem_dedicated, &mem_shared);

                let total_mb = if a.total_vram_bytes > 0 {
                    Some(a.total_vram_bytes / (1024 * 1024))
                } else {
                    None
                };
                let used_mb = used_bytes.map(|b| b / (1024 * 1024));

                GpuProbe {
                    name: a.name,
                    usage_pct,
                    vram_used_mb: used_mb,
                    vram_total_mb: total_mb,
                }
            })
            .collect();

        // DXGI can expose multiple nodes for one card; keep the richest entry per name.
        let mut best: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for (idx, gpu) in gpus.iter().enumerate() {
            let score = (gpu.usage_pct.is_some() as u8)
                + (gpu.vram_used_mb.is_some() as u8)
                + (gpu.vram_total_mb.is_some() as u8);
            best
                .entry(gpu.name.clone())
                .and_modify(|prev| {
                    let prev_score = (gpus[*prev].usage_pct.is_some() as u8)
                        + (gpus[*prev].vram_used_mb.is_some() as u8)
                        + (gpus[*prev].vram_total_mb.is_some() as u8);
                    if score > prev_score {
                        *prev = idx;
                    }
                })
                .or_insert(idx);
        }
        let mut keep: Vec<usize> = best.values().copied().collect();
        keep.sort_unstable();
        gpus = keep.into_iter().map(|i| gpus[i].clone()).collect();
        gpus
    }
}

#[cfg(windows)]
pub fn probe_gpus() -> Vec<GpuProbe> {
    windows_impl::probe_gpus()
}

#[cfg(not(windows))]
pub fn probe_gpus() -> Vec<GpuProbe> {
    probe_gpus_nvml()
}

pub(crate) fn probe_gpus_nvml() -> Vec<GpuProbe> {
    use nvml_wrapper::Nvml;

    let mut gpus = Vec::new();
    if let Ok(nvml) = Nvml::init() {
        if let Ok(n) = nvml.device_count() {
            for i in 0..n {
                if let Ok(dev) = nvml.device_by_index(i) {
                    let name = dev.name().unwrap_or_else(|_| "GPU".into());
                    let usage_pct = dev.utilization_rates().ok().map(|u| u.gpu as f32);
                    let (vram_used_mb, vram_total_mb) = if let Ok(mem) = dev.memory_info() {
                        (
                            Some(mem.used / (1024 * 1024)),
                            Some(mem.total / (1024 * 1024)),
                        )
                    } else {
                        (None, None)
                    };
                    gpus.push(GpuProbe {
                        name,
                        usage_pct,
                        vram_used_mb,
                        vram_total_mb,
                    });
                }
            }
        }
    }
    gpus
}

#[cfg(all(test, windows))]
mod tests {
    use super::probe_gpus;
    use crate::gpu_probe::windows_impl;

    #[test]
    fn windows_gpu_probe_returns_metrics() {
        let gpus = probe_gpus();
        eprintln!("GPU probe: {gpus:?}");
        assert!(!gpus.is_empty(), "expected at least one DXGI adapter");
        let with_usage = gpus.iter().filter(|g| g.usage_pct.is_some()).count();
        let with_vram_used = gpus.iter().filter(|g| g.vram_used_mb.is_some()).count();
        eprintln!("with usage: {with_usage}, with vram used: {with_vram_used}");
    }

    #[test]
    fn dump_pdh_gpu_wildcards() {
        for path in [
            r"\GPU Engine(*engtype_3D*)\Utilization Percentage",
            r"\GPU Engine(*)\Utilization Percentage",
        ] {
            let eng = windows_impl::pdh_wildcard_values(path, true);
            eprintln!("engine {path} ({})", eng.len());
            for (i, v) in eng.iter().take(5) {
                eprintln!("  {i} = {v}");
            }
        }
        let mem =
            windows_impl::pdh_wildcard_values(r"\GPU Adapter Memory(*)\Dedicated Usage", false);
        eprintln!("memory wildcard ({})", mem.len());
        for (i, v) in mem.iter().take(10) {
            eprintln!("  {i} = {v}");
        }
    }
}
