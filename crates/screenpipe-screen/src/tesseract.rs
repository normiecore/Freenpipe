use image::{DynamicImage, GenericImageView};
use rusty_tesseract::{Args, Data, DataOutput, Image};
use screenpipe_core::{Language, TESSERACT_LANGUAGES};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;
use tracing::{debug, info, warn};

// Tesseract discovery (Handvel): the binary is looked up once per session,
// installed system-wide or bundled beside the engine (including an AppImage's
// `$APPDIR`). When none is found, OCR through tesseract stays off for the rest
// of the session instead of failing on every frame. The binary found is run
// by its full path, and the language data folder is given to that child
// process only: discovery never changes this process's environment, which
// other threads (and C libraries reading it) may be using at the same time.

#[cfg(windows)]
const TESSERACT_EXE: &str = "tesseract.exe";
#[cfg(not(windows))]
const TESSERACT_EXE: &str = "tesseract";

/// Language data file whose presence marks a usable tessdata folder.
const ENG_TRAINEDDATA: &str = "eng.traineddata";

/// Tesseract is the engine live capture uses on this platform (Linux and
/// other non-Apple, non-Windows systems), so its state feeds `/health`.
const TESSERACT_IS_PRIMARY: bool = cfg!(not(any(target_os = "windows", target_os = "macos")));

/// Consecutive failed runs after which tesseract is reported unavailable.
const FAILURES_BEFORE_UNAVAILABLE: u32 = 3;

/// Keeps a console window from flashing up for each tesseract run.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Where tesseract was found for this session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TesseractInstall {
    /// The tesseract executable.
    pub binary: PathBuf,
    /// The tessdata folder given to tesseract as `TESSDATA_PREFIX`: the
    /// user's own setting, or the folder found beside the binary or in a
    /// distro location. `None` leaves tesseract to its built-in default.
    pub tessdata: Option<PathBuf>,
}

static TESSERACT: OnceLock<Option<TesseractInstall>> = OnceLock::new();
static CONSECUTIVE_FAILURES: AtomicU32 = AtomicU32::new(0);

/// The tesseract found for this session, discovering it on first use.
/// `None` means OCR through tesseract is off until the engine restarts.
pub fn tesseract_install() -> Option<&'static TesseractInstall> {
    TESSERACT.get_or_init(discover).as_ref()
}

/// Whether a tesseract binary was found for this session.
pub fn tesseract_available() -> bool {
    tesseract_install().is_some()
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
}

