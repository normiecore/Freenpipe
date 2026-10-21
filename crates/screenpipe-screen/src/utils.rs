use crate::capture_screenshot_by_window::{
    capture_all_visible_windows, CapturedWindow, WindowFilters,
};
use crate::custom_ocr::CustomOcrConfig;
use crate::monitor::SafeMonitor;
use image::DynamicImage;
use image_compare::{Algorithm, Metric, Similarity};
use screenpipe_db::CustomOcrConfig as DBCustomOcrConfig;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::{Duration, Instant};
use tracing::{debug, warn};

#[derive(Clone, Debug, Default)]
pub enum OcrEngine {
    Unstructured,
    #[default]
    Tesseract,
    WindowsNative,
    AppleNative,
    Custom(CustomOcrConfig),
}

impl From<OcrEngine> for screenpipe_db::OcrEngine {
    fn from(val: OcrEngine) -> Self {
        match val {
            OcrEngine::Unstructured => screenpipe_db::OcrEngine::Unstructured,
            OcrEngine::Tesseract => screenpipe_db::OcrEngine::Tesseract,
            OcrEngine::WindowsNative => screenpipe_db::OcrEngine::WindowsNative,
            OcrEngine::AppleNative => screenpipe_db::OcrEngine::AppleNative,
            OcrEngine::Custom(config) => {
                screenpipe_db::OcrEngine::Custom(DBCustomOcrConfig::from(config))
            }
        }
    }
}

impl From<screenpipe_db::OcrEngine> for OcrEngine {
    fn from(engine: screenpipe_db::OcrEngine) -> Self {
        match engine {
            screenpipe_db::OcrEngine::Unstructured => OcrEngine::Unstructured,
            screenpipe_db::OcrEngine::Tesseract => OcrEngine::Tesseract,
            screenpipe_db::OcrEngine::WindowsNative => OcrEngine::WindowsNative,
            screenpipe_db::OcrEngine::AppleNative => OcrEngine::AppleNative,
            screenpipe_db::OcrEngine::Custom(config) => OcrEngine::Custom(config.into()),
        }
    }
}

impl OcrEngine {
    pub fn platform_default() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::AppleNative
        }
        #[cfg(target_os = "windows")]
        {
            Self::WindowsNative
        }
        #[cfg(target_os = "linux")]
        {
            Self::Tesseract
        }
    }
}

impl std::str::FromStr for OcrEngine {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "tesseract" => Ok(Self::Tesseract),
            "windows-native" => Ok(Self::WindowsNative),
            "unstructured" => Ok(Self::Unstructured),
            "apple-native" => Ok(Self::AppleNative),
            _ => Ok(Self::platform_default()),
        }
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    #[test]
    fn ocr_engine_from_str_tesseract() {
        assert!(matches!(
            "tesseract".parse::<OcrEngine>().unwrap(),
            OcrEngine::Tesseract
        ));
    }

    #[test]
    fn ocr_engine_from_str_windows_native() {
        assert!(matches!(
            "windows-native".parse::<OcrEngine>().unwrap(),
            OcrEngine::WindowsNative
        ));
    }

    #[test]
    fn ocr_engine_from_str_unstructured() {
        assert!(matches!(
            "unstructured".parse::<OcrEngine>().unwrap(),
            OcrEngine::Unstructured
        ));
    }

    #[test]
    fn ocr_engine_from_str_apple_native() {
        assert!(matches!(
            "apple-native".parse::<OcrEngine>().unwrap(),
            OcrEngine::AppleNative
        ));
    }

    #[test]
    fn ocr_engine_from_str_unknown_returns_platform_default() {
        let engine = "something-else".parse::<OcrEngine>().unwrap();
        // platform_default varies by OS, just ensure it doesn't error
        let default = OcrEngine::platform_default();
        assert_eq!(
            std::mem::discriminant(&engine),
            std::mem::discriminant(&default)
        );
    }
}

pub fn calculate_hash(image: &DynamicImage) -> u64 {
    let mut hasher = DefaultHasher::new();
    image.as_bytes().hash(&mut hasher);
    hasher.finish()
}

pub fn compare_images_histogram(
    image1: &DynamicImage,
    image2: &DynamicImage,
) -> anyhow::Result<f64> {
    let image_one = image1.to_luma8();
    let mut image_two = image2.to_luma8();
    // Resize to match if dimensions differ (e.g. monitor resolution change)
    if image_one.dimensions() != image_two.dimensions() {
        image_two = image::imageops::resize(
            &image_two,
            image_one.width(),
            image_one.height(),
            image::imageops::FilterType::Nearest,
        );
    }
    image_compare::gray_similarity_histogram(Metric::Hellinger, &image_one, &image_two)
        .map_err(|e| anyhow::anyhow!("Failed to compare images: {}", e))
}

