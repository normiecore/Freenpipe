// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Which bun binary runs Pi.
//!
//! The bun that ships with the app is bun's standard Windows x64 build, which
//! executes AVX2 instructions. On a processor without AVX2 (older Pentium and
//! Celeron parts, some VMs that mask the feature) it dies at once with
//! `STATUS_ILLEGAL_INSTRUCTION` (`0xC000001D`) and chat, pipes and the MCP
//! server all stop working. bun publishes a "baseline" build without AVX2;
//! on such machines the runtime downloads the matching release from bun's
//! GitHub releases, checks it against a pinned SHA-256, unpacks it under
//! `<data dir>/pi-agent/bun-baseline/` and uses it instead.
//!
//! The baseline build is chosen when the CPU lacks AVX2 (CPUID, see
//! [`crate::cpu_features`]), when a stock bun has already died with an illegal
//! instruction on this machine (a marker file remembers that), or when
//! `SCREENPIPE_FORCE_BASELINE_BUN=1` asks for it. The choice is the pure
//! function [`choose_bun_build`]; everything else is plumbing around it.
//! Other platforms always use the stock bun.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::env;

/// bun release the baseline build is taken from (the release the app bundles).
pub const BASELINE_BUN_VERSION: &str = "1.3.10";
/// Release asset with the baseline build for Windows x64.
pub const BASELINE_BUN_ASSET: &str = "bun-windows-x64-baseline.zip";
/// Where the asset is downloaded from (bun's own GitHub releases).
pub const BASELINE_BUN_URL: &str =
    "https://github.com/oven-sh/bun/releases/download/bun-v1.3.10/bun-windows-x64-baseline.zip";
/// SHA-256 of [`BASELINE_BUN_ASSET`] as listed in the release's
/// `SHASUMS256.txt`.
pub const BASELINE_BUN_SHA256: &str =
    "715709c69b176e20994533d3292bd0b7c32de9c0c5575b916746ec6b2aa38346";
/// Directory under the Pi install directory holding baseline builds.
pub const BASELINE_DIR_NAME: &str = "bun-baseline";
/// Marker under the Pi install directory: a stock bun died with an illegal
/// instruction on this machine.
pub const STOCK_BUN_CRASH_MARKER: &str = ".stock-bun-illegal-instruction";
/// Windows NTSTATUS for an illegal instruction.
pub const STATUS_ILLEGAL_INSTRUCTION: u32 = 0xC000_001D;

/// Largest download accepted for the asset (the 1.3.10 zip is about 41 MB).
const MAX_ASSET_BYTES: u64 = 256 * 1024 * 1024;

/// The two bun builds the runtime can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BunBuild {
    /// The bun shipped with the app (or found on the system).
    Stock,
    /// bun's baseline Windows x64 build, for CPUs without AVX2.
    Baseline,
}

/// Facts [`choose_bun_build`] decides on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BunFacts {
    /// Running on Windows x64, the only target with a baseline download.
    pub windows_x64: bool,
    /// AVX2 support as read by CPUID; `None` when it cannot be known.
    pub cpu_has_avx2: Option<bool>,
    /// `SCREENPIPE_FORCE_BASELINE_BUN` asked for the baseline build.
    pub forced: bool,
    /// A stock bun already died with an illegal instruction here.
    pub stock_crashed: bool,
}

/// Pick the bun build and say why.
pub fn choose_bun_build(facts: BunFacts) -> (BunBuild, &'static str) {
    if !facts.windows_x64 {
        return (
            BunBuild::Stock,
            "baseline builds are only used on Windows x64",
        );
    }
    if facts.forced {
        return (BunBuild::Baseline, "SCREENPIPE_FORCE_BASELINE_BUN is set");
    }
    if facts.cpu_has_avx2 == Some(false) {
        return (BunBuild::Baseline, "the CPU does not support AVX2");
    }
    if facts.stock_crashed {
        return (
            BunBuild::Baseline,
            "the stock bun crashed with an illegal instruction earlier",
        );
    }
    (BunBuild::Stock, "the CPU supports AVX2")
}

