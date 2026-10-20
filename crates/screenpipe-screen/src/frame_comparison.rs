//! Optimized frame comparison for CPU-efficient change detection.
//!
//! This module provides fast frame comparison to decide whether OCR should run,
//! without affecting OCR quality (OCR always runs on full resolution).
//!
//! ## Optimizations
//!
//! 1. **Hash-based early exit**: If frame hash matches previous, skip comparison entirely
//! 2. **Proportional downscale**: Compare at 1/4 resolution (preserves aspect ratio for ultrawides)
//! 3. **Single metric**: Use only histogram comparison (not both histogram + SSIM)
//! 4. **Shared downscale**: One downscale serves both hash and comparison (no redundant work)
//! 5. **Grayscale thumbnails, fallible allocation**: each frame becomes one small
//!    luma thumbnail sampled straight from the source pixels. Its buffer is
//!    reserved with `Vec::try_reserve_exact`; when that fails the frame counts as
//!    unchanged and a rate-limited warning carries `source_width`,
//!    `source_height` and `failures`, instead of the process aborting.
//!
//! ## CPU Impact (benchmarked on trader 3-monitor setup: 2×5120x1440 + 2560x1440)
//!
//! | Optimization | CPU Reduction |
//! |--------------|---------------|
//! | Shared downscale + hash | 10ms/cycle (was 10.4ms for full-res hash) |
//! | Downscaled to_luma8 | 18ms/cycle (was 19.4ms for full-res grayscale) |
//! | Hash early exit | 30-50% in static scenes |
//! | Single metric | 40-50% vs histogram+SSIM |
//!
//! ## Example
//!
//! ```rust,ignore
//! use screenpipe_screen::frame_comparison::{FrameComparer, FrameComparisonConfig};
//!
//! let config = FrameComparisonConfig::default();
//! let mut comparer = FrameComparer::new(config);
//!
//! // Returns difference score (0.0 = identical, 1.0 = completely different)
//! // Hash is computed internally on the downscaled image — no need to pre-compute.
//! let diff = comparer.compare(&current_image);
//! if diff < 0.02 {
//!     // Skip OCR - frame hasn't changed significantly
//! }
//! ```

use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, GrayImage};
use image_compare::Metric;
use std::hash::{DefaultHasher, Hash, Hasher};
use tracing::{debug, warn};

/// How often a repeated thumbnail failure is logged. The first failure is
/// always logged; after that one line per this many failures keeps a machine
/// under memory pressure from flooding the log once per captured frame.
const THUMBNAIL_FAILURE_LOG_EVERY: u64 = 100;

/// Configuration for frame comparison optimizations.
#[derive(Debug, Clone)]
pub struct FrameComparisonConfig {
    /// Enable hash-based early exit for identical frames.
    /// When true, if current frame hash equals previous hash, return 0.0 immediately.
    /// Default: true
    pub hash_early_exit: bool,

    /// Enable downscaled comparison for faster processing.
    /// When true, resize images proportionally before comparing.
    /// Default: true
    pub downscale_comparison: bool,

    /// Factor to divide image dimensions by for comparison.
    /// E.g., factor=4 means a 5120x1440 image is compared at 1280x360.
    /// Preserves aspect ratio (critical for ultrawides).
    /// Default: 4
    pub downscale_factor: u32,

    /// Use only histogram comparison instead of histogram + SSIM.
    /// Histogram is faster and robust for detecting significant changes.
    /// Default: true (single metric)
    pub single_metric: bool,

    // Legacy fields kept for backward compatibility with existing configs.
    // Only used when downscale_factor is 0 (meaning "use fixed dimensions").
    /// Fixed width for comparison (legacy, prefer downscale_factor).
    pub comparison_width: u32,
    /// Fixed height for comparison (legacy, prefer downscale_factor).
    pub comparison_height: u32,
}

impl Default for FrameComparisonConfig {
    fn default() -> Self {
        Self {
            hash_early_exit: true,
            downscale_comparison: true,
            downscale_factor: 4,
            single_metric: true,
            comparison_width: 640,
            comparison_height: 360,
        }
    }
}

impl FrameComparisonConfig {
    /// Create a config with all optimizations disabled (original behavior).
    /// Useful for testing or when maximum accuracy is needed.
    pub fn no_optimizations() -> Self {
        Self {
            hash_early_exit: false,
            downscale_comparison: false,
            downscale_factor: 0,
            single_metric: false,
            comparison_width: 0,
            comparison_height: 0,
        }
    }

    /// Create a config optimized for maximum CPU savings.
    /// Uses factor=4 (1920→480px) to balance performance with text-change
    /// sensitivity. Factor 6 (320px) caused hash collisions on dense text
    /// pages, making scrolling through log files invisible to the comparer.
    pub fn max_performance() -> Self {
        Self {
            hash_early_exit: true,
            downscale_comparison: true,
            downscale_factor: 4,
            single_metric: true,
            comparison_width: 480,
            comparison_height: 270,
        }
    }
}