pub fn compare_images_ssim(image1: &DynamicImage, image2: &DynamicImage) -> f64 {
    let image_one = image1.to_luma8();
    let mut image_two = image2.to_luma8();
    // Resize to match if dimensions differ (e.g. monitor resolution change)
    if image_one.dimensions() != image_two.dimensions() {
        image_two = image::imageops::resize(
            &image_two,
            image_one.width(),
            image_one.height(),
            image::imageops::FilterType::Nearest,
        );
    }
    let result: Similarity =
        image_compare::gray_similarity_structure(&Algorithm::MSSIMSimple, &image_one, &image_two)
            .expect("images should have matching dimensions after resize");
    result.score
}

/// Capture only the monitor screenshot (no window capture, no hash).
/// Window capture is deferred until after frame comparison to avoid
/// expensive work on frames that will be skipped.
///
/// `excluded_window_ids` — SCK window IDs to exclude from the capture
/// (macOS only). The OS won't render those windows into the buffer.
/// Pass an empty slice to capture everything.
pub async fn capture_monitor_image(
    monitor: &SafeMonitor,
    #[allow(unused_variables)] excluded_window_ids: &[u32],
) -> Result<(DynamicImage, Duration), anyhow::Error> {
    let capture_start = Instant::now();

    #[cfg(target_os = "macos")]
    let image = if excluded_window_ids.is_empty() {
        monitor.capture_image().await
    } else {
        monitor.capture_image_excluding(excluded_window_ids).await
    }
    .map_err(|e| {
        debug!("failed to capture monitor image: {}", e);
        anyhow::anyhow!("monitor capture failed: {}", e)
    })?;

    #[cfg(not(target_os = "macos"))]
    let image = monitor.capture_image().await.map_err(|e| {
        debug!("failed to capture monitor image: {}", e);
        anyhow::anyhow!("monitor capture failed: {}", e)
    })?;

    let capture_duration = capture_start.elapsed();
    Ok((image, capture_duration))
}

/// Capture all visible windows on a monitor (called only when frame changed).
pub async fn capture_windows(
    monitor: &SafeMonitor,
    window_filters: &WindowFilters,
    capture_unfocused_windows: bool,
) -> Vec<CapturedWindow> {
    match capture_all_visible_windows(monitor, window_filters, capture_unfocused_windows).await {
        Ok(images) => images,
        Err(e) => {
            warn!(
                "Failed to capture window images: {}. Continuing with empty result.",
                e
            );
            Vec::new()
        }
    }
}

pub async fn capture_screenshot(
    monitor: &SafeMonitor,
    window_filters: &WindowFilters,
    capture_unfocused_windows: bool,
) -> Result<(DynamicImage, Vec<CapturedWindow>, u64, Duration), anyhow::Error> {
    let capture_start = Instant::now();
    let image = monitor.capture_image().await.map_err(|e| {
        debug!("failed to capture monitor image: {}", e);
        anyhow::anyhow!("monitor capture failed")
    })?;
    let image_hash = calculate_hash(&image);
    let capture_duration = capture_start.elapsed();

    let window_images =
        match capture_all_visible_windows(monitor, window_filters, capture_unfocused_windows).await
        {
            Ok(images) => images,
            Err(e) => {
                warn!(
                    "Failed to capture window images: {}. Continuing with empty result.",
                    e
                );
                Vec::new()
            }
        };

    Ok((image, window_images, image_hash, capture_duration))
}

// --- Frame validity ----------------------------------------------------------
//
// Capture backends sometimes hand back frames that never held real screen
// content: a GPU buffer that was never written (all zeros, so pure black), or
// a buffer whose tail was not filled, which converts from zeroed YUV to a band
// of flat green. Such frames must not be stored or used as the change
// detector's baseline. The checks sample a fixed grid, so they cost the same
// few thousand pixel reads at any resolution, and they are deliberately
// strict: a dark theme, a black terminal with a single prompt or a night scene
// in a video is real content and must survive.

/// Samples per axis of the validity grid (at most; small images sample every
/// pixel column or row).
pub const FRAME_VALIDITY_GRID: u32 = 64;

