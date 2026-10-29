// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

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
//!     [--frames 10] [--interval-ms 300] [--out <dir>] [--warp-first]
//! ```
//!
//! Exit code 0 when every monitor produced the requested number of non-black
//! frames and both enumerations agree, 1 otherwise. On other platforms the
//! example only prints a note and exits 0.
//!
//! `--warp-first` runs a different check instead: hardware D3D11 device
//! creation is refused for the first few seconds (as during a GPU driver
//! install), so every session starts on WARP. Each monitor is then read the
//! way the engine reads it (a closed session is recreated inside the same
//! read) until it runs on the hardware device again, which the one-minute
//! retry allows about 60 s in. It passes when every monitor started on WARP,
//! is back on hardware within 90 s and missed at most one read.

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
    use screenpipe_screen::wgc_capture::{
        refuse_hardware_device_for, PersistentCapture, WARP_HARDWARE_RETRY,
    };
    use std::path::PathBuf;
    use std::process::ExitCode;
    use std::time::{Duration, Instant};

    struct Options {
        frames: u32,
        interval: Duration,
        out: PathBuf,
        warp_first: bool,
    }

    fn parse_options() -> Result<Options, String> {
        let mut options = Options {
            frames: 10,
            interval: Duration::from_millis(300),
            out: std::env::temp_dir().join("handvel-wgc-smoke"),
            warp_first: false,
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
                "--warp-first" => options.warp_first = true,
                "-h" | "--help" => return Err(
                    "usage: wgc_smoke [--frames N] [--interval-ms MS] [--out DIR] [--warp-first]"
                        .into(),
                ),
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

    /// How long `--warp-first` refuses hardware devices. Short, so the first
    /// rate-limited retry (one minute after the WARP device appeared) finds
    /// the hardware device available.
    const WARP_HOLD: Duration = Duration::from_secs(5);

    /// How long `--warp-first` waits for every monitor to reach hardware.
    const WARP_DEADLINE: Duration = Duration::from_secs(90);

    /// One monitor under `--warp-first`.
    struct WarpWatch {
        id: u32,
        capture: Option<PersistentCapture>,
        started_on: Option<&'static str>,
        on_hardware_after: Option<Duration>,
        missed: u32,
        restarts: u32,
    }

    impl WarpWatch {
        /// One read the way `SafeMonitor::capture_image` does it on Windows:
        /// a failing session is replaced and the new one's first frame is
        /// returned from the same read.
        fn read(&mut self) -> Result<(), String> {
            if let Some(capture) = self.capture.as_ref() {
                match capture.get_latest_image(Duration::from_millis(200)) {
                    Ok(_) => return Ok(()),
                    Err(e) => println!("monitor {}: session ended: {:#}", self.id, e),
                }
            }
            if let Some(mut old) = self.capture.take() {
                old.stop();
                self.restarts += 1;
            }
            let capture = PersistentCapture::new(self.id).map_err(|e| format!("{:#}", e))?;
            match capture.get_latest_image(Duration::from_millis(500)) {
                Ok(_) => {
                    self.capture = Some(capture);
                    Ok(())
                }
                // Like the engine, a session without a first frame is dropped
                // and the next read starts another one.
                Err(e) => Err(format!("{:#}", e)),
            }
        }
    }

    /// The `--warp-first` check. See the module docs.
    fn warp_first(monitors: &[SafeMonitor], interval: Duration) -> bool {
        refuse_hardware_device_for(WARP_HOLD);
        let start = Instant::now();
        println!(
            "hardware D3D11 devices refused for {:?}; hardware is retried {:?} after the WARP device was created",
            WARP_HOLD, WARP_HARDWARE_RETRY
        );
        let mut watches: Vec<WarpWatch> = monitors
            .iter()
            .map(|m| WarpWatch {
                id: m.id(),
                capture: None,
                started_on: None,
                on_hardware_after: None,
                missed: 0,
                restarts: 0,
            })
            .collect();

        while start.elapsed() < WARP_DEADLINE {
            for watch in watches.iter_mut().filter(|w| w.on_hardware_after.is_none()) {
                if let Err(e) = watch.read() {
                    watch.missed += 1;
                    println!("monitor {}: read without a frame: {}", watch.id, e);
                }
                let kind = watch.capture.as_ref().map(|c| c.device_kind());
                if watch.started_on.is_none() {
                    watch.started_on = kind;
                    if let Some(kind) = kind {
                        println!("monitor {}: started on the {} device", watch.id, kind);
                    }
                }
                if kind == Some("hardware") {
                    let after = start.elapsed();
                    watch.on_hardware_after = Some(after);
                    println!(
                        "monitor {}: on the hardware device after {:.1} s ({} restart(s), {} read(s) without a frame)",
                        watch.id,
                        after.as_secs_f64(),
                        watch.restarts,
                        watch.missed
                    );
                }
            }
            if watches.iter().all(|w| w.on_hardware_after.is_some()) {
                break;
            }
            std::thread::sleep(interval);
        }

        let mut ok = !watches.is_empty();
        for watch in &watches {
            let passed = watch.started_on == Some("warp")
                && watch.on_hardware_after.is_some()
                && watch.missed <= 1;
            if !passed {
                ok = false;
                println!(
                    "monitor {}: FAILED (started on {}, on hardware: {}, reads without a frame: {})",
                    watch.id,
                    watch.started_on.unwrap_or("no device"),
                    watch
                        .on_hardware_after
                        .map(|d| format!("after {:.1} s", d.as_secs_f64()))
                        .unwrap_or_else(|| format!("not within {:?}", WARP_DEADLINE)),
                    watch.missed
                );
            }
        }
        ok
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
        if options.warp_first {
            println!(
                "{} monitor(s); checking the move from WARP back to hardware",
                monitors.len()
            );
        } else {
            println!(
                "{} monitor(s); writing frames to {}",
                monitors.len(),
                options.out.display()
            );
        }

        let all_good = if options.warp_first {
            warp_first(&monitors, options.interval)
        } else {
            let mut all_good = compare_with_xcap(&monitors);
            for monitor in &monitors {
                if smoke_monitor(monitor, &options) < options.frames {
                    all_good = false;
                }
            }
            all_good
        };

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