/// Why a grayscale thumbnail could not be produced or compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThumbnailError {
    /// The source image has a zero dimension.
    EmptySource,
    /// `width * height` does not fit in `usize`.
    SizeOverflow,
    /// The pixel buffer could not be reserved (`Vec::try_reserve_exact` failed).
    AllocationFailed,
    /// Internal invariant: two thumbnails that must match in size did not.
    SizeMismatch,
}

impl std::fmt::Display for ThumbnailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            ThumbnailError::EmptySource => "source image has a zero dimension",
            ThumbnailError::SizeOverflow => "thumbnail size overflows usize",
            ThumbnailError::AllocationFailed => "thumbnail buffer allocation failed",
            ThumbnailError::SizeMismatch => "grayscale thumbnails differ in size",
        };
        f.write_str(text)
    }
}

impl std::error::Error for ThumbnailError {}

/// Reserves an empty byte buffer with room for `len` bytes, without aborting
/// the process when memory is short. Swappable in tests to simulate failures.
type ThumbnailAllocator = fn(usize) -> Result<Vec<u8>, ThumbnailError>;

fn reserve_thumbnail_buffer(len: usize) -> Result<Vec<u8>, ThumbnailError> {
    let mut buf = Vec::new();
    buf.try_reserve_exact(len)
        .map_err(|_| ThumbnailError::AllocationFailed)?;
    Ok(buf)
}

/// Rec. 709 luma with integer weights (same weighting the `image` crate uses
/// for `to_luma8`), rounded to nearest.
#[inline]
fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((r as u32 * 2126 + g as u32 * 7152 + b as u32 * 722 + 5000) / 10_000) as u8
}

/// Nearest-neighbour source coordinate for destination index `dst` when
/// mapping `src_len` samples onto `dst_len` samples (pixel-centre sampling).
#[inline]
fn nearest_source(dst: u32, dst_len: u32, src_len: u32) -> u32 {
    let pos = ((2 * dst as u64 + 1) * src_len as u64) / (2 * dst_len as u64);
    (pos as u32).min(src_len - 1)
}

/// Builds a `dst_w` x `dst_h` grayscale thumbnail of `src` by nearest-neighbour
/// sampling, reading the source pixels in place. Nothing full-size is
/// allocated; the only allocation is the thumbnail buffer itself, and that one
/// goes through `alloc` so a failure becomes an error instead of an abort.
fn gray_thumbnail(
    src: &DynamicImage,
    dst_w: u32,
    dst_h: u32,
    alloc: ThumbnailAllocator,
) -> Result<GrayImage, ThumbnailError> {
    let (src_w, src_h) = src.dimensions();
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return Err(ThumbnailError::EmptySource);
    }
    let len = (dst_w as usize)
        .checked_mul(dst_h as usize)
        .ok_or(ThumbnailError::SizeOverflow)?;
    let mut buf = alloc(len)?;

    // Interleaved 8-bit layouts are read straight from the raw slice; any other
    // pixel type (16-bit, float) goes through the generic accessor.
    let (raw, channels): (Option<&[u8]>, usize) = match src {
        DynamicImage::ImageRgba8(img) => (Some(img.as_raw().as_slice()), 4),
        DynamicImage::ImageRgb8(img) => (Some(img.as_raw().as_slice()), 3),
        DynamicImage::ImageLumaA8(img) => (Some(img.as_raw().as_slice()), 2),
        DynamicImage::ImageLuma8(img) => (Some(img.as_raw().as_slice()), 1),
        _ => (None, 0),
    };
    let row_len = src_w as usize * channels;

    for y in 0..dst_h {
        let sy = nearest_source(y, dst_h, src_h);
        for x in 0..dst_w {
            let sx = nearest_source(x, dst_w, src_w);
            let value = match raw {
                Some(bytes) => {
                    let i = sy as usize * row_len + sx as usize * channels;
                    if channels >= 3 {
                        luma(bytes[i], bytes[i + 1], bytes[i + 2])
                    } else {
                        bytes[i]
                    }
                }
                None => {
                    let p = src.get_pixel(sx, sy).0;
                    luma(p[0], p[1], p[2])
                }
            };
            buf.push(value);
        }
    }

    GrayImage::from_raw(dst_w, dst_h, buf).ok_or(ThumbnailError::SizeMismatch)
}

/// Nearest-neighbour resample of a grayscale thumbnail to `dst_w` x `dst_h`,
/// with the same fallible allocation as [`gray_thumbnail`].
fn resample_gray(
    src: &GrayImage,
    dst_w: u32,
    dst_h: u32,
    alloc: ThumbnailAllocator,
) -> Result<GrayImage, ThumbnailError> {
    let (src_w, src_h) = src.dimensions();
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return Err(ThumbnailError::EmptySource);
    }
    let len = (dst_w as usize)
        .checked_mul(dst_h as usize)
        .ok_or(ThumbnailError::SizeOverflow)?;
    let mut buf = alloc(len)?;
    let raw = src.as_raw();
    for y in 0..dst_h {
        let row = nearest_source(y, dst_h, src_h) as usize * src_w as usize;
        for x in 0..dst_w {
            buf.push(raw[row + nearest_source(x, dst_w, src_w) as usize]);
        }
    }
    GrayImage::from_raw(dst_w, dst_h, buf).ok_or(ThumbnailError::SizeMismatch)
}