/// A sample counts as black only when no channel exceeds this value. Zeroed
/// buffers and windows the OS blanked out are exactly 0; real dark interfaces
/// (terminals, dark themes, video) carry at least some pixels above it.
const BLACK_CHANNEL_MAX: u8 = 2;

/// Decode-garbage green is what zero-filled YUV turns into: red and blue clamp
/// to 0, green lands between roughly 77 and 136 depending on the colour
/// matrix. The bounds keep a little slack for rounding.
const GARBAGE_GREEN_RB_MAX: u8 = 4;
const GARBAGE_GREEN_G_MIN: u8 = 64;
const GARBAGE_GREEN_G_MAX: u8 = 160;

/// A grid row belongs to a garbage band when at least this share of its
/// samples is garbage green (a cursor or one stray pixel may cross it).
const GARBAGE_ROW_MIN_SHARE: f64 = 0.97;

/// Classify a captured frame as corrupt, or `None` when it may hold real
/// screen content.
///
/// - [`CorruptFrameKind::Black`]: every pixel is black (no channel above 2),
///   or the image has no pixels at all. The grid only nominates candidates; a
///   candidate is confirmed by reading every pixel, so a single prompt line or
///   a clock on an otherwise black screen keeps the frame.
/// - [`CorruptFrameKind::Green`]: a full-width band of decode-garbage green
///   rows covering at least a quarter of the frame height anywhere, or at
///   least an eighth of it when the band touches the top or bottom edge (where
///   an unfilled buffer tail shows up). A whole green frame is the largest
///   such band.
pub fn detect_corrupt_frame(image: &DynamicImage) -> Option<crate::metrics::CorruptFrameKind> {
    use crate::metrics::CorruptFrameKind;
    use image::GenericImageView;

    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Some(CorruptFrameKind::Black);
    }
    let cols = FRAME_VALIDITY_GRID.min(width);
    let rows = FRAME_VALIDITY_GRID.min(height);
    // Sample the centre of each grid cell so the edges of the frame (borders,
    // a one-pixel line of garbage) do not decide the outcome on their own.
    let coord = |i: u32, cells: u32, extent: u32| -> u32 {
        let c = (2 * u64::from(i) + 1) * u64::from(extent) / (2 * u64::from(cells));
        (c as u32).min(extent - 1)
    };

    let min_garbage_per_row = (f64::from(cols) * GARBAGE_ROW_MIN_SHARE).ceil() as u32;
    let mut lit_samples = 0u32;
    let mut garbage_rows = Vec::with_capacity(rows as usize);
    for r in 0..rows {
        let y = coord(r, rows, height);
        let mut garbage = 0u32;
        for c in 0..cols {
            let x = coord(c, cols, width);
            let px = image.get_pixel(x, y);
            let (red, green, blue) = (px[0], px[1], px[2]);
            if red.max(green).max(blue) > BLACK_CHANNEL_MAX {
                lit_samples += 1;
            }
            if red <= GARBAGE_GREEN_RB_MAX
                && blue <= GARBAGE_GREEN_RB_MAX
                && (GARBAGE_GREEN_G_MIN..=GARBAGE_GREEN_G_MAX).contains(&green)
            {
                garbage += 1;
            }
        }
        garbage_rows.push(garbage >= min_garbage_per_row);
    }

    if lit_samples == 0 && every_pixel_black(image) {
        return Some(CorruptFrameKind::Black);
    }
    if has_garbage_band(&garbage_rows) {
        return Some(CorruptFrameKind::Green);
    }
    None
}

/// True when no pixel of `image` has a channel above [`BLACK_CHANNEL_MAX`].
/// Stops at the first lit pixel, so it only reads the whole frame when the
/// frame really is black.
fn every_pixel_black(image: &DynamicImage) -> bool {
    use image::GenericImageView;
    let dark = |r: u8, g: u8, b: u8| r.max(g).max(b) <= BLACK_CHANNEL_MAX;
    match image {
        DynamicImage::ImageRgba8(buf) => buf
            .as_raw()
            .chunks_exact(4)
            .all(|p| dark(p[0], p[1], p[2])),
        DynamicImage::ImageRgb8(buf) => buf
            .as_raw()
            .chunks_exact(3)
            .all(|p| dark(p[0], p[1], p[2])),
        other => other.pixels().all(|(_, _, p)| dark(p[0], p[1], p[2])),
    }
}

