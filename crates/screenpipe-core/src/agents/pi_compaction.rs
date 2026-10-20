// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! The Freenpipe runtime patch for the installed Pi.
//!
//! Pi 0.84.1 compacts a session only when a run ends or before the next
//! prompt, keeps fixed token budgets that do not fit small local models, and
//! reports network failures without their error codes. The patch in
//! `assets/pi-patches/` changes that: it writes `freenpipe-compaction.js`
//! into the installed package and inserts a handful of one-line hooks into
//! Pi's own files (the behaviour is described in that module).
//!
//! Applying is careful by design:
//!
//! * it only touches the exact release it was written for: every package
//!   named by a `target` line must have that version, and every edited file
//!   must match its recorded stock SHA-256 (or already carry all of its
//!   markers);
//! * every hunk's text must occur exactly once, and all files are checked
//!   before any is written, so a mismatch changes nothing;
//! * it is idempotent: markers in the edited files show what is applied;
//! * when anything does not match, Pi keeps running unpatched and the reason
//!   is logged once per process;
//! * `SCREENPIPE_DISABLE_PI_PATCHES=1` turns it off and restores the stock
//!   files by reversing the hunks.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use anyhow::{anyhow, bail, Context, Result};
use tracing::{info, warn};

use super::bun_runtime::{env_flag_on, sha256_hex};
use super::pi_runtime::{self, write_bytes_atomic};
use crate::env;

/// The patch text, as shipped.
pub const PATCH_SOURCE: &str = include_str!("../../assets/pi-patches/pi-0.84.1.fpatch");
/// The JS module the hooks call into.
pub const MODULE_SOURCE: &str = include_str!("../../assets/pi-patches/freenpipe-compaction.js");
/// Prefix of the marker every hunk's replacement carries.
pub const MARKER_PREFIX: &str = "freenpipe-patch:";

/// One edit in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub id: String,
    pub find: String,
    pub replace: String,
}

impl Hunk {
    /// The marker that shows this hunk is applied.
    pub fn marker(&self) -> String {
        format!("{MARKER_PREFIX}{}", self.id)
    }
}

/// The edits to one file of one package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePatch {
    pub package: String,
    pub path: String,
    pub stock_sha256: String,
    pub hunks: Vec<Hunk>,
}

/// A parsed patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    /// `(package, version)` pairs the patch is written for.
    pub targets: Vec<(String, String)>,
    /// `(package, path)` where the JS module is written.
    pub module: (String, String),
    pub files: Vec<FilePatch>,
}

fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// Parse the patch format described at the top of the `.fpatch` file.
pub fn parse(text: &str) -> Result<Patch> {
    let text = normalize_newlines(text);
    let mut targets = Vec::new();
    let mut module = None;
    let mut files: Vec<FilePatch> = Vec::new();
    let mut lines = text.lines().enumerate().peekable();

    // Read literal lines up to the next "@@@ <word>" line; returns the text
    // and that word.
    fn block<'a>(
        lines: &mut std::iter::Peekable<impl Iterator<Item = (usize, &'a str)>>,
    ) -> Result<(String, String)> {
        let mut body: Vec<&str> = Vec::new();
        for (_, line) in lines.by_ref() {
            if let Some(word) = line.strip_prefix("@@@ ") {
                return Ok((body.join("\n"), word.trim().to_string()));
            }
            body.push(line);
        }
        bail!("unterminated @@@ block")
    }

    while let Some((n, line)) = lines.next() {
        let line_no = n + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut words = trimmed.split_whitespace();
        match words.next() {
            Some("target") => {
                let (Some(pkg), Some(ver), None) = (words.next(), words.next(), words.next()) else {
                    bail!("line {line_no}: expected `target <package> <version>`");
                };
                targets.push((pkg.to_string(), ver.to_string()));
            }
            Some("module") => {
                let (Some(pkg), Some(path), None) = (words.next(), words.next(), words.next()) else {
                    bail!("line {line_no}: expected `module <package> <path>`");
                };
                if module.is_some() {
                    bail!("line {line_no}: only one module line is allowed");
                }
                module = Some((pkg.to_string(), path.to_string()));
            }
            Some("file") => {
                let (Some(pkg), Some(path), Some(sha), None) =
                    (words.next(), words.next(), words.next(), words.next())
                else {
                    bail!("line {line_no}: expected `file <package> <path> <sha256>`");
                };
                if sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
                    bail!("line {line_no}: bad sha256");
                }
                files.push(FilePatch {
                    package: pkg.to_string(),
                    path: path.to_string(),
                    stock_sha256: sha.to_ascii_lowercase(),
                    hunks: Vec::new(),
                });
            }
            Some("hunk") => {
                let (Some(id), None) = (words.next(), words.next()) else {
                    bail!("line {line_no}: expected `hunk <id>`");
                };
                let file = files
                    .last_mut()
                    .ok_or_else(|| anyhow!("line {line_no}: hunk before any file line"))?;
                match lines.next() {
                    Some((_, l)) if l.trim() == "@@@ find" => {}
                    _ => bail!("line {line_no}: hunk {id} must start with `@@@ find`"),
                }
                let (find, next) = block(&mut lines)?;
                if next != "replace" {
                    bail!("hunk {id}: expected `@@@ replace` after the find text");
                }
                let (replace, end) = block(&mut lines)?;
                if end != "end" {
                    bail!("hunk {id}: expected `@@@ end`");
                }
                let hunk = Hunk {
                    id: id.to_string(),
                    find,
                    replace,
                };
                if hunk.find.is_empty() {
                    bail!("hunk {id}: empty find text");
                }
                if !hunk.replace.contains(&hunk.marker()) {
                    bail!("hunk {id}: replacement lacks its marker {}", hunk.marker());
                }
                if hunk.find.contains(MARKER_PREFIX) {
                    bail!("hunk {id}: find text must not contain a marker");
                }
                file.hunks.push(hunk);
            }
            Some(other) => bail!("line {line_no}: unknown directive `{other}`"),
            None => {}
        }
    }

    let module = module.ok_or_else(|| anyhow!("the patch has no module line"))?;
    if targets.is_empty() {
        bail!("the patch has no target line");
    }
    let mut ids = std::collections::HashSet::new();
    for f in &files {
        if f.hunks.is_empty() {
            bail!("{} {} has no hunks", f.package, f.path);
        }
        for h in &f.hunks {
            if !ids.insert(h.id.clone()) {
                bail!("hunk id {} is used twice", h.id);
            }
        }
    }
    Ok(Patch {
        targets,
        module,
        files,
    })
}

/// The patch shipped with this build.
pub fn bundled() -> Result<&'static Patch> {
    static PATCH: OnceLock<std::result::Result<Patch, String>> = OnceLock::new();
    PATCH
        .get_or_init(|| parse(PATCH_SOURCE).map_err(|e| format!("{e:#}")))
        .as_ref()
        .map_err(|e| anyhow!("bundled Pi patch is invalid: {e}"))
}

/// Where a file of the patch stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileState {
    /// Unchanged stock release.
    Stock,
    /// Every hunk is applied.
    Patched,
    /// Neither: another release, a local edit, or a half-applied state.
    Mismatch(String),
}

/// Classify a file's content.
pub fn file_state(file: &FilePatch, content: &str) -> FileState {
    let present = file
        .hunks
        .iter()
        .filter(|h| content.contains(&h.marker()))
        .count();
    if present == file.hunks.len() {
        return FileState::Patched;
    }
    if present > 0 {
        return FileState::Mismatch(format!(
            "{} has {present} of {} patch markers",
            file.path,
            file.hunks.len()
        ));
    }
    if sha256_hex(content.as_bytes()) == file.stock_sha256 {
        FileState::Stock
    } else {
        FileState::Mismatch(format!("{} is not the stock release file", file.path))
    }
}

fn replace_once(text: &str, from: &str, to: &str, what: &str) -> Result<String> {
    let count = text.matches(from).count();
    if count != 1 {
        bail!("{what}: expected the text once, found it {count} times");
    }
    Ok(text.replacen(from, to, 1))
}