/// Whether a process exit code is Windows' illegal-instruction status.
/// Windows reports NTSTATUS exit codes as `i32`, so `0xC000001D` arrives as a
/// negative number.
pub fn is_illegal_instruction_exit(code: Option<i32>) -> bool {
    code.map(|c| c as u32 == STATUS_ILLEGAL_INSTRUCTION)
        .unwrap_or(false)
}

/// Whether an environment value turns a flag on (`1`, `true`, `yes`, `on`).
pub fn env_flag_on(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

/// `<install>/bun-baseline/bun-v<version>`.
pub fn baseline_root(install_dir: &Path) -> PathBuf {
    install_dir
        .join(BASELINE_DIR_NAME)
        .join(format!("bun-v{BASELINE_BUN_VERSION}"))
}

/// The baseline `bun.exe` once unpacked.
pub fn baseline_bun_path(install_dir: &Path) -> PathBuf {
    baseline_root(install_dir)
        .join("bun-windows-x64-baseline")
        .join("bun.exe")
}

/// Remember that a stock bun died with an illegal instruction.
pub fn record_stock_bun_crash(install_dir: &Path) {
    let marker = install_dir.join(STOCK_BUN_CRASH_MARKER);
    if marker.exists() {
        return;
    }
    let _ = std::fs::create_dir_all(install_dir);
    match std::fs::write(&marker, chrono::Utc::now().to_rfc3339()) {
        Ok(()) => warn!(
            "bun: the stock build died with an illegal instruction; the baseline build will be used"
        ),
        Err(e) => warn!("bun: could not record the illegal-instruction crash: {}", e),
    }
}

/// Facts for this machine and process.
pub fn current_facts(install_dir: &Path) -> BunFacts {
    BunFacts {
        windows_x64: cfg!(all(windows, target_arch = "x86_64")),
        cpu_has_avx2: crate::cpu_features::cpu_has_avx2(),
        forced: env_flag_on(env::non_empty(env::SCREENPIPE_FORCE_BASELINE_BUN).as_deref()),
        stock_crashed: install_dir.join(STOCK_BUN_CRASH_MARKER).exists(),
    }
}

/// Whether this machine should run the baseline build.
pub fn wants_baseline(install_dir: &Path) -> bool {
    choose_bun_build(current_facts(install_dir)).0 == BunBuild::Baseline
}

static LOGGED_CHOICE: AtomicBool = AtomicBool::new(false);
static LOGGED_MISSING: AtomicBool = AtomicBool::new(false);

/// Given the stock bun the caller found, return the bun to run: the baseline
/// build when this machine needs it and it is installed, else `stock`.
/// Logs the decision once per process.
pub fn select_bun(install_dir: &Path, stock: Option<String>) -> Option<String> {
    let (build, reason) = choose_bun_build(current_facts(install_dir));
    if build == BunBuild::Stock {
        return stock;
    }
    let baseline = baseline_bun_path(install_dir);
    if baseline.is_file() {
        if !LOGGED_CHOICE.swap(true, Ordering::Relaxed) {
            info!(
                "bun: using the baseline (non-AVX2) build at {} because {}",
                baseline.display(),
                reason
            );
        }
        return Some(baseline.to_string_lossy().into_owned());
    }
    if !LOGGED_MISSING.swap(true, Ordering::Relaxed) {
        warn!(
            "bun: the baseline build is needed ({}) but not installed yet; using {:?}",
            reason, stock
        );
    }
    stock
}

static BASELINE_FETCH_STARTED: AtomicBool = AtomicBool::new(false);

/// React to a stock bun that died with an illegal instruction: remember it
/// and, on Windows x64, fetch the baseline build in the background (once per
/// process) so the next start of Pi uses it.
pub fn handle_stock_bun_crash(install_dir: &Path) {
    record_stock_bun_crash(install_dir);
    if !cfg!(all(windows, target_arch = "x86_64")) || baseline_bun_path(install_dir).is_file() {
        return;
    }
    if BASELINE_FETCH_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let dir = install_dir.to_path_buf();
    let spawned = std::thread::Builder::new()
        .name("bun-baseline".to_string())
        .spawn(move || {
            if let Err(e) = ensure_baseline_bun(&dir) {
                warn!("bun: could not set up the baseline build: {:#}", e);
                BASELINE_FETCH_STARTED.store(false, Ordering::SeqCst);
            }
        });
    if spawned.is_err() {
        BASELINE_FETCH_STARTED.store(false, Ordering::SeqCst);
    }
}

/// Lower-case hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Make sure the baseline build is unpacked, downloading it if needed.
/// Blocking: call it from a background thread. Returns the `bun.exe` path.
pub fn ensure_baseline_bun(install_dir: &Path) -> Result<PathBuf> {
    // One download at a time in this process; the staging directory is named
    // after the process id.
    static BASELINE_LOCK: Mutex<()> = Mutex::new(());

    let exe = baseline_bun_path(install_dir);
    if exe.is_file() {
        return Ok(exe);
    }
    if !cfg!(all(windows, target_arch = "x86_64")) {
        return Err(anyhow!(
            "the baseline bun build is only used on Windows x64"
        ));
    }
    let _guard = BASELINE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    if exe.is_file() {
        return Ok(exe);
    }
    let root = baseline_root(install_dir);
    std::fs::create_dir_all(install_dir.join(BASELINE_DIR_NAME))?;
    info!(
        "bun: downloading the baseline build {} from {}",
        BASELINE_BUN_VERSION, BASELINE_BUN_URL
    );
    let bytes = download_asset(BASELINE_BUN_URL)?;
    let actual = sha256_hex(&bytes);
    if actual != BASELINE_BUN_SHA256 {
        return Err(anyhow!(
            "baseline bun download failed its checksum (expected {}, got {})",
            BASELINE_BUN_SHA256,
            actual
        ));
    }
    let staging = install_dir
        .join(BASELINE_DIR_NAME)
        .join(format!(".unpack-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    let zip = staging.join(BASELINE_BUN_ASSET);
    std::fs::write(&zip, &bytes)?;
    let unpacked = unpack_zip(&zip, &staging);
    let _ = std::fs::remove_file(&zip);
    if let Err(e) = unpacked {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    if !staging
        .join("bun-windows-x64-baseline")
        .join("bun.exe")
        .is_file()
    {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(anyhow!("the baseline bun archive did not contain bun.exe"));
    }
    let _ = std::fs::remove_dir_all(&root);
    std::fs::rename(&staging, &root)
        .with_context(|| format!("move baseline bun into {}", root.display()))?;
    info!("bun: baseline build ready at {}", exe.display());
    Ok(exe)
}

/// Download `url` on a thread of its own (the blocking HTTP client must not
/// run inside an async runtime).
fn download_asset(url: &'static str) -> Result<Vec<u8>> {
    std::thread::spawn(move || -> Result<Vec<u8>> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()?;
        let response = client.get(url).send()?.error_for_status()?;
        if let Some(len) = response.content_length() {
            if len > MAX_ASSET_BYTES {
                return Err(anyhow!(
                    "baseline bun asset is unexpectedly large ({len} bytes)"
                ));
            }
        }
        let bytes = response.bytes()?;
        if bytes.len() as u64 > MAX_ASSET_BYTES {
            return Err(anyhow!("baseline bun asset is unexpectedly large"));
        }
        Ok(bytes.to_vec())
    })
    .join()
    .map_err(|_| anyhow!("baseline bun download thread panicked"))?
}

/// Unpack a zip with the tools Windows ships (`tar.exe`, then PowerShell's
/// `Expand-Archive`).
#[cfg(windows)]
fn unpack_zip(zip: &Path, dest: &Path) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let system_root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let tar = PathBuf::from(&system_root).join("System32").join("tar.exe");
    let tar_ok = std::process::Command::new(if tar.is_file() {
        tar.as_os_str().to_owned()
    } else {
        "tar.exe".into()
    })
    .arg("-xf")
    .arg(zip)
    .arg("-C")
    .arg(dest)
    .creation_flags(CREATE_NO_WINDOW)
    .output()
    .map(|o| o.status.success())
    .unwrap_or(false);
    if tar_ok {
        return Ok(());
    }
    let script = format!(
        "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
        zip.display().to_string().replace('\'', "''"),
        dest.display().to_string().replace('\'', "''")
    );
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .context("run powershell Expand-Archive")?;
    if output.status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "could not unpack the baseline bun archive: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[cfg(not(windows))]
fn unpack_zip(_zip: &Path, _dest: &Path) -> Result<()> {
    Err(anyhow!(
        "the baseline bun build is only used on Windows x64"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows(avx2: Option<bool>) -> BunFacts {
        BunFacts {
            windows_x64: true,
            cpu_has_avx2: avx2,
            forced: false,
            stock_crashed: false,
        }
    }

    #[test]
    fn a_cpu_with_avx2_keeps_the_stock_bun() {
        assert_eq!(choose_bun_build(windows(Some(true))).0, BunBuild::Stock);
    }

    #[test]
    fn a_cpu_without_avx2_gets_the_baseline_bun() {
        let (build, reason) = choose_bun_build(windows(Some(false)));
        assert_eq!(build, BunBuild::Baseline);
        assert!(reason.contains("AVX2"));
    }

    #[test]
    fn forcing_selects_the_baseline_even_with_avx2() {
        let facts = BunFacts {
            forced: true,
            ..windows(Some(true))
        };
        assert_eq!(choose_bun_build(facts).0, BunBuild::Baseline);
    }

    #[test]
    fn an_earlier_illegal_instruction_crash_selects_the_baseline() {
        let facts = BunFacts {
            stock_crashed: true,
            ..windows(Some(true))
        };
        assert_eq!(choose_bun_build(facts).0, BunBuild::Baseline);
        // Unknown CPU support alone does not switch builds.
        assert_eq!(choose_bun_build(windows(None)).0, BunBuild::Stock);
    }

    #[test]
    fn other_platforms_always_use_the_stock_bun() {
        let facts = BunFacts {
            windows_x64: false,
            cpu_has_avx2: Some(false),
            forced: true,
            stock_crashed: true,
        };
        assert_eq!(choose_bun_build(facts).0, BunBuild::Stock);
    }

    #[test]
    fn the_illegal_instruction_status_is_recognised() {
        assert!(is_illegal_instruction_exit(Some(0xC000_001Du32 as i32)));
        assert!(is_illegal_instruction_exit(Some(-1073741795)));
        assert!(!is_illegal_instruction_exit(Some(1)));
        assert!(!is_illegal_instruction_exit(Some(0xC000_0005u32 as i32)));
        assert!(!is_illegal_instruction_exit(None));
    }

    #[test]
    fn env_flags_accept_the_usual_spellings() {
        for on in ["1", "true", "YES", " on "] {
            assert!(env_flag_on(Some(on)), "{on}");
        }
        for off in ["0", "false", "", "no"] {
            assert!(!env_flag_on(Some(off)), "{off}");
        }
        assert!(!env_flag_on(None));
    }

    #[test]
    fn the_baseline_lives_under_the_install_dir() {
        let p = baseline_bun_path(Path::new("/d/pi-agent"));
        assert!(p.starts_with("/d/pi-agent/bun-baseline/bun-v1.3.10"));
        assert!(p.ends_with("bun-windows-x64-baseline/bun.exe"));
    }

    #[test]
    fn the_crash_marker_switches_the_facts() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!current_facts(dir.path()).stock_crashed);
        record_stock_bun_crash(dir.path());
        assert!(current_facts(dir.path()).stock_crashed);
    }

    #[test]
    fn selection_falls_back_to_stock_when_the_baseline_is_absent() {
        // On the Linux test machine the stock bun is always kept; on Windows
        // without AVX2 the absent baseline still leaves the stock path.
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            select_bun(dir.path(), Some("bun".to_string())),
            Some("bun".to_string())
        );
    }

    #[test]
    fn sha256_matches_a_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(BASELINE_BUN_SHA256.len(), 64);
        assert!(BASELINE_BUN_URL.ends_with(BASELINE_BUN_ASSET));
        assert!(BASELINE_BUN_URL.contains(&format!("bun-v{BASELINE_BUN_VERSION}/")));
    }

    #[cfg(not(windows))]
    #[test]
    fn no_download_is_attempted_off_windows() {
        let dir = tempfile::tempdir().unwrap();
        assert!(ensure_baseline_bun(dir.path()).is_err());
        // A crash report only leaves the marker behind.
        handle_stock_bun_crash(dir.path());
        assert!(dir.path().join(STOCK_BUN_CRASH_MARKER).exists());
        assert!(!dir.path().join(BASELINE_DIR_NAME).exists());
    }
}
