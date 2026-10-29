// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Local crash dumps for crashes Rust's panic hook cannot see (shell-017).
//!
//! A stack buffer overrun, a `__fastfail` or an `abort()` in native code ends
//! the process without running any Rust code, so `last-panic.log` stays
//! empty. On Windows the app registers the helper DLL built from
//! `crates/screenpipe-wer-helper` (`screenpipe_wer_dump_helper.dll`, shipped
//! next to the executable) as a WER runtime exception module. When such a
//! crash happens, WerFault.exe loads the helper, which writes a minidump into
//! `<data dir>/crash-dumps/pending/`. At the next start the app moves the
//! finished dumps one level up into `<data dir>/crash-dumps/`, logs how many
//! it recovered, and keeps only the newest few.
//!
//! The dumps never leave the machine: nothing here or in the helper uploads
//! anything, and the user decides whether to attach one to a bug report.
//!
//! Registration happens early in `app_main` (before Tauri builds anything, so
//! start-up crashes are covered too), when no log subscriber exists yet; its
//! outcome is logged later by [`log_and_harvest`], which also starts the
//! harvest thread.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use tracing::{info, warn};

/// File name of the helper DLL beside the app executable.
#[cfg_attr(not(windows), allow(dead_code))] // the Windows registration uses it
pub const HELPER_DLL_NAME: &str = "screenpipe_wer_dump_helper.dll";
/// Name of the thread that collects dumps at start-up.
pub const HARVEST_THREAD_NAME: &str = "screenpipe-wer-dump-harvest";
/// Sub-directory of `crash-dumps` the helper writes into.
pub const PENDING_DIR: &str = "pending";
/// Finished dumps kept in `crash-dumps`; older ones are deleted at harvest.
pub const MAX_KEPT_DUMPS: usize = 10;
/// A `.partial` file older than this was left by a helper that was killed
/// while writing; younger ones may still be in progress.
pub const STALE_PARTIAL_AGE: Duration = Duration::from_secs(60 * 60);

const DUMP_EXTENSION: &str = "dmp";
const PARTIAL_SUFFIX: &str = ".partial";

// Context block shared with the helper. Same values as
// crates/screenpipe-wer-helper/src/layout.rs (a test keeps them in sync).
// Only the Windows registration builds one.
#[cfg_attr(not(windows), allow(dead_code))]
const CONTEXT_MAGIC: u32 = u32::from_le_bytes(*b"FPWR");
#[cfg_attr(not(windows), allow(dead_code))]
const CONTEXT_VERSION: u32 = 1;
#[cfg_attr(not(windows), allow(dead_code))]
const CONTEXT_DIR_UNITS: usize = 1024;

/// The block the helper reads out of this process after a crash. It must
/// live as long as the process, so it is leaked once at registration.
#[repr(C)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct ContextBlock {
    magic: u32,
    version: u32,
    dir_len: u32,
    reserved: u32,
    dir: [u16; CONTEXT_DIR_UNITS],
}

#[cfg_attr(not(windows), allow(dead_code))]
impl ContextBlock {
    /// Block naming `dir` (UTF-16 units, no terminator). `None` when the
    /// path is empty or too long for the block.
    pub fn new(dir: &[u16]) -> Option<Self> {
        if dir.is_empty() || dir.len() >= CONTEXT_DIR_UNITS || dir.contains(&0) {
            return None;
        }
        let mut block = ContextBlock {
            magic: CONTEXT_MAGIC,
            version: CONTEXT_VERSION,
            dir_len: dir.len() as u32,
            reserved: 0,
            dir: [0; CONTEXT_DIR_UNITS],
        };
        block.dir[..dir.len()].copy_from_slice(dir);
        Some(block)
    }
}

/// `<data dir>/crash-dumps`.
pub fn crash_dumps_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(screenpipe_core::paths::CRASH_DUMPS_DIR)
}

/// `<data dir>/crash-dumps/pending`, where the helper writes.
pub fn pending_dir(data_dir: &Path) -> PathBuf {
    crash_dumps_dir(data_dir).join(PENDING_DIR)
}