/// Apply every hunk of `file` to its stock content.
pub fn patch_text(file: &FilePatch, stock: &str) -> Result<String> {
    let mut text = stock.to_string();
    for hunk in &file.hunks {
        text = replace_once(&text, &hunk.find, &hunk.replace, &format!("hunk {}", hunk.id))?;
    }
    Ok(text)
}

/// Undo every hunk of `file`, and check the result is the stock file.
pub fn unpatch_text(file: &FilePatch, patched: &str) -> Result<String> {
    let mut text = patched.to_string();
    for hunk in file.hunks.iter().rev() {
        text = replace_once(&text, &hunk.replace, &hunk.find, &format!("undo {}", hunk.id))?;
    }
    if sha256_hex(text.as_bytes()) != file.stock_sha256 {
        bail!("{}: undoing the patch did not give the stock file", file.path);
    }
    Ok(text)
}

/// Directory of an installed package: hoisted into the install's
/// `node_modules`, or nested under the agent package.
pub fn package_dir(install_dir: &Path, package: &str) -> Option<PathBuf> {
    let top = pi_runtime::node_module_dir(install_dir, package);
    if top.join("package.json").is_file() {
        return Some(top);
    }
    let nested = pi_runtime::node_module_dir(&pi_runtime::package_dir(install_dir), package);
    nested.join("package.json").is_file().then_some(nested)
}

fn package_version(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("package.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("version")?.as_str().map(str::to_string)
}

/// What a sync did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// There is no Pi install to patch.
    NotInstalled,
    /// The patch was written now.
    Applied,
    /// The install already carried the patch.
    AlreadyApplied,
    /// The patch was removed (patches are disabled).
    Removed,
    /// Patches are disabled and none was present.
    Disabled,
    /// The install does not match the patch; Pi runs unpatched.
    Skipped(String),
}

struct Planned {
    path: PathBuf,
    new_content: Option<String>,
}

fn plan_files(install_dir: &Path, patch: &Patch, apply: bool) -> Result<Vec<Planned>> {
    let mut planned = Vec::new();
    for file in &patch.files {
        let dir = package_dir(install_dir, &file.package)
            .ok_or_else(|| anyhow!("{} is not installed", file.package))?;
        let path = dir.join(&file.path);
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("read {}", path.display()))?;
        let new_content = match (file_state(file, &content), apply) {
            (FileState::Stock, true) => Some(patch_text(file, &content)?),
            (FileState::Patched, true) | (FileState::Stock, false) => None,
            (FileState::Patched, false) => Some(unpatch_text(file, &content)?),
            (FileState::Mismatch(why), _) => bail!(why),
        };
        planned.push(Planned { path, new_content });
    }
    Ok(planned)
}

fn check_targets(install_dir: &Path, patch: &Patch) -> Result<()> {
    for (package, version) in &patch.targets {
        let dir = package_dir(install_dir, package)
            .ok_or_else(|| anyhow!("{package} is not installed"))?;
        match package_version(&dir) {
            Some(v) if &v == version => {}
            Some(v) => bail!("{package} is {v}, the patch is for {version}"),
            None => bail!("{package} has no readable version"),
        }
    }
    Ok(())
}

/// Apply `patch` (with `module_source`) to the install at `install_dir`.
pub fn apply_in(install_dir: &Path, patch: &Patch, module_source: &str) -> Result<Outcome> {
    if !pi_runtime::cli_entrypoint(install_dir).is_file() {
        return Ok(Outcome::NotInstalled);
    }
    check_targets(install_dir, patch)?;
    let planned = plan_files(install_dir, patch, true)?;

    let (module_pkg, module_rel) = &patch.module;
    let module_dir = package_dir(install_dir, module_pkg)
        .ok_or_else(|| anyhow!("{module_pkg} is not installed"))?;
    let module_path = module_dir.join(module_rel);
    let module_source = normalize_newlines(module_source);
    let module_current = std::fs::read_to_string(&module_path).ok();
    let module_changed = module_current.as_deref() != Some(module_source.as_str());

    if planned.iter().all(|p| p.new_content.is_none()) && !module_changed {
        return Ok(Outcome::AlreadyApplied);
    }
    // The module first: a hook must never point at a missing module.
    if module_changed {
        write_bytes_atomic(&module_path, module_source.as_bytes())?;
    }
    for p in &planned {
        if let Some(ref text) = p.new_content {
            write_bytes_atomic(&p.path, text.as_bytes())?;
        }
    }
    Ok(Outcome::Applied)
}

