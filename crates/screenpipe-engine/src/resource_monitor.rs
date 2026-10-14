use chrono::Local;
use serde_json::json;
use std::env;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};
use sysinfo::{CpuExt, PidExt, ProcessExt, System, SystemExt};
use tracing::debug;
use tracing::{error, info};

/// Samples the engine's own CPU and memory use. Everything stays local: a
/// DEBUG log line per sample, plus the optional `SAVE_RESOURCE_USAGE` JSONL
/// file. Nothing is sent over the network.
pub struct ResourceMonitor {
    start_time: Instant,
    resource_log_file: Option<String>, // analyse output here: https://colab.research.google.com/drive/1zELlGdzGdjChWKikSqZTHekm5XRxY-1r?usp=sharing
}

/// Static hardware info collected once at startup.
/// Only contains general model names — no serial numbers, UUIDs, or PII.
#[derive(Clone, Debug)]
struct HardwareInfo {
    cpu_brand: String,
    cpu_arch: String,
    gpu_names: Vec<String>,
}

impl HardwareInfo {
    fn collect() -> Self {
        let mut sys = System::new();
        sys.refresh_cpu();

        let cpu_brand = sys
            .cpus()
            .first()
            .map(|c| c.brand().trim().to_string())
            .unwrap_or_default();

        let cpu_arch = std::env::consts::ARCH.to_string();

        let gpu_names = detect_gpus();

        Self {
            cpu_brand,
            cpu_arch,
            gpu_names,
        }
    }
}

/// Best-effort GPU detection using platform tools.
/// Returns a list of GPU model names (e.g. ["Apple M2 Pro", "AMD Radeon RX 7900"]).
/// Never panics — returns an empty vec on any failure.
/// Capped at 8 entries to keep the startup log line short.
fn detect_gpus() -> Vec<String> {
    let gpus = detect_gpus_platform();
    // Cap to 8 GPUs (more than enough) and truncate long names
    gpus.into_iter()
        .take(8)
        .map(|s| {
            if s.len() > 200 {
                s[..200].to_string()
            } else {
                s
            }
        })
        .collect()
}

fn detect_gpus_platform() -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        detect_gpus_macos()
    }
    #[cfg(target_os = "linux")]
    {
        detect_gpus_linux()
    }
    #[cfg(target_os = "windows")]
    {
        detect_gpus_windows()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Vec::new()
    }
}

/// Run a command with a timeout to avoid blocking startup if a tool hangs.
fn run_cmd_with_timeout(cmd: &str, args: &[&str], timeout_secs: u64) -> Option<String> {
    use std::process::{Command, Stdio};
    let mut command = Command::new(cmd);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    // On Windows, prevent a console window from flashing on screen
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let mut child = command.spawn().ok()?;

    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let mut out = String::new();
                if let Some(mut stdout) = child.stdout.take() {
                    let _ = std::io::Read::read_to_string(&mut stdout, &mut out);
                }
                return Some(out);
            }
            Ok(Some(_)) => return None, // exited with error
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }
}

#[cfg(target_os = "macos")]
fn detect_gpus_macos() -> Vec<String> {
    let output = match run_cmd_with_timeout(
        "system_profiler",
        &["SPDisplaysDataType", "-detailLevel", "mini"],
        5,
    ) {
        Some(s) => s,
        None => return Vec::new(),
    };

    output
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with("Chipset Model:") || trimmed.starts_with("Chip:") {
                Some(trimmed.split(':').nth(1)?.trim().to_string())
            } else {
                None
            }
        })
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(target_os = "linux")]
fn detect_gpus_linux() -> Vec<String> {
    let mut gpus = Vec::new();

    // Try lspci (most common)
    if let Some(stdout) = run_cmd_with_timeout("lspci", &[], 5) {
        for line in stdout.lines() {
            // Match VGA, 3D, and Display controllers
            if line.contains("VGA") || line.contains("3D") || line.contains("Display") {
                // Format: "01:00.0 VGA compatible controller: NVIDIA Corporation GA106 [GeForce RTX 3060] (rev a1)"
                if let Some(desc) = line.split(": ").nth(1) {
                    // Strip PCI revision suffix like "(rev a1)"
                    let name = desc
                        .rfind(" (rev")
                        .map(|i| &desc[..i])
                        .unwrap_or(desc)
                        .trim()
                        .to_string();
                    if !name.is_empty() {
                        gpus.push(name);
                    }
                }
            }
        }
    }

    gpus
}