/// Directories the harvest collects from: the pending directory, and the
/// helper's fallback `crash-dumps` next to the executable (used when it
/// could not read the context block).
pub fn harvest_sources(data_dir: &Path, exe_dir: Option<&Path>) -> Vec<PathBuf> {
    let dest = crash_dumps_dir(data_dir);
    let mut sources = vec![pending_dir(data_dir)];
    if let Some(dir) = exe_dir {
        let fallback = dir.join(screenpipe_core::paths::CRASH_DUMPS_DIR);
        if fallback != dest && !sources.contains(&fallback) {
            sources.push(fallback);
        }
    }
    sources
}

fn is_finished_dump(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(stem, ext)| !stem.is_empty() && ext.eq_ignore_ascii_case(DUMP_EXTENSION))
}

/// What one harvest did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HarvestReport {
    /// Dumps moved into `crash-dumps`.
    pub moved: usize,
    /// Dumps that could not be moved (left where they were).
    pub failed: usize,
    /// Old dumps deleted to stay within [`MAX_KEPT_DUMPS`].
    pub pruned: usize,
    /// Abandoned `.partial` files deleted.
    pub partials_removed: usize,
}

/// A name in `dest` that is not taken yet: `name`, else `stem-1.dmp`, ...
fn free_name(dest: &Path, name: &str) -> PathBuf {
    let first = dest.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, DUMP_EXTENSION));
    (1..)
        .map(|n| dest.join(format!("{stem}-{n}.{ext}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

/// Move a file, copying when source and destination are on different
/// volumes.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)?;
    if let Err(e) = std::fs::remove_file(from) {
        // Keep exactly one copy: undo the copy when the source stays.
        let _ = std::fs::remove_file(to);
        return Err(e);
    }
    Ok(())
}

/// Move finished dumps from `sources` into `dest`, delete abandoned partial
/// files, then keep only the `keep` newest dumps in `dest`.
pub fn harvest(sources: &[PathBuf], dest: &Path, keep: usize, now: SystemTime) -> HarvestReport {
    let mut report = HarvestReport::default();
    if std::fs::create_dir_all(dest).is_err() {
        return report;
    }
    for source in sources {
        let Ok(entries) = std::fs::read_dir(source) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_finished_dump(&name) {
                match move_file(&path, &free_name(dest, &name)) {
                    Ok(()) => report.moved += 1,
                    Err(_) => report.failed += 1,
                }
            } else if name.ends_with(PARTIAL_SUFFIX) {
                let abandoned = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| now.duration_since(t).ok())
                    .is_some_and(|age| age >= STALE_PARTIAL_AGE);
                if abandoned && std::fs::remove_file(&path).is_ok() {
                    report.partials_removed += 1;
                }
            }
        }
    }
    report.pruned = prune(dest, keep);
    report
}

/// Delete the oldest finished dumps in `dir` beyond `keep`. Returns how many
/// were deleted.
fn prune(dir: &Path, keep: usize) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut dumps: Vec<(SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|e| is_finished_dump(&e.file_name().to_string_lossy()))
        .filter(|e| e.path().is_file())
        .map(|e| {
            let modified = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (modified, e.path())
        })
        .collect();
    if dumps.len() <= keep {
        return 0;
    }
    // Newest first; names break ties so the order is stable.
    dumps.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    dumps
        .into_iter()
        .skip(keep)
        .filter(|(_, path)| std::fs::remove_file(path).is_ok())
        .count()
}

/// How registering the helper went.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))] // only Windows registers
pub enum Registration {
    /// WER will call the helper for fail-fast crashes.
    Enabled { dll: PathBuf },
    /// The helper is not installed beside the executable (normal for dev
    /// builds, which do not stage it).
    HelperMissing { expected: PathBuf },
    /// Registration failed; crashes still get WER's own handling.
    Failed { dll: PathBuf, reason: String },
    /// Not a Windows build.
    Unsupported,
}

static REGISTRATION: OnceLock<Registration> = OnceLock::new();

/// Register the helper with WER (Windows; a no-op elsewhere). Call once,
/// early in `app_main`. Safe to call before logging is set up: the outcome
/// is kept for [`log_and_harvest`].
pub fn register_early(data_dir: &Path) {
    REGISTRATION.get_or_init(|| platform::register(data_dir));
}