/// Restore the stock files of `patch` at `install_dir`.
pub fn remove_in(install_dir: &Path, patch: &Patch) -> Result<Outcome> {
    if !pi_runtime::cli_entrypoint(install_dir).is_file() {
        return Ok(Outcome::NotInstalled);
    }
    let planned = plan_files(install_dir, patch, false)?;
    if planned.iter().all(|p| p.new_content.is_none()) {
        return Ok(Outcome::Disabled);
    }
    for p in &planned {
        if let Some(ref text) = p.new_content {
            write_bytes_atomic(&p.path, text.as_bytes())?;
        }
    }
    // The module goes last, once nothing imports it.
    let (module_pkg, module_rel) = &patch.module;
    if let Some(dir) = package_dir(install_dir, module_pkg) {
        let _ = std::fs::remove_file(dir.join(module_rel));
    }
    Ok(Outcome::Removed)
}

static LOGGED_SKIP: AtomicBool = AtomicBool::new(false);

/// Bring the install at `install_dir` in line with the patch setting:
/// apply the bundled patch, or remove it when
/// `SCREENPIPE_DISABLE_PI_PATCHES=1`. Never fails: problems are logged
/// (once per process) and Pi keeps running unpatched.
pub fn sync(install_dir: &Path) -> Outcome {
    let disabled = env_flag_on(env::non_empty(env::SCREENPIPE_DISABLE_PI_PATCHES).as_deref());
    let result = bundled().and_then(|patch| {
        if disabled {
            remove_in(install_dir, patch)
        } else {
            apply_in(install_dir, patch, MODULE_SOURCE)
        }
    });
    match result {
        Ok(outcome) => {
            match outcome {
                Outcome::Applied => info!(
                    "pi runtime patch applied (compaction and transport errors) in {}",
                    install_dir.display()
                ),
                Outcome::Removed => info!("pi runtime patch removed (patches are disabled)"),
                _ => {}
            }
            outcome
        }
        Err(e) => {
            let reason = format!("{e:#}");
            if !LOGGED_SKIP.swap(true, Ordering::Relaxed) {
                warn!(
                    "pi runtime patch not {}; Pi runs unpatched: {}",
                    if disabled { "removed" } else { "applied" },
                    reason
                );
            }
            Outcome::Skipped(reason)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_patch(stock: &str) -> String {
        format!(
            "# test\n\
target @earendil-works/pi-coding-agent 0.84.1\n\
module @earendil-works/pi-coding-agent dist/core/compaction/fp.js\n\
file @earendil-works/pi-coding-agent dist/a.js {}\n\
hunk one-v1\n\
@@@ find\n\
const a = 1;\n\
@@@ replace\n\
const a = 2; // freenpipe-patch:one-v1\n\
@@@ end\n\
hunk two-v1\n\
@@@ find\n\
function f() {{\n\
    return a;\n\
@@@ replace\n\
function f() {{\n\
    // freenpipe-patch:two-v1\n\
    return a + 1;\n\
@@@ end\n",
            sha256_hex(stock.as_bytes())
        )
    }

    const STOCK: &str = "const a = 1;\nfunction f() {\n    return a;\n}\n";

    fn install(dir: &Path, version: &str, stock: &str) {
        let pkg = pi_runtime::package_dir(dir);
        std::fs::create_dir_all(pkg.join("dist")).unwrap();
        std::fs::write(
            pkg.join("package.json"),
            format!(r#"{{"name":"@earendil-works/pi-coding-agent","version":"{version}"}}"#),
        )
        .unwrap();
        std::fs::write(pkg.join("dist").join("cli.js"), "").unwrap();
        std::fs::write(pkg.join("dist").join("a.js"), stock).unwrap();
    }

    #[test]
    fn the_bundled_patch_parses_and_targets_0_84_1() {
        let patch = bundled().unwrap();
        assert!(patch
            .targets
            .contains(&(pi_runtime::PI_CODING_AGENT.to_string(), "0.84.1".to_string())));
        assert!(patch
            .targets
            .contains(&(pi_runtime::PI_AI.to_string(), "0.84.1".to_string())));
        assert_eq!(patch.files.len(), 4);
        assert_eq!(patch.module.0, pi_runtime::PI_CODING_AGENT);
        // The hooks import the module by these names.
        for export in [
            "export async function afterToolStep",
            "export function settingsForSession",
            "export function noteCompactionFailure",
            "export function checkSummary",
            "export function serializeConversationRecords",
        ] {
            assert!(MODULE_SOURCE.contains(export), "{export}");
        }
        assert!(PATCH_SOURCE.contains(pi_runtime::PI_VERSION));
    }

    #[test]
    fn crlf_checkouts_parse_to_the_same_patch() {
        let lf = fixture_patch(STOCK);
        let crlf = lf.replace('\n', "\r\n");
        assert_eq!(parse(&lf).unwrap(), parse(&crlf).unwrap());
    }

    #[test]
    fn malformed_patches_are_rejected() {
        let good = fixture_patch(STOCK);
        assert!(parse(&good).is_ok());
        let no_marker = good.replace("// freenpipe-patch:one-v1", "");
        assert!(parse(&no_marker).is_err());
        let unterminated = good.replace("@@@ end\nhunk two-v1", "hunk two-v1");
        assert!(parse(&unterminated).is_err());
        let dup = good.replace("hunk two-v1", "hunk one-v1").replace("patch:two-v1", "patch:one-v1");
        assert!(parse(&dup).is_err());
        assert!(parse("target a 1\n").is_err());
        assert!(parse(&good.replace("target", "tarjet")).is_err());
    }

    #[test]
    fn a_stock_install_is_patched_once_and_can_be_restored() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path(), "0.84.1", STOCK);
        let patch = parse(&fixture_patch(STOCK)).unwrap();
        let file = pi_runtime::package_dir(dir.path()).join("dist/a.js");
        let module = pi_runtime::package_dir(dir.path()).join("dist/core/compaction/fp.js");

        assert_eq!(apply_in(dir.path(), &patch, "export {};\r\n").unwrap(), Outcome::Applied);
        let patched = std::fs::read_to_string(&file).unwrap();
        assert!(patched.contains("const a = 2; // freenpipe-patch:one-v1"));
        assert!(patched.contains("return a + 1;"));
        assert_eq!(std::fs::read_to_string(&module).unwrap(), "export {};\n");

        assert_eq!(
            apply_in(dir.path(), &patch, "export {};\n").unwrap(),
            Outcome::AlreadyApplied
        );
        // A changed module is refreshed even when the hooks are in place.
        assert_eq!(
            apply_in(dir.path(), &patch, "export const v = 2;\n").unwrap(),
            Outcome::Applied
        );

        assert_eq!(remove_in(dir.path(), &patch).unwrap(), Outcome::Removed);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), STOCK);
        assert!(!module.exists());
        assert_eq!(remove_in(dir.path(), &patch).unwrap(), Outcome::Disabled);
    }

    #[test]
    fn another_release_or_a_modified_file_is_left_alone() {
        let patch = parse(&fixture_patch(STOCK)).unwrap();

        let dir = tempfile::tempdir().unwrap();
        install(dir.path(), "0.84.4", STOCK);
        let err = apply_in(dir.path(), &patch, "x").unwrap_err();
        assert!(format!("{err:#}").contains("0.84.4"));

        let dir = tempfile::tempdir().unwrap();
        let edited = STOCK.replace("return a;", "return a * 3;");
        install(dir.path(), "0.84.1", &edited);
        assert!(apply_in(dir.path(), &patch, "x").is_err());
        let file = pi_runtime::package_dir(dir.path()).join("dist/a.js");
        assert_eq!(std::fs::read_to_string(file).unwrap(), edited);
        let module = pi_runtime::package_dir(dir.path()).join("dist/core/compaction/fp.js");
        assert!(!module.exists(), "nothing may be written on a mismatch");
    }

    #[test]
    fn a_half_patched_file_is_a_mismatch() {
        let patch = parse(&fixture_patch(STOCK)).unwrap();
        let file = &patch.files[0];
        let half = STOCK.replace("const a = 1;", "const a = 2; // freenpipe-patch:one-v1");
        assert!(matches!(file_state(file, &half), FileState::Mismatch(_)));
        assert_eq!(file_state(file, STOCK), FileState::Stock);
        let full = patch_text(file, STOCK).unwrap();
        assert_eq!(file_state(file, &full), FileState::Patched);
        assert_eq!(unpatch_text(file, &full).unwrap(), STOCK);
    }

    #[test]
    fn a_hunk_whose_text_occurs_twice_is_refused() {
        let doubled = format!("{STOCK}const a = 1;\n");
        let patch = parse(&fixture_patch(&doubled)).unwrap();
        assert!(patch_text(&patch.files[0], &doubled).is_err());
    }

    #[test]
    fn no_install_means_nothing_to_do() {
        let dir = tempfile::tempdir().unwrap();
        let patch = parse(&fixture_patch(STOCK)).unwrap();
        assert_eq!(apply_in(dir.path(), &patch, "x").unwrap(), Outcome::NotInstalled);
        assert_eq!(sync(dir.path()), Outcome::NotInstalled);
    }

    #[test]
    fn nested_packages_are_found() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path(), "0.84.1", STOCK);
        let nested = pi_runtime::node_module_dir(
            &pi_runtime::package_dir(dir.path()),
            pi_runtime::PI_AI,
        );
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("package.json"), r#"{"version":"0.84.1"}"#).unwrap();
        assert_eq!(package_dir(dir.path(), pi_runtime::PI_AI), Some(nested));
    }

    /// The bundled patch against a real Pi 0.84.1 install. Run with
    /// `FREENPIPE_PI_TEST_INSTALL=<dir with node_modules> cargo test -p
    /// screenpipe-core --lib pi_compaction -- --ignored`.
    #[test]
    #[ignore]
    fn the_bundled_patch_applies_to_a_real_install() {
        let Some(dir) = std::env::var_os("FREENPIPE_PI_TEST_INSTALL") else {
            eprintln!("FREENPIPE_PI_TEST_INSTALL is not set; skipping");
            return;
        };
        let dir = PathBuf::from(dir);
        let patch = bundled().unwrap();
        let first = apply_in(&dir, patch, MODULE_SOURCE).unwrap();
        assert!(matches!(first, Outcome::Applied | Outcome::AlreadyApplied));
        assert_eq!(
            apply_in(&dir, patch, MODULE_SOURCE).unwrap(),
            Outcome::AlreadyApplied
        );
        if std::env::var_os("FREENPIPE_PI_TEST_KEEP").is_none() {
            assert_eq!(remove_in(&dir, patch).unwrap(), Outcome::Removed);
            assert_eq!(apply_in(&dir, patch, MODULE_SOURCE).unwrap(), Outcome::Applied);
        }
    }

    /// Runs the module's bun tests when bun is on PATH (CI installs it).
    #[test]
    fn the_compaction_module_passes_its_bun_tests() {
        let Ok(bun) = which::which("bun") else {
            eprintln!("bun is not on PATH; skipping the JS tests");
            return;
        };
        let test = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("pi-patches")
            .join("freenpipe-compaction.test.ts");
        let output = std::process::Command::new(bun)
            .arg("test")
            .arg(&test)
            .current_dir(test.parent().unwrap())
            .output()
            .expect("run bun test");
        assert!(
            output.status.success(),
            "bun test failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