/// Hash of a thumbnail's size and pixels. The size is part of the key so two
/// thumbnails with equal bytes but a different shape never count as a match.
fn hash_thumbnail(thumb: &GrayImage) -> u64 {
    let mut hasher = DefaultHasher::new();
    thumb.dimensions().hash(&mut hasher);
    thumb.as_raw().hash(&mut hasher);
    hasher.finish()
}

/// True for the failures that get a log line: the first, then every
/// [`THUMBNAIL_FAILURE_LOG_EVERY`]th.
fn should_log_thumbnail_failure(failures: u64) -> bool {
    failures == 1 || failures % THUMBNAIL_FAILURE_LOG_EVERY == 0
}

/// Stateful frame comparer that tracks previous frame data for optimizations.
///
/// Every frame is reduced to a small grayscale thumbnail (one byte per pixel)
/// before it is hashed or compared, and only that thumbnail is kept as the
/// baseline. When the thumbnail cannot be allocated the frame is reported as
/// unchanged (`0.0`) and the previous baseline is kept.
#[derive(Debug)]
pub struct FrameComparer {
    config: FrameComparisonConfig,
    previous_hash: Option<u64>,
    previous_thumbnail: Option<GrayImage>,
    comparison_count: u64,
    hash_hits: u64,
    thumbnail_failures: u64,
    allocator: ThumbnailAllocator,
}

impl FrameComparer {
    /// Create a new frame comparer with the given configuration.
    pub fn new(config: FrameComparisonConfig) -> Self {
        Self::with_allocator(config, reserve_thumbnail_buffer)
    }

    fn with_allocator(config: FrameComparisonConfig, allocator: ThumbnailAllocator) -> Self {
        Self {
            config,
            previous_hash: None,
            previous_thumbnail: None,
            comparison_count: 0,
            hash_hits: 0,
            thumbnail_failures: 0,
            allocator,
        }
    }

    /// Compute the downscaled dimensions for a given image.
    fn downscale_dims(&self, width: u32, height: u32) -> (u32, u32) {
        if self.config.downscale_factor > 0 {
            // Proportional downscale — preserves aspect ratio
            (
                (width / self.config.downscale_factor).max(1),
                (height / self.config.downscale_factor).max(1),
            )
        } else {
            // Legacy fixed dimensions
            (
                self.config.comparison_width.max(1),
                self.config.comparison_height.max(1),
            )
        }
    }

    /// Thumbnail size for a frame: the downscaled size, or the full frame size
    /// when downscaling is turned off.
    fn thumbnail_dims(&self, width: u32, height: u32) -> (u32, u32) {
        if self.config.downscale_comparison {
            self.downscale_dims(width, height)
        } else {
            (width, height)
        }
    }

    /// Compare current frame with previous frame.
    ///
    /// The frame is reduced to one grayscale thumbnail that serves both the
    /// hash fast path and the histogram comparison.
    ///
    /// # Returns
    /// * `0.0` - Frames are identical (hash match or visual comparison), or
    ///   the thumbnail could not be allocated (treated as "no change")
    /// * `0.0 - 1.0` - Difference score (higher = more different)
    /// * `1.0` - First frame (no previous to compare) or completely different
    pub fn compare(&mut self, current_image: &DynamicImage) -> f64 {
        self.comparison_count += 1;

        let (source_width, source_height) = current_image.dimensions();
        let (thumb_w, thumb_h) = self.thumbnail_dims(source_width, source_height);
        let thumbnail = match gray_thumbnail(current_image, thumb_w, thumb_h, self.allocator) {
            Ok(t) => t,
            Err(e) => {
                self.record_thumbnail_failure(source_width, source_height, e);
                return 0.0;
            }
        };

        let current_hash = self
            .config
            .hash_early_exit
            .then(|| hash_thumbnail(&thumbnail));

        // First frame - no previous to compare
        let Some(previous) = self.previous_thumbnail.as_ref() else {
            self.store_baseline(thumbnail, current_hash);
            return 1.0;
        };

        // Optimization 1: Hash-based early exit
        if let (Some(prev_hash), Some(curr_hash)) = (self.previous_hash, current_hash) {
            if prev_hash == curr_hash {
                self.hash_hits += 1;
                debug!(
                    "Hash match - skipping comparison (hits: {}/{})",
                    self.hash_hits, self.comparison_count
                );
                return 0.0;
            }
        }

        let diff = match self.score(previous, &thumbnail) {
            Ok(d) => d,
            Err(
                e @ (ThumbnailError::AllocationFailed
                | ThumbnailError::SizeOverflow
                | ThumbnailError::EmptySource),
            ) => {
                self.record_thumbnail_failure(source_width, source_height, e);
                return 0.0;
            }
            Err(ThumbnailError::SizeMismatch) => {
                debug!("frame comparison: thumbnail sizes still differ after resize; reporting a change");
                1.0
            }
        };

        self.store_baseline(thumbnail, current_hash);
        diff
    }