/// Log the registration outcome and collect dumps from earlier crashes on a
/// background thread (Windows only: no other platform has a helper that
/// writes dumps). Call once logging works.
pub fn log_and_harvest(data_dir: &Path) {
    match REGISTRATION.get() {
        Some(Registration::Enabled { dll }) => info!(
            "fail-fast crash dumps enabled: WER helper {} registered",
            dll.display()
        ),
        Some(Registration::HelperMissing { expected }) => info!(
            "fail-fast crash dumps unavailable: {} is not installed beside the app (expected in dev builds)",
            expected.display()
        ),
        Some(Registration::Failed { dll, reason }) => warn!(
            "fail-fast crash dumps unavailable: registering {} failed: {reason}",
            dll.display()
        ),
        Some(Registration::Unsupported) | None => {}
    }
    if !cfg!(windows) {
        return;
    }

    let data_dir = data_dir.to_path_buf();
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let spawned = std::thread::Builder::new()
        .name(HARVEST_THREAD_NAME.to_string())
        .spawn(move || {
            let dest = crash_dumps_dir(&data_dir);
            let sources = harvest_sources(&data_dir, exe_dir.as_deref());
            let report = harvest(&sources, &dest, MAX_KEPT_DUMPS, SystemTime::now());
            if report.moved > 0 {
                info!(
                    "recovered {} crash minidump(s) into {} (kept locally, never uploaded)",
                    report.moved,
                    dest.display()
                );
            }
            if report.failed > 0 {
                warn!(
                    "{} crash minidump(s) could not be moved into {}",
                    report.failed,
                    dest.display()
                );
            }
            if report.pruned > 0 || report.partials_removed > 0 {
                info!(
                    "crash dump cleanup: {} old dump(s) and {} abandoned partial file(s) removed",
                    report.pruned, report.partials_removed
                );
            }
        });
    if let Err(e) = spawned {
        warn!("could not start the crash dump harvest: {e}");
    }
}

/// Debug builds and developer mode only: end the process with a fail-fast
/// abort, the kind of crash the WER helper exists for, to check that a dump
/// appears in `crash-dumps` at the next start.
#[tauri::command]
#[specta::specta]
pub async fn debug_force_native_crash(app: tauri::AppHandle) -> Result<(), String> {
    if !crate::relaunch::debug_commands_allowed(&app) {
        return Err("debug commands are off; turn on developer mode first".to_string());
    }
    warn!("debug_force_native_crash: aborting the process to test fail-fast crash dumps");
    // On Windows this is a __fastfail: no unwinding, no panic hook, straight
    // to Windows Error Reporting.
    std::process::abort();
}

