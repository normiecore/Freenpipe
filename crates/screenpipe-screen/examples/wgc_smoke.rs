// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Windows Graphics Capture smoke test.
//!
//! For every monitor this opens a persistent WGC session, grabs a series of
//! frames, writes them as JPEGs and checks that they are not black. It also
//! lists the monitors through xcap and compares ids and stable ids with the
//! native enumeration, so a monitor keeps the identity it had in earlier
//! builds.
//!
//! ```text
//! cargo run -p screenpipe-screen --example wgc_smoke --profile release-dev -- \
//!     [--frames 10] [--interval-ms 300] [--out <dir>]
//! ```
//!
//! Exit code 0 when every monitor produced the requested number of non-black
//! frames and both enumerations agree, 1 otherwise. On other platforms the
//! example only prints a note and exits 0.

#[cfg(not(target_os = "windows"))]
fn main() {
    println!("wgc_smoke exercises Windows Graphics Capture and only runs on Windows.");
}

/// Fraction of sampled pixels that must be brighter than near-black for a
/// frame to count as real content.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const MIN_LIT_FRACTION: f64 = 0.005;

/// Fraction of a 64x64 sample grid whose brightest channel is above 8.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn lit_fraction(image: &image::RgbaImage) -> f64 {
    let (w, h) = image.dimensions();
    if w == 0 || h == 0 {
        return 0.0;
    }
    const GRID: u32 = 64;
    let mut lit = 0u32;
    for gy in 0..GRID {
        for gx in 0..GRID {
            let x = (gx * w / GRID).min(w - 1);
            let y = (gy * h / GRID).min(h - 1);
            let p = image.get_pixel(x, y).0;
            if p[0].max(p[1]).max(p[2]) > 8 {
                lit += 1;
            }
        }
    }
    lit as f64 / (GRID * GRID) as f64
}

#[cfg(target_os = "windows")]
fn main() -> std::process::ExitCode {
    windows_smoke::run()
}

#[cfg(target_os = "windows")]
mod windows_smoke {
    use super::{lit_fraction, MIN_LIT_FRACTION};
    use image::codecs::jpeg::JpegEncoder;
    use screenpipe_screen::monitor::{list_monitors_detailed, SafeMonitor};
    use screenpipe_screen::wgc_capture::PersistentCapture;
    use std::path::PathBuf;
    use std::process::ExitCode;
    use std::time::Duration;

    struct Options {
        frames: u32,
        interval: Duration,
        out: PathBuf,
    }

    fn parse_options() -> Result<Options, String> {
        let mut options = Options {
            frames: 10,
            interval: Duration::from_millis(300),
            out: std::env::temp_dir().join("freenpipe-wgc-smoke"),
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            let mut value = || args.next().ok_or_else(|| format!("{} needs a value", arg));
            match arg.as_str() {
                "--frames" => {
                    options.frames = value()?.parse().map_err(|e| format!("--frames: {}", e))?
                }
                "--interval-ms" => {
                    options.interval = Duration::from_millis(
                        value()?
                            .parse()
                            .map_err(|e| format!("--interval-ms: {}", e))?,
                    )
                }
                "--out" => options.out = PathBuf::from(value()?),
                "-h" | "--help" => {
                    return Err(
                        "usage: wgc_smoke [--frames N] [--interval-ms MS] [--out DIR]".into(),
                    )
                }
                other => return Err(format!("unknown argument {}", other)),
            }
        }
        if options.frames == 0 {
            return Err("--frames must be at least 1".into());
        }
        Ok(options)
    }

    /// Compare the native monitor list with the one xcap reports.
    fn compare_with_xcap(native: &[SafeMonitor]) -> bool {
        let xcap_monitors = match xcap::Monitor::all() {
            Ok(list) => list,
            Err(e) => {
                println!("xcap enumeration failed: {}", e);
                return false;
            }
        };
        let xcap: Vec<SafeMonitor> = xcap_monitors.into_iter().map(SafeMonitor::new).collect();
        let mut agree = native.len() == xcap.len();
        if !agree {
            println!(
                "monitor count differs: native {} vs xcap {}",
                native.len(),
                xcap.len()
            );
        }
        for monitor in native {
            match xcap.iter().find(|x| x.id() == monitor.id()) {
                Some(x) if x.stable_id() == monitor.stable_id() => println!(
                    "monitor {}: id and stable_id match xcap ({})",
                    monitor.id(),
                    monitor.stable_id()
                ),
                Some(x) => {
                    agree = false;
                    println!(
                        "monitor {}: stable_id differs: native {} vs xcap {}",
                        monitor.id(),
                        monitor.stable_id(),
                        x.stable_id()
                    );
                }
                None => {
                    agree = false;
                    println!("monitor {}: not reported by xcap", monitor.id());
                }
            }
        }
        agree
    }