/// True when the per-row garbage flags contain a band large enough to call
/// the frame corrupt (see [`detect_corrupt_frame`]).
fn has_garbage_band(rows: &[bool]) -> bool {
    let n = rows.len();
    if n == 0 {
        return false;
    }
    let anywhere_min = (n / 4).max(1);
    let edge_min = (n / 8).max(1);
    let mut start = 0usize;
    while start < n {
        if !rows[start] {
            start += 1;
            continue;
        }
        let mut end = start;
        while end < n && rows[end] {
            end += 1;
        }
        let len = end - start;
        let touches_edge = start == 0 || end == n;
        if len >= anywhere_min || (touches_edge && len >= edge_min) {
            return true;
        }
        start = end;
    }
    false
}

#[cfg(test)]
mod frame_validity_tests {
    use super::*;
    use crate::metrics::CorruptFrameKind;
    use image::{Rgb, RgbImage, Rgba, RgbaImage};

    const W: u32 = 1920;
    const H: u32 = 1080;

    fn fill(color: [u8; 3]) -> RgbImage {
        RgbImage::from_pixel(W, H, Rgb(color))
    }

    fn rect(img: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, color: [u8; 3]) {
        for yy in y..(y + h).min(img.height()) {
            for xx in x..(x + w).min(img.width()) {
                img.put_pixel(xx, yy, Rgb(color));
            }
        }
    }

    /// Draws `chars` glyph-like cells (9x16, roughly 20% ink: one vertical
    /// stem and three thin bars, the way monospace text covers a cell) on a
    /// line starting at (x, y).
    fn text_line(img: &mut RgbImage, x: u32, y: u32, chars: u32, color: [u8; 3]) {
        for i in 0..chars {
            let cx = x + i * 9;
            rect(img, cx + 1, y + 3, 1, 10, color);
            rect(img, cx + 1, y + 3, 6, 1, color);
            rect(img, cx + 1, y + 8, 5, 1, color);
            rect(img, cx + 1, y + 12, 6, 1, color);
        }
    }

    fn check(img: RgbImage) -> Option<CorruptFrameKind> {
        detect_corrupt_frame(&DynamicImage::ImageRgb8(img))
    }

    #[test]
    fn zeroed_buffer_is_black() {
        assert_eq!(check(fill([0, 0, 0])), Some(CorruptFrameKind::Black));
        // WGC hands out BGRA converted to RGBA; alpha does not matter.
        let rgba = RgbaImage::from_pixel(W, H, Rgba([0, 0, 0, 0]));
        assert_eq!(
            detect_corrupt_frame(&DynamicImage::ImageRgba8(rgba)),
            Some(CorruptFrameKind::Black)
        );
    }

    #[test]
    fn an_image_without_pixels_is_black() {
        assert_eq!(check(RgbImage::new(0, 0)), Some(CorruptFrameKind::Black));
        assert_eq!(check(RgbImage::new(1920, 0)), Some(CorruptFrameKind::Black));
    }

    #[test]
    fn any_lit_pixel_keeps_a_frame_even_between_sample_points() {
        // (960, 540) lies between grid points, so only the confirming scan
        // sees it. A frame with any visible content is not a zeroed buffer.
        let mut img = fill([0, 0, 0]);
        img.put_pixel(960, 540, Rgb([255, 255, 255]));
        assert_eq!(check(img), None);
        // Values at or below the black cut-off are still black.
        let mut img = fill([0, 0, 0]);
        img.put_pixel(17, 33, Rgb([2, 1, 2]));
        assert_eq!(check(img), Some(CorruptFrameKind::Black));
    }

    #[test]
    fn ordinary_content_is_valid() {
        assert_eq!(check(fill([120, 130, 140])), None);
        let mut img = fill([0, 0, 0]);
        // A menu bar across the top is real content.
        rect(&mut img, 0, 0, W, 22, [200, 200, 200]);
        assert_eq!(check(img), None);
    }

    // --- Dark interfaces that must never be dropped --------------------------

    #[test]
    fn dark_themes_are_valid() {
        // VS Code / Windows dark mode greys, Windows Terminal's near-black.
        for bg in [[30, 30, 30], [32, 32, 32], [12, 12, 12], [24, 24, 27]] {
            assert_eq!(check(fill(bg)), None, "background {bg:?}");
        }
    }

    #[test]
    fn black_terminal_with_one_prompt_line_and_dark_taskbar_is_valid() {
        let mut img = fill([0, 0, 0]);
        text_line(&mut img, 8, 40, 18, [204, 204, 204]);
        // Windows 11 dark taskbar.
        rect(&mut img, 0, H - 48, W, 48, [32, 32, 32]);
        assert_eq!(check(img), None);
    }