#[cfg(windows)]
mod platform {
    use super::{pending_dir, ContextBlock, Registration, HELPER_DLL_NAME};
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    /// Per-user list of runtime exception modules WER may load.
    const HELPER_MODULES_KEY: &str =
        r"Software\Microsoft\Windows\Windows Error Reporting\RuntimeExceptionHelperModules";

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().collect()
    }

    /// Remove values for a helper with our file name whose DLL no longer
    /// exists (an old install location). Other entries are left alone.
    fn prune_stale_values(key: &winreg::RegKey, current: &Path) {
        let stale: Vec<String> = key
            .enum_values()
            .filter_map(Result::ok)
            .map(|(name, _)| name)
            .filter(|name| {
                let path = PathBuf::from(name);
                let ours = path
                    .file_name()
                    .is_some_and(|f| f.eq_ignore_ascii_case(HELPER_DLL_NAME));
                ours && path != current && !path.exists()
            })
            .collect();
        for name in stale {
            let _ = key.delete_value(&name);
        }
    }

    fn allow_list(dll: &Path) -> Result<(), String> {
        let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
        let (key, _) = hkcu
            .create_subkey(HELPER_MODULES_KEY)
            .map_err(|e| format!("cannot open the helper-module registry key: {e}"))?;
        prune_stale_values(&key, dll);
        key.set_value(dll.as_os_str(), &0u32)
            .map_err(|e| format!("cannot write the helper-module registry value: {e}"))
    }

    pub(super) fn register(data_dir: &Path) -> Registration {
        let Some(exe_dir) = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
        else {
            return Registration::Failed {
                dll: PathBuf::from(HELPER_DLL_NAME),
                reason: "cannot resolve the executable directory".to_string(),
            };
        };
        let dll = exe_dir.join(HELPER_DLL_NAME);
        if !dll.is_file() {
            return Registration::HelperMissing { expected: dll };
        }
        if let Err(reason) = allow_list(&dll) {
            return Registration::Failed { dll, reason };
        }
        // The directory the context block names. The helper creates it as
        // well, but having it in place before any crash is cheaper.
        let _ = std::fs::create_dir_all(pending_dir(data_dir));
        // The block lives for the rest of the process: the helper reads it
        // out of our memory after a crash. A path too long for it leaves the
        // context empty, and the helper writes next to itself instead.
        let context: *const std::ffi::c_void =
            match ContextBlock::new(&wide(&pending_dir(data_dir))) {
                Some(block) => {
                    let leaked: &'static ContextBlock = Box::leak(Box::new(block));
                    (leaked as *const ContextBlock).cast()
                }
                None => std::ptr::null(),
            };
        let mut dll_wide = wide(&dll);
        dll_wide.push(0);
        // SAFETY: `dll_wide` is NUL-terminated and outlives the call; the
        // context pointer is null or a leaked block that is never freed.
        let result = unsafe {
            windows::Win32::System::ErrorReporting::WerRegisterRuntimeExceptionModule(
                windows::core::PCWSTR(dll_wide.as_ptr()),
                context,
            )
        };
        match result {
            Ok(()) => Registration::Enabled { dll },
            Err(e) => Registration::Failed {
                dll,
                reason: format!("WerRegisterRuntimeExceptionModule failed: {e}"),
            },
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::Registration;
    use std::path::Path;

    pub(super) fn register(_data_dir: &Path) -> Registration {
        Registration::Unsupported
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn set_age(path: &Path, age: Duration) {
        let when = SystemTime::now() - age;
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_modified(when).unwrap();
    }

    fn sorted_files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.path().is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn layout_matches_the_helper_crate() {
        let helper = include_str!("../../../../crates/screenpipe-wer-helper/src/layout.rs");
        assert!(helper.contains(r#"pub const CONTEXT_MAGIC: u32 = u32::from_le_bytes(*b"FPWR");"#));
        assert!(helper.contains(&format!(
            "pub const CONTEXT_VERSION: u32 = {CONTEXT_VERSION};"
        )));
        assert!(helper.contains(&format!(
            "pub const CONTEXT_DIR_UNITS: usize = {CONTEXT_DIR_UNITS};"
        )));
        assert!(helper.contains(&format!(
            r#"pub const PENDING_DIR: &str = "{PENDING_DIR}";"#
        )));
        assert!(helper.contains(&format!(
            r#"pub const PARTIAL_SUFFIX: &str = "{PARTIAL_SUFFIX}";"#
        )));
        assert!(helper.contains(&format!(
            r#"pub const CRASH_DUMPS_DIR: &str = "{}";"#,
            screenpipe_core::paths::CRASH_DUMPS_DIR
        )));
        // CONTEXT_BLOCK_BYTES in the helper: a 16-byte header plus the path.
        assert_eq!(
            std::mem::size_of::<ContextBlock>(),
            16 + CONTEXT_DIR_UNITS * 2
        );
        let cargo = include_str!("../../../../crates/screenpipe-wer-helper/Cargo.toml");
        assert!(cargo.contains(&format!(
            r#"name = "{}""#,
            HELPER_DLL_NAME.trim_end_matches(".dll")
        )));
    }

    #[test]
    fn context_block_carries_the_pending_directory() {
        let dir: Vec<u16> = r"C:\Users\Ada\.screenpipe\crash-dumps\pending"
            .encode_utf16()
            .collect();
        let block = ContextBlock::new(&dir).unwrap();
        assert_eq!(block.magic, CONTEXT_MAGIC);
        assert_eq!(block.version, CONTEXT_VERSION);
        assert_eq!(block.dir_len as usize, dir.len());
        assert_eq!(&block.dir[..dir.len()], dir.as_slice());
        assert!(block.dir[dir.len()..].iter().all(|u| *u == 0));
        assert!(ContextBlock::new(&vec![b'x' as u16; CONTEXT_DIR_UNITS]).is_none());
        assert!(ContextBlock::new(&[]).is_none());
    }

    #[test]
    fn dumps_land_in_the_data_dir_crash_dumps_folder() {
        let data = Path::new("/data");
        assert_eq!(crash_dumps_dir(data), Path::new("/data/crash-dumps"));
        assert_eq!(pending_dir(data), Path::new("/data/crash-dumps/pending"));
        assert_eq!(
            harvest_sources(data, Some(Path::new("/opt/handvel"))),
            vec![
                PathBuf::from("/data/crash-dumps/pending"),
                PathBuf::from("/opt/handvel/crash-dumps")
            ]
        );
        // An executable in the data dir must not harvest into itself.
        assert_eq!(
            harvest_sources(data, Some(data)),
            vec![PathBuf::from("/data/crash-dumps/pending")]
        );
    }

    #[test]
    fn harvest_moves_finished_dumps_and_leaves_ones_in_progress() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let exe_dir = tmp.path().join("install");
        let pending = pending_dir(&data);
        touch(&pending.join("Handvel-1-20261020T101500Z.dmp"), b"one");
        touch(
            &exe_dir.join("crash-dumps/Handvel-2-20261020T101600Z.dmp"),
            b"two",
        );
        touch(
            &pending.join("Handvel-3-20261020T101700Z.dmp.partial"),
            b"busy",
        );
        let dead = pending.join("Handvel-4-20261019T101700Z.dmp.partial");
        touch(&dead, b"dead");
        set_age(&dead, STALE_PARTIAL_AGE + Duration::from_secs(60));
        touch(&pending.join("notes.txt"), b"keep me");

        let dest = crash_dumps_dir(&data);
        let sources = harvest_sources(&data, Some(&exe_dir));
        let report = harvest(&sources, &dest, MAX_KEPT_DUMPS, SystemTime::now());
        assert_eq!(
            report,
            HarvestReport {
                moved: 2,
                failed: 0,
                pruned: 0,
                partials_removed: 1
            }
        );
        assert_eq!(
            sorted_files(&dest),
            vec![
                "Handvel-1-20261020T101500Z.dmp",
                "Handvel-2-20261020T101600Z.dmp"
            ]
        );
        assert_eq!(
            fs::read(dest.join("Handvel-2-20261020T101600Z.dmp")).unwrap(),
            b"two"
        );
        assert_eq!(
            sorted_files(&pending),
            vec!["Handvel-3-20261020T101700Z.dmp.partial", "notes.txt"]
        );

        // A second run finds nothing new.
        let again = harvest(&sources, &dest, MAX_KEPT_DUMPS, SystemTime::now());
        assert_eq!(again, HarvestReport::default());
    }

    #[test]
    fn harvest_never_overwrites_an_existing_dump() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path();
        let dest = crash_dumps_dir(data);
        touch(&dest.join("a.dmp"), b"old");
        touch(&pending_dir(data).join("a.dmp"), b"new");
        let report = harvest(
            &harvest_sources(data, None),
            &dest,
            MAX_KEPT_DUMPS,
            SystemTime::now(),
        );
        assert_eq!(report.moved, 1);
        assert_eq!(fs::read(dest.join("a.dmp")).unwrap(), b"old");
        assert_eq!(fs::read(dest.join("a-1.dmp")).unwrap(), b"new");
    }

    #[test]
    fn harvest_keeps_only_the_newest_dumps() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path();
        let dest = crash_dumps_dir(data);
        for i in 0..5u64 {
            let path = dest.join(format!("old-{i}.dmp"));
            touch(&path, b"x");
            set_age(&path, Duration::from_secs(3_600 * (10 - i)));
        }
        touch(&pending_dir(data).join("fresh.dmp"), b"y");
        let report = harvest(&harvest_sources(data, None), &dest, 3, SystemTime::now());
        assert_eq!(report.moved, 1);
        assert_eq!(report.pruned, 3);
        assert_eq!(
            sorted_files(&dest),
            vec!["fresh.dmp", "old-3.dmp", "old-4.dmp"]
        );
    }

    #[test]
    fn registration_reports_unsupported_off_windows() {
        if cfg!(windows) {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(platform::register(tmp.path()), Registration::Unsupported);
    }
}
