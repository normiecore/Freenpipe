// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Choosing the bash that Pi's bash tool runs on Windows.
//!
//! A `bash.exe` on a Windows machine is not always a usable bash: the WSL
//! launcher in `System32` starts a Linux distribution (or an install prompt
//! when none is set up), the Store's app-execution aliases are stubs, and a
//! broken Git install can hang. Every candidate is therefore started once
//! with a tiny script that prints a token, under a timeout; the first one that
//! answers wins. WSL launchers are never tried. A candidate that does not
//! answer in time gets one more, longer try (Git for Windows can be slow on
//! its first start after boot while a virus scanner checks it) before it is
//! rejected. When nothing answers, the runtime falls back to its own
//! PortableGit download (see `agents::pi`).
//!
//! Candidate ordering, WSL detection, the probe-output check and the retry
//! policy are pure and tested on every platform; only the probe itself is
//! Windows-only.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Text the probe script prints.
pub const PROBE_TOKEN: &str = "handvel-bash-probe-ok";
/// How long a candidate may take to answer the probe.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// The limit for the one retry of a candidate that timed out: a cold first
/// start of Git bash under on-access scanning can take well over 5 s.
pub const SLOW_START_TIMEOUT: Duration = Duration::from_secs(20);

/// How one probe of a candidate went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeResult {
    /// It printed the token: a working bash.
    Works,
    /// It ran but did not print the token (a WSL stub, a broken install).
    NoToken,
    /// It could not be started at all.
    CannotStart,
    /// It did not finish within the limit and was stopped.
    TimedOut,
}

/// The time limit for the next probe of a candidate, given the results of
/// the earlier probes of the same candidate, or `None` when it should be
/// rejected: the first probe gets [`PROBE_TIMEOUT`], a timeout earns one
/// retry with [`SLOW_START_TIMEOUT`], and anything else is final.
pub fn next_probe_timeout(earlier: &[ProbeResult]) -> Option<Duration> {
    match earlier {
        [] => Some(PROBE_TIMEOUT),
        [ProbeResult::TimedOut] => Some(SLOW_START_TIMEOUT),
        _ => None,
    }
}

/// Whether `path` is the WSL launcher or an app-execution alias rather than
/// a real bash.
pub fn is_wsl_launcher(path: &Path) -> bool {
    let lower = path
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    let launcher_dirs = [
        "\\windows\\system32\\",
        "\\windows\\sysnative\\",
        "\\windows\\syswow64\\",
    ];
    if lower.ends_with("\\bash.exe") && launcher_dirs.iter().any(|d| lower.contains(d)) {
        return true;
    }
    lower.contains("\\microsoft\\windowsapps\\")
}

/// Candidate bash executables in the order they are tried: the PortableGit
/// the runtime manages, the usual Git for Windows locations, then whatever
/// `where bash` printed. Duplicates and WSL launchers are dropped.
pub fn candidate_paths(
    local_app_data: Option<&Path>,
    program_files_dirs: &[PathBuf],
    where_output: &str,
) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        let key = p.to_string_lossy().replace('/', "\\").to_ascii_lowercase();
        let seen = out
            .iter()
            .any(|q| q.to_string_lossy().replace('/', "\\").to_ascii_lowercase() == key);
        if !seen && !is_wsl_launcher(&p) {
            out.push(p);
        }
    };
    if let Some(local) = local_app_data {
        push(
            local
                .join("screenpipe")
                .join("git-portable")
                .join("bin")
                .join("bash.exe"),
        );
    }
    for dir in program_files_dirs {
        push(dir.join("Git").join("bin").join("bash.exe"));
    }
    if let Some(local) = local_app_data {
        push(
            local
                .join("Programs")
                .join("Git")
                .join("bin")
                .join("bash.exe"),
        );
    }
    for line in where_output.lines() {
        let line = line.trim();
        if !line.is_empty() {
            push(PathBuf::from(line));
        }
    }
    out
}

/// Whether the probe's standard output shows a working bash.
pub fn probe_output_ok(stdout: &[u8]) -> bool {
    String::from_utf8_lossy(stdout).contains(PROBE_TOKEN)
}