    fn save_jpeg(image: &image::RgbaImage, path: &std::path::Path) -> Result<(), String> {
        let rgb = image::DynamicImage::ImageRgba8(image.clone()).to_rgb8();
        let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
        let mut writer = std::io::BufWriter::new(file);
        JpegEncoder::new_with_quality(&mut writer, 80)
            .encode_image(&rgb)
            .map_err(|e| e.to_string())
    }

    /// Capture `options.frames` frames from one monitor. Returns how many were
    /// written and not black.
    fn smoke_monitor(monitor: &SafeMonitor, options: &Options) -> u32 {
        let id = monitor.id();
        let capture = match PersistentCapture::new(id) {
            Ok(c) => c,
            Err(e) => {
                println!("monitor {}: could not start WGC capture: {:#}", id, e);
                return 0;
            }
        };
        let mut good = 0;
        for n in 0..options.frames {
            if n > 0 {
                std::thread::sleep(options.interval);
            }
            let timeout = if n == 0 {
                Duration::from_secs(2)
            } else {
                Duration::from_millis(500)
            };
            let image = match capture.get_latest_image(timeout) {
                Ok(img) => img.to_rgba8(),
                Err(e) => {
                    println!("monitor {} frame {}: capture failed: {:#}", id, n, e);
                    continue;
                }
            };
            let lit = lit_fraction(&image);
            let path = options
                .out
                .join(format!("monitor-{}-frame-{:02}.jpg", id, n));
            if let Err(e) = save_jpeg(&image, &path) {
                println!(
                    "monitor {} frame {}: writing {} failed: {}",
                    id,
                    n,
                    path.display(),
                    e
                );
                continue;
            }
            let ok = lit >= MIN_LIT_FRACTION;
            println!(
                "monitor {} frame {}: {}x{}, sequence {}, {:.1}% lit{} -> {}",
                id,
                n,
                image.width(),
                image.height(),
                capture.frame_sequence(),
                lit * 100.0,
                if ok { "" } else { " (BLACK)" },
                path.display()
            );
            if ok {
                good += 1;
            }
        }
        println!(
            "monitor {} ({}): {}/{} non-black frames on the {} device",
            id,
            monitor.stable_id(),
            good,
            options.frames,
            capture.device_kind()
        );
        good
    }

    pub fn run() -> ExitCode {
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
        tracing_subscriber::fmt().with_env_filter(filter).init();

        let options = match parse_options() {
            Ok(o) => o,
            Err(message) => {
                eprintln!("{}", message);
                return ExitCode::FAILURE;
            }
        };
        if let Err(e) = std::fs::create_dir_all(&options.out) {
            eprintln!("cannot create {}: {}", options.out.display(), e);
            return ExitCode::FAILURE;
        }

        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("tokio runtime: {}", e);
                return ExitCode::FAILURE;
            }
        };
        let monitors = match runtime.block_on(list_monitors_detailed()) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("monitor enumeration failed: {}", e);
                return ExitCode::FAILURE;
            }
        };
        println!(
            "{} monitor(s); writing frames to {}",
            monitors.len(),
            options.out.display()
        );

        let ids_agree = compare_with_xcap(&monitors);
        let mut all_good = ids_agree;
        for monitor in &monitors {
            if smoke_monitor(monitor, &options) < options.frames {
                all_good = false;
            }
        }

        if all_good {
            println!("PASS");
            ExitCode::SUCCESS
        } else {
            println!("FAIL");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_frames_are_not_lit() {
        let black = image::RgbaImage::from_pixel(320, 200, image::Rgba([0, 0, 0, 255]));
        assert_eq!(lit_fraction(&black), 0.0);
        let gray = image::RgbaImage::from_pixel(320, 200, image::Rgba([40, 40, 40, 255]));
        assert_eq!(lit_fraction(&gray), 1.0);
        assert!(lit_fraction(&gray) >= MIN_LIT_FRACTION);
    }
}