    #[test]
    fn fullscreen_black_terminal_with_a_few_lines_is_valid() {
        let mut img = fill([0, 0, 0]);
        for line in 0..4 {
            text_line(&mut img, 8, 8 + line * 18, 40, [192, 192, 192]);
        }
        assert_eq!(check(img), None);
    }

    #[test]
    fn night_scene_in_a_video_is_valid() {
        // Very dark, but not a zeroed buffer: a dim gradient of 3..=14.
        let mut img = RgbImage::new(W, H);
        for (x, y, px) in img.enumerate_pixels_mut() {
            let v = 3 + ((x / 160 + y / 90) % 12) as u8;
            *px = Rgb([v, v, v + 2]);
        }
        assert_eq!(check(img), None);
    }

    #[test]
    fn oled_black_ui_with_sparse_white_text_is_valid() {
        let mut img = fill([0, 0, 0]);
        // A clock and a couple of labels on an otherwise pure black screen.
        text_line(&mut img, 900, 500, 5, [255, 255, 255]);
        text_line(&mut img, 40, 1000, 12, [180, 180, 180]);
        text_line(&mut img, 1600, 20, 8, [180, 180, 180]);
        assert_eq!(check(img), None);
    }

    // --- Green decode garbage --------------------------------------------------

    #[test]
    fn green_band_at_the_bottom_is_corrupt() {
        // Top two thirds hold a normal desktop, the unfilled tail is green.
        let mut img = fill([40, 44, 52]);
        text_line(&mut img, 20, 100, 60, [220, 220, 220]);
        rect(&mut img, 0, 720, W, 360, [0, 135, 0]);
        assert_eq!(check(img), Some(CorruptFrameKind::Green));
    }

    #[test]
    fn green_band_for_each_colour_matrix_is_corrupt() {
        // BT.601 and BT.709, limited and full range, of zeroed YUV.
        for g in [136u8, 135, 77, 84] {
            let mut img = fill([200, 200, 200]);
            rect(&mut img, 0, 810, W, 270, [0, g, 0]);
            assert_eq!(check(img), Some(CorruptFrameKind::Green), "green {g}");
        }
    }

    #[test]
    fn a_whole_green_frame_is_corrupt() {
        assert_eq!(check(fill([0, 136, 0])), Some(CorruptFrameKind::Green));
    }

    #[test]
    fn a_large_green_band_in_the_middle_is_corrupt() {
        let mut img = fill([250, 250, 250]);
        rect(&mut img, 0, 300, W, 300, [1, 134, 2]);
        assert_eq!(check(img), Some(CorruptFrameKind::Green));
    }

    #[test]
    fn green_interface_elements_are_valid() {
        // Windows "green" accent taskbar (150% scaling: 72 px).
        let mut img = fill([243, 243, 243]);
        rect(&mut img, 0, H - 72, W, 72, [16, 124, 16]);
        assert_eq!(check(img), None);

        // A pure green banner that does not span the full width.
        let mut img = fill([255, 255, 255]);
        rect(&mut img, 200, 0, W - 400, 200, [0, 128, 0]);
        assert_eq!(check(img), None);

        // A thin full-width pure green bar inside the page.
        let mut img = fill([255, 255, 255]);
        rect(&mut img, 0, 400, W, 60, [0, 128, 0]);
        assert_eq!(check(img), None);

        // Spreadsheet green, success banners, chroma-key green.
        for color in [[33, 115, 70], [40, 167, 69], [0, 177, 64], [30, 215, 96]] {
            assert_eq!(check(fill(color)), None, "{color:?}");
        }
    }

    #[test]
    fn a_cursor_over_the_band_does_not_hide_it() {
        let mut img = fill([90, 90, 90]);
        rect(&mut img, 0, 540, W, 540, [0, 135, 0]);
        rect(&mut img, 950, 700, 20, 30, [255, 255, 255]);
        assert_eq!(check(img), Some(CorruptFrameKind::Green));
    }

    #[test]
    fn tiny_images_are_classified_without_panicking() {
        assert_eq!(check(RgbImage::new(1, 1)), Some(CorruptFrameKind::Black));
        assert_eq!(
            check(RgbImage::from_pixel(3, 2, Rgb([0, 120, 0]))),
            Some(CorruptFrameKind::Green)
        );
        assert_eq!(check(RgbImage::from_pixel(5, 7, Rgb([9, 9, 9]))), None);
    }
}