/// Run the probe script with `bash` and wait at most `timeout`. On a
/// timeout the whole process tree is stopped: Git for Windows'
/// `bin\bash.exe` is a launcher that starts the real bash as a child.
#[cfg(windows)]
pub fn probe(bash: &Path, timeout: Duration) -> ProbeResult {
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let child = Command::new(bash)
        .args(["--noprofile", "--norc", "-c"])
        .arg(format!("printf {PROBE_TOKEN}"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("bash probe: cannot start {}: {}", bash.display(), e);
            return ProbeResult::CannotStart;
        }
    };
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(25));
            }
            _ => {
                // taskkill /T also stops the bash the launcher started.
                let _ = Command::new("taskkill")
                    .args(["/F", "/T", "/PID", &child.id().to_string()])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .creation_flags(CREATE_NO_WINDOW)
                    .status();
                let _ = child.kill();
                let _ = child.wait();
                tracing::warn!(
                    "bash probe: {} did not answer within {:?}",
                    bash.display(),
                    timeout
                );
                return ProbeResult::TimedOut;
            }
        }
    }
    let mut out = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_end(&mut out);
    }
    if probe_output_ok(&out) {
        ProbeResult::Works
    } else {
        tracing::debug!("bash probe: {} gave no probe output", bash.display());
        ProbeResult::NoToken
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsl_launchers_and_store_aliases_are_recognised() {
        for p in [
            r"C:\Windows\System32\bash.exe",
            r"c:\windows\sysnative\BASH.EXE",
            r"C:\Windows\SysWOW64\bash.exe",
            r"C:\Users\me\AppData\Local\Microsoft\WindowsApps\bash.exe",
            "C:/Windows/System32/bash.exe",
        ] {
            assert!(is_wsl_launcher(Path::new(p)), "{p}");
        }
        for p in [
            r"C:\Program Files\Git\bin\bash.exe",
            r"C:\Users\me\AppData\Local\screenpipe\git-portable\bin\bash.exe",
            r"D:\msys64\usr\bin\bash.exe",
        ] {
            assert!(!is_wsl_launcher(Path::new(p)), "{p}");
        }
    }

    #[test]
    fn candidates_come_in_preference_order_without_wsl_or_duplicates() {
        let local = PathBuf::from(r"C:\Users\me\AppData\Local");
        let pf = vec![
            PathBuf::from(r"C:\Program Files"),
            PathBuf::from(r"C:\Program Files (x86)"),
        ];
        let where_out = "C:\\Windows\\System32\\bash.exe\r\nC:\\Program Files\\Git\\bin\\bash.exe\r\nD:\\msys64\\usr\\bin\\bash.exe\r\n";
        let got: Vec<String> = candidate_paths(Some(&local), &pf, where_out)
            .into_iter()
            .map(|p| p.to_string_lossy().replace('/', "\\"))
            .collect();
        assert_eq!(
            got,
            vec![
                r"C:\Users\me\AppData\Local\screenpipe\git-portable\bin\bash.exe",
                r"C:\Program Files\Git\bin\bash.exe",
                r"C:\Program Files (x86)\Git\bin\bash.exe",
                r"C:\Users\me\AppData\Local\Programs\Git\bin\bash.exe",
                r"D:\msys64\usr\bin\bash.exe",
            ]
        );
    }

    #[test]
    fn a_slow_first_start_gets_one_longer_retry() {
        use ProbeResult::*;
        assert_eq!(next_probe_timeout(&[]), Some(PROBE_TIMEOUT));
        assert_eq!(next_probe_timeout(&[TimedOut]), Some(SLOW_START_TIMEOUT));
        assert!(SLOW_START_TIMEOUT > PROBE_TIMEOUT);
        // Two timeouts, or an answer without the token, are final.
        assert_eq!(next_probe_timeout(&[TimedOut, TimedOut]), None);
        assert_eq!(next_probe_timeout(&[NoToken]), None);
        assert_eq!(next_probe_timeout(&[CannotStart]), None);
        assert_eq!(next_probe_timeout(&[TimedOut, NoToken]), None);
    }

    #[test]
    fn only_the_token_counts_as_a_working_bash() {
        assert!(probe_output_ok(PROBE_TOKEN.as_bytes()));
        assert!(probe_output_ok(format!("noise\n{PROBE_TOKEN}").as_bytes()));
        assert!(!probe_output_ok(b""));
        assert!(!probe_output_ok(
            b"Windows Subsystem for Linux has no installed distributions."
        ));
    }
}