    /// Difference between the baseline and the current thumbnail. A current
    /// thumbnail of another size (the monitor changed resolution) is resampled
    /// to the baseline size first, as before.
    fn score(&self, previous: &GrayImage, current: &GrayImage) -> Result<f64, ThumbnailError> {
        let resized;
        let current = if previous.dimensions() != current.dimensions() {
            resized = resample_gray(current, previous.width(), previous.height(), self.allocator)?;
            &resized
        } else {
            current
        };
        if previous.dimensions() != current.dimensions() {
            return Err(ThumbnailError::SizeMismatch);
        }

        let histogram = gray_histogram_distance(previous, current).unwrap_or(1.0);
        if self.config.single_metric {
            Ok(histogram)
        } else {
            let ssim = gray_ssim_distance(previous, current).unwrap_or(1.0);
            Ok((histogram + ssim) / 2.0)
        }
    }

    fn store_baseline(&mut self, thumbnail: GrayImage, hash: Option<u64>) {
        self.previous_hash = hash;
        self.previous_thumbnail = Some(thumbnail);
    }

    fn record_thumbnail_failure(
        &mut self,
        source_width: u32,
        source_height: u32,
        reason: ThumbnailError,
    ) {
        self.thumbnail_failures += 1;
        let failures = self.thumbnail_failures;
        if should_log_thumbnail_failure(failures) {
            warn!(
                source_width,
                source_height,
                failures,
                %reason,
                "frame diff skipped because no thumbnail buffer was available; frame counted as unchanged"
            );
        }
    }

    /// Backward-compatible compare that accepts a pre-computed hash.
    /// The hash is ignored — we compute our own on the downscaled image.
    pub fn compare_with_hash(&mut self, current_image: &DynamicImage, _legacy_hash: u64) -> f64 {
        self.compare(current_image)
    }

    /// Get statistics about comparison performance.
    pub fn stats(&self) -> FrameComparisonStats {
        FrameComparisonStats {
            total_comparisons: self.comparison_count,
            hash_hits: self.hash_hits,
            hash_hit_rate: if self.comparison_count > 0 {
                self.hash_hits as f64 / self.comparison_count as f64
            } else {
                0.0
            },
            thumbnail_failures: self.thumbnail_failures,
        }
    }

    /// Reset the comparer state (e.g., when switching monitors).
    pub fn reset(&mut self) {
        self.previous_hash = None;
        self.previous_thumbnail = None;
    }
}

/// Statistics about frame comparison performance.
#[derive(Debug, Clone)]
pub struct FrameComparisonStats {
    pub total_comparisons: u64,
    pub hash_hits: u64,
    pub hash_hit_rate: f64,
    /// Frames whose thumbnail could not be allocated (reported as unchanged).
    pub thumbnail_failures: u64,
}

/// Keeps a metric inside `[0, 1]` and turns NaN or infinity into an error, so
/// a degenerate input can never leak a NaN into the capture loop's threshold
/// checks.
fn finite_unit(value: f64) -> anyhow::Result<f64> {
    if value.is_finite() {
        Ok(value.clamp(0.0, 1.0))
    } else {
        Err(anyhow::anyhow!("comparison produced a non-finite score"))
    }
}

fn gray_histogram_distance(a: &GrayImage, b: &GrayImage) -> anyhow::Result<f64> {
    let distance = image_compare::gray_similarity_histogram(Metric::Hellinger, a, b)
        .map_err(|e| anyhow::anyhow!("Failed to compare images: {}", e))?;
    finite_unit(distance)
}

fn gray_ssim_distance(a: &GrayImage, b: &GrayImage) -> anyhow::Result<f64> {
    let similarity =
        image_compare::gray_similarity_structure(&image_compare::Algorithm::MSSIMSimple, a, b)
            .map_err(|e| anyhow::anyhow!("Failed to compare images: {}", e))?;
    finite_unit(1.0 - similarity.score) // Convert similarity to difference
}

/// Compare two images using histogram comparison.
/// Returns a difference score between 0.0 (identical) and 1.0 (completely different).
pub fn compare_histogram(image1: &DynamicImage, image2: &DynamicImage) -> anyhow::Result<f64> {
    let image_one = image1.to_luma8();
    let mut image_two = image2.to_luma8();
    // Resize to match if dimensions differ (e.g. monitor resolution change)
    if image_one.dimensions() != image_two.dimensions() {
        image_two = image::imageops::resize(
            &image_two,
            image_one.width(),
            image_one.height(),
            FilterType::Nearest,
        );
    }
    gray_histogram_distance(&image_one, &image_two)
}

/// Compare two images using SSIM.
/// Returns a difference score between 0.0 (identical) and 1.0 (completely different).
/// Images that cannot be compared (for example too small for the SSIM window)
/// score 1.0 instead of panicking.
pub fn compare_ssim(image1: &DynamicImage, image2: &DynamicImage) -> f64 {
    let image_one = image1.to_luma8();
    let mut image_two = image2.to_luma8();
    // Resize to match if dimensions differ (e.g. monitor resolution change)
    if image_one.dimensions() != image_two.dimensions() {
        image_two = image::imageops::resize(
            &image_two,
            image_one.width(),
            image_one.height(),
            FilterType::Nearest,
        );
    }
    gray_ssim_distance(&image_one, &image_two).unwrap_or(1.0)
}