/// Folders where an installer usually puts tesseract but which are often
/// missing from the engine's `PATH` (GUI launches inherit a short one).
fn well_known_install_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(windows)]
    {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
            if let Some(base) = std::env::var_os(var) {
                dirs.push(PathBuf::from(base).join("Tesseract-OCR"));
            }
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let local = PathBuf::from(local);
            dirs.push(local.join("Programs").join("Tesseract-OCR"));
            dirs.push(local.join("Tesseract-OCR"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        for d in ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"] {
            dirs.push(PathBuf::from(d));
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        for d in ["/usr/bin", "/usr/local/bin", "/snap/bin"] {
            dirs.push(PathBuf::from(d));
        }
    }
    dirs
}

/// The places rusty-tesseract's own lookup searches, in its order: every
/// `PATH` folder, the working directory, the engine's folder (plus its `lib`
/// on Linux and `../Resources` on macOS) and `~/.local/bin`. Checked here
/// rather than through `rusty_tesseract::find_tesseract_path`, which logs an
/// error when nothing is found; on Windows and macOS tesseract is optional
/// and its absence is normal.
fn standard_binary_candidates(
    path_var: Option<&std::ffi::OsStr>,
    cwd: Option<&Path>,
    exe_dir: Option<&Path>,
    home: Option<&Path>,
) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = path_var
        .map(|p| {
            std::env::split_paths(p)
                .filter(|dir| !dir.as_os_str().is_empty())
                .map(|dir| dir.join(TESSERACT_EXE))
                .collect()
        })
        .unwrap_or_default();
    if let Some(dir) = cwd {
        out.push(dir.join(TESSERACT_EXE));
    }
    if let Some(dir) = exe_dir {
        out.push(dir.join(TESSERACT_EXE));
        #[cfg(target_os = "macos")]
        out.push(dir.join("..").join("Resources").join(TESSERACT_EXE));
        #[cfg(target_os = "linux")]
        out.push(dir.join("lib").join(TESSERACT_EXE));
    }
    if let Some(dir) = home {
        out.push(dir.join(".local").join("bin").join(TESSERACT_EXE));
    }
    out
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let var = "USERPROFILE";
    #[cfg(not(windows))]
    let var = "HOME";
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Binary locations tried after the standard search (see
/// [`standard_binary_candidates`]): bundle folders beside the engine, the
/// AppImage tree, then installer defaults.
fn extra_binary_candidates(
    exe_dir: Option<&Path>,
    appdir: Option<&Path>,
    install_dirs: &[PathBuf],
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(dir) = exe_dir {
        out.push(dir.join("tesseract").join(TESSERACT_EXE));
        out.push(dir.join("resources").join("tesseract").join(TESSERACT_EXE));
        out.push(dir.join("Tesseract-OCR").join(TESSERACT_EXE));
    }
    if let Some(dir) = appdir {
        out.push(dir.join("usr").join("bin").join(TESSERACT_EXE));
        out.push(dir.join("bin").join(TESSERACT_EXE));
    }
    out.extend(install_dirs.iter().map(|d| d.join(TESSERACT_EXE)));
    out
}

/// Folders that may hold the language data, best first.
fn tessdata_candidates(
    binary: &Path,
    exe_dir: Option<&Path>,
    appdir: Option<&Path>,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(dir) = binary.parent() {
        out.push(dir.join("tessdata"));
        if let Some(prefix) = dir.parent() {
            out.push(prefix.join("share").join("tessdata"));
            out.push(
                prefix
                    .join("share")
                    .join("tesseract-ocr")
                    .join("5")
                    .join("tessdata"),
            );
            out.push(
                prefix
                    .join("share")
                    .join("tesseract-ocr")
                    .join("4")
                    .join("tessdata"),
            );
        }
    }
    if let Some(dir) = exe_dir {
        out.push(dir.join("tessdata"));
        out.push(dir.join("tesseract").join("tessdata"));
        out.push(dir.join("resources").join("tesseract").join("tessdata"));
    }
    if let Some(dir) = appdir {
        let share = dir.join("usr").join("share");
        out.push(share.join("tesseract-ocr").join("5").join("tessdata"));
        out.push(share.join("tesseract-ocr").join("4").join("tessdata"));
        out.push(share.join("tessdata"));
    }
    // Common distro paths for tessdata
    for path in [
        "/usr/share/tesseract-ocr/5/tessdata",
        "/usr/share/tesseract-ocr/4/tessdata",
        "/usr/share/tesseract/tessdata",
        "/usr/share/tessdata",
        "/usr/local/share/tessdata",
    ] {
        out.push(PathBuf::from(path));
    }
    out
}

fn first_file(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| p.is_file()).cloned()
}

fn first_tessdata_with_eng(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates
        .iter()
        .find(|dir| dir.join(ENG_TRAINEDDATA).is_file())
        .cloned()
}

fn discover() -> Option<TesseractInstall> {
    let exe_dir = exe_dir();
    let appdir = std::env::var_os("APPDIR").map(PathBuf::from);

    let path_var = std::env::var_os("PATH");
    let cwd = std::env::current_dir().ok();
    let found = standard_binary_candidates(
        path_var.as_deref(),
        cwd.as_deref(),
        exe_dir.as_deref(),
        home_dir().as_deref(),
    )
    .into_iter()
    .find(|path| path.is_file())
    .or_else(|| {
        first_file(&extra_binary_candidates(
            exe_dir.as_deref(),
            appdir.as_deref(),
            &well_known_install_dirs(),
        ))
    });
    let Some(binary) = found else {
        if TESSERACT_IS_PRIMARY {
            warn!(
                "tesseract was not found (it is neither installed system-wide nor bundled beside \
                 the engine), so OCR through tesseract is off for the rest of this session; \
                 install tesseract-ocr and restart to get searchable screen text"
            );
            crate::metrics::set_ocr_engine_available(false);
        } else {
            // Optional here: live capture uses the platform's own OCR engine.
            debug!(
                "tesseract not found; the optional tesseract OCR engine is off for this session"
            );
        }
        return None;
    };

    // The language data folder for tesseract's TESSDATA_PREFIX: the user's
    // own setting wins, otherwise the first folder holding English data.
    let tessdata = match std::env::var_os("TESSDATA_PREFIX").filter(|v| !v.is_empty()) {
        Some(user_set) => Some(PathBuf::from(user_set)),
        None => first_tessdata_with_eng(&tessdata_candidates(
            &binary,
            exe_dir.as_deref(),
            appdir.as_deref(),
        )),
    };
    info!(
        binary = %binary.display(),
        tessdata = %tessdata.as_deref().map(|p| p.display().to_string()).unwrap_or_else(|| "default".into()),
        "tesseract found"
    );
    Some(TesseractInstall { binary, tessdata })
}

fn note_tesseract_success() {
    CONSECUTIVE_FAILURES.store(0, Ordering::Relaxed);
    if TESSERACT_IS_PRIMARY {
        crate::metrics::set_ocr_engine_available(true);
    }
}

fn note_tesseract_failure() {
    let failures = CONSECUTIVE_FAILURES.fetch_add(1, Ordering::Relaxed) + 1;
    if TESSERACT_IS_PRIMARY && failures == FAILURES_BEFORE_UNAVAILABLE {
        warn!(
            "tesseract failed {} times in a row; reporting OCR as unavailable until a run succeeds",
            failures
        );
        crate::metrics::set_ocr_engine_available(false);
    }
}

fn empty_ocr() -> (String, String, Option<f64>) {
    (String::new(), "[]".to_string(), None)
}

/// Runs `install`'s tesseract on one image file and returns its TSV word
/// data. The binary is started by its full path and `TESSDATA_PREFIX` is set
/// on the child process only.
fn run_tesseract_tsv(
    install: &TesseractInstall,
    image_path: &Path,
    args: &Args,
) -> Result<DataOutput, String> {
    let mut command = Command::new(&install.binary);
    command
        .arg(image_path)
        .arg("stdout")
        .arg("-l")
        .arg(&args.lang);
    if let Some(dpi) = args.dpi {
        command.arg("--dpi").arg(dpi.to_string());
    }
    if let Some(psm) = args.psm {
        command.arg("--psm").arg(psm.to_string());
    }
    if let Some(oem) = args.oem {
        command.arg("--oem").arg(oem.to_string());
    }
    for (key, value) in &args.config_variables {
        command.arg("-c").arg(format!("{}={}", key, value));
    }
    command.arg("tsv");
    if let Some(dir) = &install.tessdata {
        command.env("TESSDATA_PREFIX", dir);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let output = command
        .output()
        .map_err(|e| format!("could not run {}: {}", install.binary.display(), e))?;
    if !output.status.success() {
        return Err(format!(
            "tesseract exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let tsv = String::from_utf8_lossy(&output.stdout).into_owned();
    let data = parse_tsv(&tsv);
    Ok(DataOutput { output: tsv, data })
}

/// Rows of tesseract's TSV output, header line skipped. A row that does not
/// parse is left out rather than failing the whole frame.
fn parse_tsv(tsv: &str) -> Vec<Data> {
    tsv.lines().skip(1).filter_map(parse_tsv_row).collect()
}

fn next_column<T: std::str::FromStr>(columns: &mut std::str::Split<'_, char>) -> Option<T> {
    columns.next()?.trim().parse().ok()
}

/// One TSV row: eleven numeric columns, then the word (empty for the page,
/// block, paragraph and line rows).
fn parse_tsv_row(line: &str) -> Option<Data> {
    let mut columns = line.split('\t');
    Some(Data {
        level: next_column(&mut columns)?,
        page_num: next_column(&mut columns)?,
        block_num: next_column(&mut columns)?,
        par_num: next_column(&mut columns)?,
        line_num: next_column(&mut columns)?,
        word_num: next_column(&mut columns)?,
        left: next_column(&mut columns)?,
        top: next_column(&mut columns)?,
        width: next_column(&mut columns)?,
        height: next_column(&mut columns)?,
        conf: next_column(&mut columns)?,
        text: columns.next().unwrap_or("").trim().to_string(),
    })
}

pub fn perform_ocr_tesseract(
    image: &DynamicImage,
    languages: Vec<Language>,
) -> (String, String, Option<f64>) {
    let (img_w, img_h) = image.dimensions();
    if img_w == 0 || img_h == 0 {
        debug!("tesseract: skipping an image with a zero dimension");
        return empty_ocr();
    }
    if crate::ocr_availability::ocr_fault_injected() {
        if TESSERACT_IS_PRIMARY {
            crate::metrics::set_ocr_engine_available(false);
        }
        return empty_ocr();
    }
    let Some(install) = tesseract_install() else {
        // Logged once at discovery; OCR stays off for this session.
        return empty_ocr();
    };

    let language_string = match languages.is_empty() {
        true => "eng".to_string(),
        _ => TESSERACT_LANGUAGES
            .iter()
            .filter_map(|(key, val)| {
                if languages.iter().any(|l| l == val) {
                    Some(key.to_string())
                } else {
                    None
                }
            })
            .collect::<Vec<String>>()
            .join("+"),
    };
    let args = Args {
        lang: language_string,
        config_variables: HashMap::from([("tessedit_create_tsv".into(), "1".into())]),
        dpi: Some(600), // 150 is a balanced option, 600 seems faster surprisingly, the bigger the number the more granualar result
        psm: Some(1), // PSM 1: Automatic page segmentation with OSD. PSM 3: Automatic page segmentation with OSD
        oem: Some(1), //1: Neural nets LSTM engine only,    3: Default, based on what is available. (Default)
    };

    let ocr_image = match Image::from_dynamic_image(image) {
        Ok(img) => img,
        Err(e) => {
            warn!("tesseract: failed to convert image: {}", e);
            return empty_ocr();
        }
    };

    let image_path = match ocr_image.get_image_path() {
        Ok(path) => PathBuf::from(path),
        Err(e) => {
            warn!("tesseract: no file for the image: {}", e);
            return empty_ocr();
        }
    };

    // Extract data output (a binary removed after discovery is an error
    // here, not a panic).
    let data_output = match run_tesseract_tsv(install, &image_path, &args) {
        Ok(data) => {
            note_tesseract_success();
            data
        }
        Err(e) => {
            warn!("tesseract: OCR failed: {}", e);
            note_tesseract_failure();
            return empty_ocr();
        }
    };

    // Extract text from data output
    let text = data_output_to_text(&data_output);
    let json_output = data_output_to_json(&data_output, img_w, img_h);

    let overall_confidence = calculate_overall_confidence(&data_output);

    (text, json_output, Some(overall_confidence))
}

fn data_output_to_text(data_output: &DataOutput) -> String {
    let mut text = String::new();
    for record in &data_output.data {
        if !record.text.is_empty() {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&record.text);
        }
    }
    text
}

fn data_output_to_json(data_output: &DataOutput, img_w: u32, img_h: u32) -> String {
    let mut words: Vec<HashMap<String, String>> = Vec::new();
    let w = if img_w > 0 { img_w as f64 } else { 1.0 };
    let h = if img_h > 0 { img_h as f64 } else { 1.0 };

    for record in &data_output.data {
        // Only include records that have text (word_num > 0 means it's a word)
        if record.word_num > 0 && !record.text.is_empty() {
            let mut word_data = HashMap::new();
            word_data.insert("text".to_string(), record.text.clone());
            word_data.insert("conf".to_string(), format!("{:.2}", record.conf));
            // Normalize bounding box coordinates to 0-1 range (matching Apple Vision output)
            word_data.insert("left".to_string(), (record.left as f64 / w).to_string());
            word_data.insert("top".to_string(), (record.top as f64 / h).to_string());
            word_data.insert("width".to_string(), (record.width as f64 / w).to_string());
            word_data.insert("height".to_string(), (record.height as f64 / h).to_string());
            // Include position metadata
            word_data.insert("level".to_string(), record.level.to_string());
            word_data.insert("page_num".to_string(), record.page_num.to_string());
            word_data.insert("block_num".to_string(), record.block_num.to_string());
            word_data.insert("par_num".to_string(), record.par_num.to_string());
            word_data.insert("line_num".to_string(), record.line_num.to_string());
            word_data.insert("word_num".to_string(), record.word_num.to_string());
            words.push(word_data);
        }
    }

    serde_json::to_string(&words).unwrap_or_else(|_| "[]".to_string())
}

fn calculate_overall_confidence(data_output: &DataOutput) -> f64 {
    let total_conf: f32 = data_output.data.iter().map(|record| record.conf).sum();
    let count = data_output.data.len();
    if count > 0 {
        (total_conf / count as f32) as f64
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"x").unwrap();
    }

    #[test]
    fn bundled_binary_beside_the_engine_is_found() {
        let tmp = tempfile::tempdir().unwrap();
        let exe_dir = tmp.path().join("app");
        let bundled = exe_dir.join("tesseract").join(TESSERACT_EXE);
        touch(&bundled);
        let candidates = extra_binary_candidates(Some(&exe_dir), None, &[]);
        assert_eq!(first_file(&candidates), Some(bundled));
    }

    #[test]
    fn path_folders_are_searched_first() {
        let tmp = tempfile::tempdir().unwrap();
        let on_path = tmp.path().join("path-bin");
        let cwd = tmp.path().join("cwd");
        let exe_dir = tmp.path().join("app");
        let home = tmp.path().join("home");
        let path_var = std::env::join_paths([&on_path]).unwrap();
        let candidates =
            standard_binary_candidates(Some(&path_var), Some(&cwd), Some(&exe_dir), Some(&home));
        assert_eq!(candidates[0], on_path.join(TESSERACT_EXE));
        assert_eq!(candidates[1], cwd.join(TESSERACT_EXE));
        assert_eq!(candidates[2], exe_dir.join(TESSERACT_EXE));
        assert_eq!(
            candidates.last().unwrap(),
            &home.join(".local").join("bin").join(TESSERACT_EXE)
        );

        // The first existing file wins.
        let in_home = home.join(".local").join("bin").join(TESSERACT_EXE);
        touch(&in_home);
        assert_eq!(first_file(&candidates), Some(in_home));
        touch(&on_path.join(TESSERACT_EXE));
        assert_eq!(first_file(&candidates), Some(on_path.join(TESSERACT_EXE)));
    }

    #[test]
    fn empty_path_entries_are_ignored() {
        let candidates =
            standard_binary_candidates(Some(std::ffi::OsStr::new("")), None, None, None);
        assert!(candidates.is_empty());
    }

    #[test]
    fn appimage_binary_is_found_under_appdir() {
        let tmp = tempfile::tempdir().unwrap();
        let appdir = tmp.path().join("squashfs-root");
        let bin = appdir.join("usr").join("bin").join(TESSERACT_EXE);
        touch(&bin);
        let candidates = extra_binary_candidates(None, Some(&appdir), &[]);
        assert_eq!(first_file(&candidates), Some(bin));
    }

    #[test]
    fn installer_folders_come_after_the_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let exe_dir = tmp.path().join("app");
        let install = tmp.path().join("Tesseract-OCR-install");
        let bundled = exe_dir.join("tesseract").join(TESSERACT_EXE);
        let installed = install.join(TESSERACT_EXE);
        touch(&installed);
        let candidates = extra_binary_candidates(Some(&exe_dir), None, &[install.clone()]);
        assert_eq!(first_file(&candidates), Some(installed));
        touch(&bundled);
        assert_eq!(first_file(&candidates), Some(bundled));
    }

    #[test]
    fn nothing_found_when_no_candidate_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let candidates = extra_binary_candidates(
            Some(tmp.path()),
            Some(tmp.path()),
            &[tmp.path().join("nowhere")],
        );
        assert_eq!(first_file(&candidates), None);
    }

    #[test]
    fn tessdata_next_to_the_binary_wins() {
        let tmp = tempfile::tempdir().unwrap();
        let binary = tmp.path().join("bundle").join(TESSERACT_EXE);
        let near = tmp.path().join("bundle").join("tessdata");
        let appdir = tmp.path().join("appdir");
        let in_appdir = appdir
            .join("usr")
            .join("share")
            .join("tesseract-ocr")
            .join("5")
            .join("tessdata");
        touch(&in_appdir.join(ENG_TRAINEDDATA));
        let candidates = tessdata_candidates(&binary, None, Some(&appdir));
        assert_eq!(first_tessdata_with_eng(&candidates), Some(in_appdir));
        touch(&near.join(ENG_TRAINEDDATA));
        assert_eq!(first_tessdata_with_eng(&candidates), Some(near));
    }

    #[test]
    fn a_folder_without_english_data_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let binary = tmp.path().join("bin").join(TESSERACT_EXE);
        touch(
            &tmp.path()
                .join("bin")
                .join("tessdata")
                .join("deu.traineddata"),
        );
        let candidates = vec![tmp.path().join("bin").join("tessdata")];
        assert_eq!(first_tessdata_with_eng(&candidates), None);
        assert!(!tessdata_candidates(&binary, None, None).is_empty());
    }

    #[test]
    fn tsv_rows_parse_and_bad_rows_are_skipped() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
                   1\t1\t0\t0\t0\t0\t0\t0\t696\t89\t-1\t\n\
                   5\t1\t1\t1\t1\t1\t18\t29\t144\t35\t95.64\tLOREM\n\
                   not\ta\trow\n\
                   5\t1\t1\t1\t1\t2\t181\t29\t123\t35\t92.3\tIPSUM";
        let rows = parse_tsv(tsv);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].level, 1);
        assert_eq!(rows[0].conf, -1.0);
        assert!(rows[0].text.is_empty());
        assert_eq!(
            (
                rows[1].word_num,
                rows[1].left,
                rows[1].width,
                rows[1].text.as_str()
            ),
            (1, 18, 144, "LOREM")
        );
        assert_eq!(rows[2].text, "IPSUM");
        assert!(parse_tsv("").is_empty());
    }

    /// Runs a stand-in tesseract (a shell script) and checks that it gets the
    /// image, the language and `TESSDATA_PREFIX`, while this process's own
    /// environment stays untouched.
    #[cfg(unix)]
    #[test]
    fn the_found_binary_runs_with_tessdata_set_for_it_alone() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let tessdata = tmp.path().join("tessdata-marker");
        fs::create_dir_all(&tessdata).unwrap();
        let fake = tmp.path().join("fake-tesseract");
        fs::write(
            &fake,
            "#!/bin/sh\n\
             printf 'level\\tpage_num\\tblock_num\\tpar_num\\tline_num\\tword_num\\tleft\\ttop\\twidth\\theight\\tconf\\ttext\\n'\n\
             printf '5\\t1\\t1\\t1\\t1\\t1\\t1\\t2\\t3\\t4\\t90\\t%s\\n' \"$(basename \"$TESSDATA_PREFIX\")\"\n\
             printf '5\\t1\\t1\\t1\\t1\\t2\\t1\\t2\\t3\\t4\\t80\\t%s\\n' \"$4\"\n\
             test -f \"$1\" && printf '5\\t1\\t1\\t1\\t1\\t3\\t1\\t2\\t3\\t4\\t70\\timage-found\\n'\n\
             exit 0\n",
        )
        .unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let image = tmp.path().join("frame.png");
        fs::write(&image, b"png").unwrap();
        let install = TesseractInstall {
            binary: fake,
            tessdata: Some(tessdata.clone()),
        };
        let args = Args {
            lang: "eng+deu".into(),
            ..Args::default()
        };

        let out = run_with_retry(|| run_tesseract_tsv(&install, &image, &args)).unwrap();
        let words: Vec<&str> = out.data.iter().map(|d| d.text.as_str()).collect();
        assert_eq!(words, vec!["tessdata-marker", "eng+deu", "image-found"]);
        assert_ne!(std::env::var_os("TESSDATA_PREFIX"), Some(tessdata.into()));
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_binary_is_an_error_not_a_panic() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let fake = tmp.path().join("broken-tesseract");
        fs::write(&fake, "#!/bin/sh\necho 'no eng data' >&2\nexit 1\n").unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let install = TesseractInstall {
            binary: fake,
            tessdata: None,
        };
        let err = run_with_retry(|| {
            run_tesseract_tsv(&install, &tmp.path().join("x.png"), &Args::default())
        })
        .unwrap_err();
        assert!(err.contains("no eng data"), "{}", err);

        let gone = TesseractInstall {
            binary: tmp.path().join("removed-after-discovery"),
            tessdata: None,
        };
        let err =
            run_tesseract_tsv(&gone, &tmp.path().join("x.png"), &Args::default()).unwrap_err();
        assert!(err.contains("could not run"), "{}", err);
    }

    /// Retries a run that hit "text file busy": a script written by a test
    /// can briefly stay open in a child forked by a parallel test.
    #[cfg(unix)]
    fn run_with_retry(run: impl Fn() -> Result<DataOutput, String>) -> Result<DataOutput, String> {
        let mut result = run();
        for _ in 0..20 {
            match &result {
                Err(e) if e.contains("busy") => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    result = run();
                }
                _ => break,
            }
        }
        result
    }

    #[test]
    fn zero_sized_images_return_empty_without_running() {
        let empty = DynamicImage::ImageRgb8(image::RgbImage::new(0, 0));
        let (text, json, conf) = perform_ocr_tesseract(&empty, vec![]);
        assert!(text.is_empty());
        assert_eq!(json, "[]");
        assert_eq!(conf, None);
    }
}