#[cfg(target_os = "windows")]
fn detect_gpus_windows() -> Vec<String> {
    // Try PowerShell first (wmic is deprecated on Windows 11+)
    if let Some(output) = run_cmd_with_timeout(
        "powershell",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-CimInstance Win32_VideoController | Select-Object -ExpandProperty Name",
        ],
        5,
    ) {
        let gpus: Vec<String> = output
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        if !gpus.is_empty() {
            return gpus;
        }
    }

    // Fallback to wmic (older Windows)
    if let Some(output) =
        run_cmd_with_timeout("wmic", &["path", "win32_VideoController", "get", "name"], 5)
    {
        return output
            .lines()
            .skip(1) // skip header "Name"
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
    }

    Vec::new()
}

pub enum RestartSignal {
    RecordingTasks,
}

impl ResourceMonitor {
    pub fn new() -> Arc<Self> {
        let resource_log_file = if env::var("SAVE_RESOURCE_USAGE").is_ok() {
            let now = Local::now();
            let filename = format!("resource_usage_{}.json", now.format("%Y%m%d_%H%M%S"));
            info!("Resource usage data will be saved to file: {}", filename);

            // Initialize the file with an empty JSON array
            if let Ok(mut file) = File::create(&filename) {
                if let Err(e) = file.write_all(b"[]") {
                    error!("Failed to initialize JSON file: {}", e);
                }
            } else {
                error!("Failed to create JSON file: {}", filename);
            }

            Some(filename)
        } else {
            None
        };

        // Collect hardware info once (CPU brand, GPU names) — never panics
        let hw_info = HardwareInfo::collect();
        debug!(
            "hardware: cpu={:?} arch={} gpus={:?}",
            hw_info.cpu_brand, hw_info.cpu_arch, hw_info.gpu_names
        );

        Arc::new(Self {
            start_time: Instant::now(),
            resource_log_file,
        })
    }

    async fn collect_metrics(&self, sys: &System) -> (f64, f64, f64, f32, f64, Duration) {
        let pid = std::process::id();
        let mut total_memory = 0.0;
        let mut max_virtual_memory = 0.0; // Changed from total to max
        let mut total_cpu = 0.0;

        if let Some(main_process) = sys.process(sysinfo::Pid::from_u32(pid)) {
            total_memory += main_process.memory() as f64 / (1024.0 * 1024.0 * 1024.0);

            // Take the maximum virtual memory instead of sum
            max_virtual_memory = main_process.virtual_memory() as f64 / (1024.0 * 1024.0 * 1024.0);

            total_cpu += main_process.cpu_usage();

            // Add child processes
            for child_process in sys.processes().values() {
                if child_process.parent() == Some(sysinfo::Pid::from_u32(pid)) {
                    total_memory += child_process.memory() as f64 / (1024.0 * 1024.0 * 1024.0);

                    // Take max instead of sum
                    max_virtual_memory = max_virtual_memory
                        .max(child_process.virtual_memory() as f64 / (1024.0 * 1024.0 * 1024.0));

                    total_cpu += child_process.cpu_usage();
                }
            }
        }

        let system_total_memory = sys.total_memory() as f64 / (1024.0 * 1024.0 * 1024.0);
        let memory_usage_percent = (total_memory / system_total_memory) * 100.0;
        let runtime = self.start_time.elapsed();

        (
            total_memory,
            system_total_memory,
            memory_usage_percent,
            total_cpu,
            max_virtual_memory,
            runtime,
        )
    }

    /// Max resource log file size (10 MB). When exceeded the file is truncated.
    const MAX_RESOURCE_LOG_BYTES: u64 = 10 * 1024 * 1024;

