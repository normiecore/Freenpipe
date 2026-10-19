use super::*;
use std::process::{Command, Stdio};
use std::string::ToString;

use crate::error::{TessError, TessResult};

use log::{debug, error};
use std::path::PathBuf;
use which::which;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[cfg(not(windows))]
const EXECUTABLE_NAME: &str = "tesseract";

#[cfg(windows)]
const EXECUTABLE_NAME: &str = "tesseract.exe";

pub fn find_tesseract_path() -> Option<PathBuf> {
    debug!("Starting search for tesseract executable");

    // Check if `tesseract` is in the PATH environment variable
    if let Ok(path) = which(EXECUTABLE_NAME) {
        debug!("Found tesseract in PATH: {:?}", path);
        return Some(path);
    }
    debug!("tesseract not found in PATH");

    // Check in current working directory
    if let Ok(cwd) = std::env::current_dir() {
        debug!("Current working directory: {:?}", cwd);
        let tesseract_in_cwd = cwd.join(EXECUTABLE_NAME);
        if tesseract_in_cwd.is_file() && tesseract_in_cwd.exists() {
            debug!(
                "Found tesseract in current working directory: {:?}",
                tesseract_in_cwd
            );
            return Some(tesseract_in_cwd);
        }
        debug!("tesseract not found in current working directory");
    }

    // Check in the same folder as the executable
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_folder) = exe_path.parent() {
            debug!("Executable folder: {:?}", exe_folder);
            let tesseract_in_exe_folder = exe_folder.join(EXECUTABLE_NAME);
            if tesseract_in_exe_folder.exists() {
                debug!(
                    "Found tesseract in executable folder: {:?}",
                    tesseract_in_exe_folder
                );
                return Some(tesseract_in_exe_folder);
            }
            debug!("tesseract not found in executable folder");

            // Platform-specific checks
            #[cfg(target_os = "macos")]
            {
                let resources_folder = exe_folder.join("../Resources");
                debug!("Resources folder: {:?}", resources_folder);
                let tesseract_in_resources = resources_folder.join(EXECUTABLE_NAME);
                if tesseract_in_resources.exists() {
                    debug!(
                        "Found tesseract in Resources folder: {:?}",
                        tesseract_in_resources
                    );
                    return Some(tesseract_in_resources);
                }
                debug!("tesseract not found in Resources folder");
            }

            #[cfg(target_os = "linux")]
            {
                let lib_folder = exe_folder.join("lib");
                debug!("Lib folder: {:?}", lib_folder);
                let tesseract_in_lib = lib_folder.join(EXECUTABLE_NAME);
                if tesseract_in_lib.exists() {
                    debug!("Found tesseract in lib folder: {:?}", tesseract_in_lib);
                    return Some(tesseract_in_lib);
                }
                debug!("tesseract not found in lib folder");
            }
        }
    }

    // Check in $HOME/.local/bin
    if let Some(home) = dirs::home_dir() {
        let tesseract_in_home = PathBuf::from(home).join(".local/bin").join(EXECUTABLE_NAME);
        if tesseract_in_home.exists() {
            debug!(
                "Found tesseract in $HOME/.local/bin: {:?}",
                tesseract_in_home
            );
            return Some(tesseract_in_home);
        }
    }

    error!("tesseract not found");
    None // Return None if tesseract is not found
}

pub(crate) fn get_tesseract_command() -> Command {
    let tesseract = find_tesseract_path().unwrap();

    Command::new(tesseract)
}

pub fn get_tesseract_version() -> TessResult<String> {
    let mut command = get_tesseract_command();
    command.arg("--version");

    run_tesseract_command(&mut command)
}

pub fn get_tesseract_langs() -> TessResult<Vec<String>> {
    let mut command = get_tesseract_command();
    command.arg("--list-langs");

    let output = run_tesseract_command(&mut command)?;
    let langs = output.lines().skip(1).map(|x| x.into()).collect();
    Ok(langs)
}

pub(crate) fn run_tesseract_command(command: &mut Command) -> TessResult<String> {
    if cfg!(debug_assertions) {
        show_command(command);
    }

    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);

    let child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| TessError::TesseractNotFoundError)?;

    let output = child
        .wait_with_output()
        .map_err(|_| TessError::TesseractNotFoundError)?;

    let out = String::from_utf8(output.stdout).unwrap();
    let err = String::from_utf8(output.stderr).unwrap();
    let status = output.status;

    match status.code() {
        Some(0) => Ok(out),
        _ => Err(TessError::CommandExitStatusError(status.to_string(), err)),
    }
}

fn show_command(command: &Command) {
    let params: Vec<String> = command
        .get_args()
        .map(|x| x.to_str().unwrap_or(""))
        .map(|s| s.to_string())
        .collect();

    println!(
        "Tesseract Command: {} {}",
        command.get_program().to_str().unwrap(),
        params.join(" ")
    );
}

pub fn image_to_string(image: &Image, args: &Args) -> TessResult<String> {
    let mut command = create_tesseract_command(image, args)?;
    let output = run_tesseract_command(&mut command)?;

    Ok(output)
}

pub(crate) fn create_tesseract_command(image: &Image, args: &Args) -> TessResult<Command> {
    let mut command = get_tesseract_command();
    command
        .arg(image.get_image_path()?)
        .arg("stdout")
        .arg("-l")
        .arg(args.lang.clone());

    if let Some(dpi) = args.dpi {
        command.arg("--dpi").arg(dpi.to_string());
    }

    if let Some(psm) = args.psm {
        command.arg("--psm").arg(psm.to_string());
    }

    if let Some(oem) = args.oem {
        command.arg("--oem").arg(oem.to_string());
    }

    for parameter in args.get_config_variable_args() {
        command.arg("-c").arg(parameter);
    }

    Ok(command)
}

#[cfg(test)]
mod tests {
    use crate::*;

    #[test]
    fn test_get_tesseract_langs() {
        let langs = get_tesseract_langs().unwrap();

        assert!(langs.contains(&"eng".into()));
    }
}