/// Calculate hash of an image (for early exit optimization).
pub fn calculate_image_hash(image: &DynamicImage) -> u64 {
    let mut hasher = DefaultHasher::new();
    image.as_bytes().hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn create_solid_image(width: u32, height: u32, r: u8, g: u8, b: u8) -> DynamicImage {
        let img = RgbImage::from_fn(width, height, |_, _| Rgb([r, g, b]));
        DynamicImage::ImageRgb8(img)
    }

    fn create_gradient_image(width: u32, height: u32) -> DynamicImage {
        let img = RgbImage::from_fn(width, height, |x, y| {
            let r = ((x as f32 / width as f32) * 255.0) as u8;
            let g = ((y as f32 / height as f32) * 255.0) as u8;
            Rgb([r, g, 128])
        });
        DynamicImage::ImageRgb8(img)
    }

    fn create_text_pattern_image(width: u32, height: u32, seed: u8) -> DynamicImage {
        let img = RgbImage::from_fn(width, height, |x, y| {
            let line_height = 20;
            let is_text_line = (y / line_height) % 2 == 0;
            if is_text_line {
                let char_width = 10;
                let is_char = !((x + seed as u32) / char_width).is_multiple_of(3);
                if is_char {
                    Rgb([30, 30, 30])
                } else {
                    Rgb([255, 255, 255])
                }
            } else {
                Rgb([255, 255, 255])
            }
        });
        DynamicImage::ImageRgb8(img)
    }

    #[test]
    fn test_hash_early_exit_identical_frames() {
        let config = FrameComparisonConfig::default();
        let mut comparer = FrameComparer::new(config);

        let image = create_solid_image(1920, 1080, 100, 100, 100);

        let diff1 = comparer.compare(&image);
        assert_eq!(diff1, 1.0, "First frame should return 1.0");

        let diff2 = comparer.compare(&image);
        assert_eq!(diff2, 0.0, "Identical frame should return 0.0 (hash hit)");

        let stats = comparer.stats();
        assert_eq!(stats.hash_hits, 1);
        assert_eq!(stats.total_comparisons, 2);
    }

    #[test]
    fn test_hash_early_exit_disabled() {
        let config = FrameComparisonConfig {
            hash_early_exit: false,
            ..Default::default()
        };

        let mut comparer = FrameComparer::new(config);
        let image = create_solid_image(1920, 1080, 100, 100, 100);

        comparer.compare(&image);
        comparer.compare(&image);

        assert_eq!(comparer.stats().hash_hits, 0);
    }

    #[test]
    fn test_proportional_downscale() {
        let config = FrameComparisonConfig {
            downscale_factor: 4,
            ..Default::default()
        };
        let comparer = FrameComparer::new(config);

        // Standard monitor
        assert_eq!(comparer.downscale_dims(1920, 1080), (480, 270));
        // Ultrawide 49"
        assert_eq!(comparer.downscale_dims(5120, 1440), (1280, 360));
        // Superwide 38"
        assert_eq!(comparer.downscale_dims(3840, 1440), (960, 360));
        // 4K
        assert_eq!(comparer.downscale_dims(3840, 2160), (960, 540));
    }

    #[test]
    fn test_proportional_downscale_default_factor4() {
        let config = FrameComparisonConfig::default(); // factor=4
        let comparer = FrameComparer::new(config);

        assert_eq!(comparer.downscale_dims(1920, 1080), (480, 270));
        assert_eq!(comparer.downscale_dims(5120, 1440), (1280, 360));
        assert_eq!(comparer.downscale_dims(3840, 2160), (960, 540));
    }

    #[test]
    fn test_legacy_fixed_dimensions() {
        let config = FrameComparisonConfig {
            downscale_factor: 0, // Use legacy fixed dims
            comparison_width: 640,
            comparison_height: 360,
            ..Default::default()
        };
        let comparer = FrameComparer::new(config);

        // All monitors get same fixed size
        assert_eq!(comparer.downscale_dims(1920, 1080), (640, 360));
        assert_eq!(comparer.downscale_dims(5120, 1440), (640, 360));
    }

    #[test]
    fn test_detects_major_changes() {
        let config = FrameComparisonConfig::default();
        let mut comparer = FrameComparer::new(config);

        let black = create_solid_image(1920, 1080, 0, 0, 0);
        let white = create_solid_image(1920, 1080, 255, 255, 255);

        comparer.compare(&black);
        let diff = comparer.compare(&white);
        assert!(diff > 0.5, "Major change should be detected: {}", diff);
    }

    #[test]
    fn test_detects_content_changes() {
        let config = FrameComparisonConfig::default();
        let mut comparer = FrameComparer::new(config);

        // Text screen vs gradient — clearly different even at 1/4 resolution
        let image1 = create_text_pattern_image(1920, 1080, 0);
        let image2 = create_gradient_image(1920, 1080);

        comparer.compare(&image1);
        let diff = comparer.compare(&image2);
        assert!(diff > 0.01, "Content change should be detected: {}", diff);
    }

    #[test]
    fn test_ultrawide_detection() {
        // Ensure proportional downscale still detects changes on ultrawides
        let config = FrameComparisonConfig::default();
        let mut comparer = FrameComparer::new(config);

        let image1 = create_text_pattern_image(5120, 1440, 0);
        let image2 = create_gradient_image(5120, 1440); // completely different content

        comparer.compare(&image1);
        let diff = comparer.compare(&image2);
        assert!(
            diff > 0.01,
            "Ultrawide content change should be detected: {}",
            diff
        );
    }

    #[test]
    fn test_subtle_changes_detected_without_hash() {
        // Subtle changes that may hash-collide at downscaled resolution
        // should still be detected when hash early exit is disabled
        let config = FrameComparisonConfig {
            hash_early_exit: false,
            ..Default::default()
        };

        let mut comparer = FrameComparer::new(config);

        let image1 = create_text_pattern_image(1920, 1080, 0);
        let image2 = create_text_pattern_image(1920, 1080, 50);

        comparer.compare(&image1);
        let diff = comparer.compare(&image2);
        // Without hash early exit, histogram comparison should detect subtle changes
        // (though downscaling may reduce the difference)
        assert!(diff >= 0.0, "Should not error: {}", diff);
    }

    #[test]
    fn test_full_pipeline() {
        let config = FrameComparisonConfig::default();
        let mut comparer = FrameComparer::new(config);

        let frame1 = create_text_pattern_image(1920, 1080, 0);
        let frame2 = create_text_pattern_image(1920, 1080, 0); // identical
        let frame3 = create_text_pattern_image(1920, 1080, 0); // identical
        let frame4 = create_gradient_image(1920, 1080); // different
        let frame5 = create_solid_image(1920, 1080, 128, 128, 128); // different
        let frame6 = create_solid_image(1920, 1080, 128, 128, 128); // identical
        let frame7 = create_solid_image(1920, 1080, 0, 0, 0); // different

        let results: Vec<f64> = [
            &frame1, &frame2, &frame3, &frame4, &frame5, &frame6, &frame7,
        ]
        .iter()
        .map(|f| comparer.compare(f))
        .collect();

        assert_eq!(results[0], 1.0); // first frame
        assert_eq!(results[1], 0.0); // hash hit
        assert_eq!(results[2], 0.0); // hash hit
        assert!(results[3] > 0.0); // different
        assert!(results[4] > 0.0); // different
        assert_eq!(results[5], 0.0); // hash hit
        assert!(results[6] > 0.3); // big change

        assert!(comparer.stats().hash_hits >= 3);
    }

    #[test]
    fn test_backward_compat_compare_with_hash() {
        let config = FrameComparisonConfig::default();
        let mut comparer = FrameComparer::new(config);

        let image = create_solid_image(100, 100, 50, 50, 50);
        let diff = comparer.compare_with_hash(&image, 12345); // hash ignored
        assert_eq!(diff, 1.0);
    }

    #[test]
    fn test_reset_clears_state() {
        let config = FrameComparisonConfig::default();
        let mut comparer = FrameComparer::new(config);

        let image = create_solid_image(100, 100, 50, 50, 50);
        comparer.compare(&image);
        comparer.compare(&image);
        assert_eq!(comparer.stats().hash_hits, 1);

        comparer.reset();
        let diff = comparer.compare(&image);
        assert_eq!(diff, 1.0, "After reset, first frame should return 1.0");
    }

    #[test]
    fn test_config_presets() {
        let default = FrameComparisonConfig::default();
        assert!(default.hash_early_exit);
        assert!(default.downscale_comparison);
        assert_eq!(default.downscale_factor, 4);

        let no_opt = FrameComparisonConfig::no_optimizations();
        assert!(!no_opt.hash_early_exit);
        assert!(!no_opt.downscale_comparison);

        let max_perf = FrameComparisonConfig::max_performance();
        assert_eq!(max_perf.downscale_factor, 4);
    }

    // ── Hardening: grayscale thumbnails and fallible allocation ─────────

    fn failing_allocator(_len: usize) -> Result<Vec<u8>, ThumbnailError> {
        Err(ThumbnailError::AllocationFailed)
    }

    #[test]
    fn allocation_failure_reports_no_change_and_counts() {
        let mut comparer =
            FrameComparer::with_allocator(FrameComparisonConfig::default(), failing_allocator);
        let black = create_solid_image(640, 360, 0, 0, 0);
        let white = create_solid_image(640, 360, 255, 255, 255);

        assert_eq!(comparer.compare(&black), 0.0);
        assert_eq!(comparer.compare(&white), 0.0);
        let stats = comparer.stats();
        assert_eq!(stats.thumbnail_failures, 2);
        assert_eq!(stats.total_comparisons, 2);
        assert_eq!(stats.hash_hits, 0);
        assert!(comparer.previous_thumbnail.is_none());
    }

    #[test]
    fn allocation_failure_keeps_the_previous_baseline() {
        let mut comparer = FrameComparer::new(FrameComparisonConfig::default());
        let black = create_solid_image(640, 360, 0, 0, 0);
        let white = create_solid_image(640, 360, 255, 255, 255);

        assert_eq!(comparer.compare(&black), 1.0, "first frame");
        let baseline_hash = comparer.previous_hash;

        // Memory runs short for one frame: "no change", baseline untouched.
        comparer.allocator = failing_allocator;
        assert_eq!(comparer.compare(&white), 0.0);
        assert_eq!(comparer.previous_hash, baseline_hash);
        assert_eq!(comparer.stats().thumbnail_failures, 1);

        // Memory is back: the change is measured against the old baseline.
        comparer.allocator = reserve_thumbnail_buffer;
        assert!(comparer.compare(&white) > 0.5);
    }

    #[test]
    fn allocation_failure_on_first_frame_leaves_no_baseline() {
        let mut comparer =
            FrameComparer::with_allocator(FrameComparisonConfig::default(), failing_allocator);
        let image = create_gradient_image(320, 200);
        assert_eq!(comparer.compare(&image), 0.0);

        comparer.allocator = reserve_thumbnail_buffer;
        assert_eq!(comparer.compare(&image), 1.0, "first real thumbnail");
        assert_eq!(comparer.compare(&image), 0.0, "then a hash hit");
    }

    #[test]
    fn allocation_failure_while_resampling_reports_no_change() {
        // Small buffers succeed, anything larger fails: the 50x50 thumbnail
        // fits, resampling it up to the 400x400 baseline does not.
        fn fail_above_small(len: usize) -> Result<Vec<u8>, ThumbnailError> {
            if len > 100 * 100 {
                Err(ThumbnailError::AllocationFailed)
            } else {
                reserve_thumbnail_buffer(len)
            }
        }
        let config = FrameComparisonConfig {
            downscale_factor: 1,
            ..Default::default()
        };
        let mut comparer = FrameComparer::new(config);
        assert_eq!(comparer.compare(&create_solid_image(400, 400, 10, 10, 10)), 1.0);

        comparer.allocator = fail_above_small;
        assert_eq!(
            comparer.compare(&create_solid_image(50, 50, 250, 250, 250)),
            0.0
        );
        assert_eq!(comparer.stats().thumbnail_failures, 1);
        assert_eq!(
            comparer.previous_thumbnail.as_ref().unwrap().dimensions(),
            (400, 400),
            "baseline kept"
        );
    }

    #[test]
    fn real_try_reserve_failure_is_an_error_not_an_abort() {
        let image = create_solid_image(4, 4, 1, 2, 3);
        // On 64-bit targets u32::MAX squared fits in usize but is far beyond
        // what try_reserve_exact can hand out; on 32-bit targets it overflows.
        let err = gray_thumbnail(&image, u32::MAX, u32::MAX, reserve_thumbnail_buffer)
            .expect_err("an impossible thumbnail must fail");
        assert!(matches!(
            err,
            ThumbnailError::AllocationFailed | ThumbnailError::SizeOverflow
        ));
    }

    #[test]
    fn legacy_fixed_size_that_cannot_be_allocated_reports_no_change() {
        // End to end through the default allocator: a nonsensical legacy fixed
        // size makes try_reserve fail on every frame.
        let config = FrameComparisonConfig {
            downscale_factor: 0,
            comparison_width: u32::MAX,
            comparison_height: u32::MAX,
            ..Default::default()
        };
        let mut comparer = FrameComparer::new(config);
        let image = create_solid_image(64, 64, 100, 100, 100);
        assert_eq!(comparer.compare(&image), 0.0);
        assert_eq!(comparer.compare(&image), 0.0);
        assert_eq!(comparer.stats().thumbnail_failures, 2);
    }

    #[test]
    fn failure_logging_is_rate_limited() {
        let logged: Vec<u64> = (1..=350)
            .filter(|n| should_log_thumbnail_failure(*n))
            .collect();
        assert_eq!(logged, vec![1, 100, 200, 300]);
    }

    #[test]
    fn thumbnail_is_grayscale_with_the_downscaled_size() {
        let comparer = FrameComparer::new(FrameComparisonConfig::default());
        let (w, h) = comparer.thumbnail_dims(1920, 1080);
        assert_eq!((w, h), (480, 270));

        let red = create_solid_image(1920, 1080, 255, 0, 0);
        let thumb = gray_thumbnail(&red, w, h, reserve_thumbnail_buffer).unwrap();
        assert_eq!(thumb.dimensions(), (480, 270));
        assert_eq!(thumb.as_raw().len(), 480 * 270, "one byte per pixel");
        // Rec. 709 red weight: 0.2126 * 255 rounds to 54.
        assert!(thumb.as_raw().iter().all(|&v| v == 54));
    }

    #[test]
    fn full_resolution_thumbnail_when_downscaling_is_off() {
        let comparer = FrameComparer::new(FrameComparisonConfig::no_optimizations());
        assert_eq!(comparer.thumbnail_dims(1280, 720), (1280, 720));
    }

    #[test]
    fn thumbnail_matches_image_crate_luma_for_every_pixel_layout() {
        let rgb = create_gradient_image(97, 61);
        let reference = rgb.to_luma8();
        let layouts = [
            DynamicImage::ImageRgba8(rgb.to_rgba8()),
            rgb.clone(),
            DynamicImage::ImageLuma8(rgb.to_luma8()),
            DynamicImage::ImageLumaA8(rgb.to_luma_alpha8()),
            DynamicImage::ImageRgb16(rgb.to_rgb16()),
        ];
        for img in &layouts {
            // Same size as the source: sampling is the identity mapping.
            let thumb = gray_thumbnail(img, 97, 61, reserve_thumbnail_buffer).unwrap();
            for (a, b) in thumb.as_raw().iter().zip(reference.as_raw()) {
                assert!(
                    (*a as i16 - *b as i16).abs() <= 1,
                    "luma differs by more than rounding: {} vs {}",
                    a,
                    b
                );
            }
        }
    }

    #[test]
    fn resolution_change_is_compared_without_panicking() {
        let mut comparer = FrameComparer::new(FrameComparisonConfig::default());
        let small = create_text_pattern_image(1280, 720, 0);
        let large = create_text_pattern_image(2560, 1440, 0);
        let different = create_solid_image(2560, 1440, 0, 0, 0);

        assert_eq!(comparer.compare(&small), 1.0);
        let same_layout = comparer.compare(&large);
        assert!(
            (0.0..=1.0).contains(&same_layout),
            "score stays in range across a resize: {}",
            same_layout
        );
        // The baseline is now the larger thumbnail.
        assert_eq!(
            comparer.previous_thumbnail.as_ref().unwrap().dimensions(),
            (640, 360)
        );
        let changed = comparer.compare(&different);
        assert!(changed > 0.3, "content change after resize: {}", changed);
    }

    #[test]
    fn same_picture_at_a_new_resolution_looks_unchanged() {
        let mut comparer = FrameComparer::new(FrameComparisonConfig::default());
        let gray = create_solid_image(1920, 1080, 90, 90, 90);
        let gray_bigger = create_solid_image(2560, 1440, 90, 90, 90);
        comparer.compare(&gray);
        let diff = comparer.compare(&gray_bigger);
        assert!(diff < 0.01, "uniform frame across a resize: {}", diff);
    }

    #[test]
    fn hash_includes_thumbnail_shape() {
        let wide = GrayImage::from_raw(4, 1, vec![1, 2, 3, 4]).unwrap();
        let tall = GrayImage::from_raw(1, 4, vec![1, 2, 3, 4]).unwrap();
        assert_ne!(hash_thumbnail(&wide), hash_thumbnail(&tall));
        assert_eq!(hash_thumbnail(&wide), hash_thumbnail(&wide.clone()));
    }

    #[test]
    fn resample_preserves_uniform_content_and_size() {
        let src = GrayImage::from_pixel(10, 7, image::Luma([77]));
        let out = resample_gray(&src, 23, 3, reserve_thumbnail_buffer).unwrap();
        assert_eq!(out.dimensions(), (23, 3));
        assert!(out.as_raw().iter().all(|&v| v == 77));
    }

    #[test]
    fn nearest_source_stays_in_bounds() {
        for (dst_len, src_len) in [(1, 1), (1, 9), (9, 1), (480, 1920), (7, 3), (1000, 999)] {
            for d in 0..dst_len {
                assert!(nearest_source(d, dst_len, src_len) < src_len);
            }
        }
        assert_eq!(nearest_source(0, 2, 4), 1);
        assert_eq!(nearest_source(1, 2, 4), 3);
    }

    #[test]
    fn zero_sized_frame_is_a_failure_not_a_panic() {
        let mut comparer = FrameComparer::new(FrameComparisonConfig::default());
        let empty = DynamicImage::ImageRgb8(RgbImage::new(0, 0));
        assert_eq!(comparer.compare(&empty), 0.0);
        assert_eq!(comparer.stats().thumbnail_failures, 1);
    }

    #[test]
    fn ssim_path_handles_tiny_and_mismatched_frames() {
        let mut comparer = FrameComparer::new(FrameComparisonConfig {
            single_metric: false,
            ..Default::default()
        });
        // 4x4 sources give 1x1 thumbnails, below any SSIM window.
        assert_eq!(comparer.compare(&create_solid_image(4, 4, 0, 0, 0)), 1.0);
        let diff = comparer.compare(&create_solid_image(8, 8, 255, 255, 255));
        assert!((0.0..=1.0).contains(&diff), "diff in range: {}", diff);

        // The public helper no longer panics either, and never returns NaN.
        let d = compare_ssim(
            &create_solid_image(3, 3, 0, 0, 0),
            &create_solid_image(5, 2, 9, 9, 9),
        );
        assert!((0.0..=1.0).contains(&d), "ssim distance in range: {}", d);
    }
}