    async fn log_to_file(&self, metrics: (f64, f64, f64, f32, f64, Duration)) {
        let (
            total_memory_gb,
            system_total_memory,
            memory_usage_percent,
            total_cpu,
            total_virtual_memory_gb,
            runtime,
        ) = metrics;

        if let Some(ref filename) = self.resource_log_file {
            let json_data = json!({
                "timestamp": Local::now().to_rfc3339(),
                "runtime_seconds": runtime.as_secs(),
                "total_memory_gb": total_memory_gb,
                "system_total_memory_gb": system_total_memory,
                "memory_usage_percent": memory_usage_percent,
                "total_cpu_percent": total_cpu,
                "total_virtual_memory_gb": total_virtual_memory_gb,
            });

            // Append-only JSONL: one JSON object per line, no read-back needed.
            if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(filename) {
                // Truncate if file exceeds size limit to prevent unbounded growth.
                if let Ok(meta) = file.metadata() {
                    if meta.len() > Self::MAX_RESOURCE_LOG_BYTES {
                        if let Ok(f) = OpenOptions::new().write(true).truncate(true).open(filename)
                        {
                            drop(f);
                            // Reopen in append mode after truncation.
                            if let Ok(reopened) = OpenOptions::new().append(true).open(filename) {
                                file = reopened;
                            }
                        }
                    }
                }
                let mut line = json_data.to_string();
                line.push('\n');
                if let Err(e) = file.write_all(line.as_bytes()) {
                    error!("Failed to write resource log: {}", e);
                }
            }
        }
    }

    async fn log_status(&self, sys: &System) {
        let metrics = self.collect_metrics(sys).await;
        let (
            total_memory_gb,
            system_total_memory,
            memory_usage_percent,
            total_cpu,
            total_virtual_memory_gb,
            runtime,
        ) = metrics;

        // Log to console with virtual memory
        let log_message = format!(
            "Runtime: {}s, Memory: {:.0}% ({:.2} GB / {:.2} GB), Virtual: {:.2} GB, CPU: {:.0}%",
            runtime.as_secs(),
            memory_usage_percent,
            total_memory_gb,
            system_total_memory,
            total_virtual_memory_gb,
            total_cpu
        );
        debug!("{}", log_message);

        // Log to file
        self.log_to_file(metrics).await;
    }

    /// Sample every `interval` until the process exits.
    pub fn start_monitoring(self: &Arc<Self>, interval: Duration) {
        let monitor = Arc::clone(self);

        tokio::spawn(async move {
            // Only load process + CPU info — skip disks, networks, components.
            let mut sys = System::new();
            sys.refresh_cpu();
            sys.refresh_processes();
            sys.refresh_memory();

            loop {
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {
                        // Only refresh what collect_metrics actually uses:
                        // CPU + process list + system memory totals.
                        // Skips disks, networks, components — saves allocations.
                        sys.refresh_cpu();
                        sys.refresh_processes();
                        sys.refresh_memory();

                        // Tell the system allocator to return freed pages to the OS.
                        // Without this, the default macOS allocator holds freed large
                        // allocations as "empty" regions indefinitely, causing RSS to
                        // grow monotonically even though Rust is freeing correctly.
                        #[cfg(target_os = "macos")]
                        {
                            extern "C" {
                                fn malloc_zone_pressure_relief(
                                    zone: *mut std::ffi::c_void,
                                    goal: usize,
                                ) -> usize;
                            }
                            // zone=NULL means all zones, goal=0 means release as much as possible
                            let freed = unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
                            if freed > 0 {
                                debug!("malloc_zone_pressure_relief freed {} bytes", freed);
                            }
                        }
                        #[cfg(target_os = "linux")]
                        {
                            extern "C" {
                                fn malloc_trim(pad: usize) -> std::ffi::c_int;
                            }
                            unsafe { malloc_trim(0) };
                        }
                        monitor.log_status(&sys).await;
                    }
                }
            }
        });
    }

    pub async fn shutdown(&self) {
        if let Some(ref file) = self.resource_log_file {
            if let Ok(mut f) = OpenOptions::new().write(true).open(file) {
                let _ = f.flush();
            }
        }
    }
}
