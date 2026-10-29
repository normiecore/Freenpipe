// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Thumbnail rendering and the in-memory cache behind
//! `GET /frames/:frame_id/thumbnail`.
//!
//! # What is cached
//!
//! Encoded JPEG thumbnails, keyed by [`ThumbnailKey`]: the frame id, the
//! requested width and quality, and a *source generation*, a hash of the
//! media file's identity (kind, path, frame position, size and modification
//! time). When snapshot compaction moves a frame from its JPEG into an MP4, or
//! a file is rewritten, the generation changes, so an old thumbnail can never
//! answer for new media. The key also drives the response `ETag`.
//!
//! # Bounds
//!
//! * Memory: entries are weighed by their encoded size and the cache holds at
//!   most [`MEMORY_BUDGET_BYTES`]; least-recently-used entries go first, and an
//!   entry idle for [`IDLE_TTL`] is dropped.
//! * Disk: nothing. Snapshot JPEGs are read into memory and video frames are
//!   piped out of ffmpeg ([`crate::video_utils::extract_frame_jpeg_bytes`]), so
//!   a thumbnail never creates a file. Thumbnails of deleted frames therefore
//!   cannot outlive the frame on disk, and a locked vault has no plaintext
//!   thumbnail files to leak.
//! * CPU: decoding and resizing run on the blocking pool, at most
//!   [`render_slots`] at a time, so a page full of thumbnails cannot spike
//!   memory with dozens of full-resolution decodes at once.
//!
//! # Deleted frames
//!
//! The cache is never asked first. The route looks the frame up in the
//! `frames` table on every request and only builds a key from the row it
//! finds, so a frame the user deleted gets a 404 even while its thumbnail is
//! still in memory.

use std::fmt;
use std::io::Cursor;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, GenericImageView, ImageReader, Limits};
use moka::future::Cache;
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

/// Width used when the request names none.
pub const DEFAULT_WIDTH: u32 = 320;
/// Smallest width a request may ask for.
pub const MIN_WIDTH: u32 = 32;
/// Largest width a request may ask for.
pub const MAX_WIDTH: u32 = 3840;
/// JPEG quality used when the request names none.
pub const DEFAULT_QUALITY: u8 = 75;
/// Lowest accepted JPEG quality.
pub const MIN_QUALITY: u8 = 1;
/// Highest accepted JPEG quality.
pub const MAX_QUALITY: u8 = 100;

/// Total encoded bytes the cache keeps in memory.
pub const MEMORY_BUDGET_BYTES: u64 = 32 * 1024 * 1024;
/// An entry not read for this long is dropped.
pub const IDLE_TTL: Duration = Duration::from_secs(30 * 60);

/// Largest source side the decoder accepts (protects against bogus headers).
const MAX_SOURCE_SIDE: u32 = 16_384;
/// Decoder allocation ceiling for one source image.
const MAX_DECODE_BYTES: u64 = 512 * 1024 * 1024;
/// Bookkeeping added to each entry's weight so empty-ish entries still count.
const ENTRY_OVERHEAD_BYTES: usize = 128;

/// How many thumbnails may be decoded and resized at the same time.
pub fn render_slots() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get() / 2)
        .unwrap_or(2)
        .clamp(2, 4)
}

/// Where a thumbnail's pixels come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceKind {
    /// A snapshot JPEG.
    Snapshot,
    /// A frame inside an MP4 video chunk.
    Video,
}

/// The identity of a media file at the moment a thumbnail is made from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceStamp<'a> {
    pub kind: SourceKind,
    pub path: &'a str,
    /// Frame position inside a video chunk (0 for snapshots).
    pub offset_index: i64,
    /// File size in bytes.
    pub len: u64,
    /// Last modification time, when the file system reports one.
    pub modified: Option<SystemTime>,
}

impl SourceStamp<'_> {
    /// A stable 64-bit hash of this identity. Stable across restarts, so an
    /// `ETag` a browser kept from an earlier run still validates.
    pub fn generation(&self) -> u64 {
        let mut hasher = Sha256::new();
        hasher.update(match self.kind {
            SourceKind::Snapshot => b"snapshot\0".as_slice(),
            SourceKind::Video => b"video\0".as_slice(),
        });
        hasher.update(self.path.as_bytes());
        hasher.update([0u8]);
        hasher.update(self.offset_index.to_le_bytes());
        hasher.update(self.len.to_le_bytes());
        let modified_nanos = self
            .modified
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0u128, |d| d.as_nanos());
        hasher.update(modified_nanos.to_le_bytes());
        let digest = hasher.finalize();
        let mut first = [0u8; 8];
        first.copy_from_slice(&digest[..8]);
        u64::from_be_bytes(first)
    }
}

/// Cache key of one rendered thumbnail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThumbnailKey {
    /// The frame whose pixels the thumbnail shows.
    pub frame_id: i64,
    /// Requested width (the served image may be narrower: no upscaling).
    pub width: u32,
    pub quality: u8,
    /// [`SourceStamp::generation`] of the media it was made from.
    pub generation: u64,
}

impl ThumbnailKey {
    /// The quoted entity tag sent as `ETag` for this thumbnail.
    pub fn etag(&self) -> String {
        format!(
            "\"frame-thumbnail-{}-{}-{}-{:016x}\"",
            self.frame_id, self.width, self.quality, self.generation
        )
    }
}

/// An encoded thumbnail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumbnail {
    /// Baseline JPEG bytes; cheap to clone into a response body.
    pub jpeg: Bytes,
    pub width: u32,
    pub height: u32,
}

/// Whether a thumbnail came from memory or was rendered for this request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheStatus {
    Hit,
    Miss,
}

impl CacheStatus {
    /// Value of the `x-screenpipe-thumbnail-cache` header.
    pub fn as_str(self) -> &'static str {
        match self {
            CacheStatus::Hit => "hit",
            CacheStatus::Miss => "miss",
        }
    }
}

/// Why a thumbnail could not be produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThumbnailError {
    /// The source file does not exist (any more).
    SourceMissing(String),
    /// The source exists but could not be read, or is not a regular file.
    SourceUnreadable(String),
    /// A video frame could not be extracted.
    Extract(String),
    /// The source bytes are not a decodable image.
    Decode(String),
    /// Encoding the thumbnail failed.
    Encode(String),
    /// The render worker went away (runtime shutting down).
    Worker(String),
}

impl ThumbnailError {
    /// True when the failure means the media file is gone, as opposed to
    /// present but unusable.
    pub fn is_missing(&self) -> bool {
        matches!(self, ThumbnailError::SourceMissing(_))
    }
}

impl fmt::Display for ThumbnailError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ThumbnailError::SourceMissing(m) => write!(f, "source missing: {m}"),
            ThumbnailError::SourceUnreadable(m) => write!(f, "source unreadable: {m}"),
            ThumbnailError::Extract(m) => write!(f, "frame extraction failed: {m}"),
            ThumbnailError::Decode(m) => write!(f, "source is not a decodable image: {m}"),
            ThumbnailError::Encode(m) => write!(f, "thumbnail encoding failed: {m}"),
            ThumbnailError::Worker(m) => write!(f, "thumbnail worker failed: {m}"),
        }
    }
}

impl std::error::Error for ThumbnailError {}

/// Bounded in-memory cache of encoded thumbnails plus the render limiter.
pub struct FrameThumbnailCache {
    entries: Cache<ThumbnailKey, Arc<Thumbnail>>,
    render_permits: Semaphore,
}

static GLOBAL: LazyLock<FrameThumbnailCache> =
    LazyLock::new(|| FrameThumbnailCache::new(MEMORY_BUDGET_BYTES, render_slots()));

impl FrameThumbnailCache {
    /// A cache holding at most `budget_bytes` of encoded thumbnails, rendering
    /// at most `render_slots` at once.
    pub fn new(budget_bytes: u64, render_slots: usize) -> Self {
        let entries = Cache::builder()
            .max_capacity(budget_bytes)
            .weigher(|_key: &ThumbnailKey, value: &Arc<Thumbnail>| -> u32 {
                let weight = value.jpeg.len().saturating_add(ENTRY_OVERHEAD_BYTES);
                u32::try_from(weight).unwrap_or(u32::MAX)
            })
            .time_to_idle(IDLE_TTL)
            .build();
        Self {
            entries,
            render_permits: Semaphore::new(render_slots.max(1)),
        }
    }

    /// The process-wide cache the frame routes use.
    pub fn global() -> &'static FrameThumbnailCache {
        &GLOBAL
    }

    /// A cached thumbnail, if one exists for `key`.
    pub async fn lookup(&self, key: &ThumbnailKey) -> Option<Arc<Thumbnail>> {
        self.entries.get(key).await
    }

    /// The cached thumbnail for `key`, or one rendered from the bytes `source`
    /// yields (a snapshot JPEG or an extracted video frame) and then cached.
    /// Concurrent requests for the same key share one render.
    pub async fn get_or_render<F>(
        &self,
        key: ThumbnailKey,
        source: F,
    ) -> Result<(Arc<Thumbnail>, CacheStatus), Arc<ThumbnailError>>
    where
        F: std::future::Future<Output = Result<Vec<u8>, ThumbnailError>> + Send,
    {
        if let Some(hit) = self.lookup(&key).await {
            return Ok((hit, CacheStatus::Hit));
        }
        let thumbnail = self
            .entries
            .try_get_with(key, async move {
                let bytes = source.await?;
                self.render(bytes, key.width, key.quality)
                    .await
                    .map(Arc::new)
            })
            .await?;
        Ok((thumbnail, CacheStatus::Miss))
    }

    /// Decode, resize and encode on the blocking pool, within the render limit.
    async fn render(
        &self,
        source: Vec<u8>,
        width: u32,
        quality: u8,
    ) -> Result<Thumbnail, ThumbnailError> {
        let _permit = self
            .render_permits
            .acquire()
            .await
            .map_err(|e| ThumbnailError::Worker(e.to_string()))?;
        tokio::task::spawn_blocking(move || render_thumbnail(&source, width, quality))
            .await
            .map_err(|e| ThumbnailError::Worker(e.to_string()))?
    }

    /// Number of cached thumbnails (after pending maintenance).
    pub async fn entry_count(&self) -> u64 {
        self.entries.run_pending_tasks().await;
        self.entries.entry_count()
    }

    /// Total weight (encoded bytes plus overhead) of the cached thumbnails.
    pub async fn weighted_size(&self) -> u64 {
        self.entries.run_pending_tasks().await;
        self.entries.weighted_size()
    }
}

/// Output size for a `src_w` x `src_h` image scaled to `width`: never wider
/// than the source (no upscaling), aspect ratio kept, at least 1x1.
pub fn thumbnail_size(src_w: u32, src_h: u32, width: u32) -> (u32, u32) {
    if src_w == 0 || src_h == 0 {
        return (src_w.max(1), src_h.max(1));
    }
    let out_w = width.clamp(1, src_w);
    let out_h = (u64::from(src_h) * u64::from(out_w) + u64::from(src_w) / 2) / u64::from(src_w);
    (out_w, u32::try_from(out_h.max(1)).unwrap_or(u32::MAX))
}

/// Decode `source` (any format the image crate recognises, JPEG in
/// practice), shrink it to at most `width` pixels wide and encode it as a
/// baseline JPEG at `quality`.
pub fn render_thumbnail(
    source: &[u8],
    width: u32,
    quality: u8,
) -> Result<Thumbnail, ThumbnailError> {
    let mut reader = ImageReader::new(Cursor::new(source))
        .with_guessed_format()
        .map_err(|e| ThumbnailError::Decode(e.to_string()))?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_SIDE);
    limits.max_image_height = Some(MAX_SOURCE_SIDE);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|e| ThumbnailError::Decode(e.to_string()))?;

    let (src_w, src_h) = image.dimensions();
    let (out_w, out_h) = thumbnail_size(src_w, src_h, width);
    let resized: DynamicImage = if (out_w, out_h) == (src_w, src_h) {
        image
    } else {
        image.thumbnail_exact(out_w, out_h)
    };
    let rgb = resized.to_rgb8();

    let mut jpeg = Vec::with_capacity((out_w as usize * out_h as usize / 8).max(1024));
    JpegEncoder::new_with_quality(&mut jpeg, quality.clamp(MIN_QUALITY, MAX_QUALITY))
        .encode_image(&rgb)
        .map_err(|e| ThumbnailError::Encode(e.to_string()))?;
    Ok(Thumbnail {
        jpeg: Bytes::from(jpeg),
        width: rgb.width(),
        height: rgb.height(),
    })
}

/// True when an `If-None-Match` header value matches `etag` (a quoted
/// strong tag as [`ThumbnailKey::etag`] gives). Lists, weak (`W/`) tags and
/// `*` are understood; comparison is weak, as RFC 9110 asks for this header.
pub fn if_none_match_matches(header_value: &str, etag: &str) -> bool {
    let wanted = etag.trim();
    header_value.split(',').map(str::trim).any(|candidate| {
        if candidate == "*" {
            return true;
        }
        let candidate = candidate.strip_prefix("W/").unwrap_or(candidate);
        !candidate.is_empty() && candidate == wanted
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn jpeg(width: u32, height: u32) -> Vec<u8> {
        let img = ImageBuffer::from_fn(width, height, |x, y| {
            Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8])
        });
        let mut out = Vec::new();
        JpegEncoder::new_with_quality(&mut out, 90)
            .encode_image(&img)
            .unwrap();
        out
    }

    fn key(frame_id: i64, width: u32) -> ThumbnailKey {
        ThumbnailKey {
            frame_id,
            width,
            quality: 75,
            generation: 0xabc,
        }
    }

    #[test]
    fn size_keeps_aspect_and_never_upscales() {
        assert_eq!(thumbnail_size(1920, 1080, 320), (320, 180));
        assert_eq!(thumbnail_size(200, 100, 320), (200, 100));
        assert_eq!(thumbnail_size(1000, 1, 32), (32, 1));
        assert_eq!(thumbnail_size(3, 1000, 1), (1, 333));
        assert_eq!(thumbnail_size(0, 0, 320), (1, 1));
    }

    #[test]
    fn render_resizes_and_reencodes() {
        let thumb = render_thumbnail(&jpeg(640, 360), 320, 75).unwrap();
        assert_eq!((thumb.width, thumb.height), (320, 180));
        let decoded = image::load_from_memory(&thumb.jpeg).unwrap();
        assert_eq!(decoded.dimensions(), (320, 180));
        assert_eq!(&thumb.jpeg[..2], &[0xFF, 0xD8]);
    }

    #[test]
    fn render_does_not_upscale_small_sources() {
        let thumb = render_thumbnail(&jpeg(100, 50), 768, 60).unwrap();
        assert_eq!((thumb.width, thumb.height), (100, 50));
    }

    #[test]
    fn render_rejects_garbage() {
        let err = render_thumbnail(b"definitely not an image", 320, 75).unwrap_err();
        assert!(matches!(err, ThumbnailError::Decode(_)), "{err:?}");
        let err = render_thumbnail(&[], 320, 75).unwrap_err();
        assert!(matches!(err, ThumbnailError::Decode(_)), "{err:?}");
    }

    #[test]
    fn generation_tracks_the_file_identity() {
        let t = UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        let base = SourceStamp {
            kind: SourceKind::Snapshot,
            path: "/data/a.jpg",
            offset_index: 0,
            len: 1000,
            modified: Some(t),
        };
        let g = base.generation();
        assert_eq!(g, base.clone().generation(), "deterministic");
        let moved = SourceStamp {
            kind: SourceKind::Video,
            path: "/data/c.mp4",
            offset_index: 4,
            ..base.clone()
        };
        assert_ne!(g, moved.generation());
        let rewritten = SourceStamp {
            len: 1001,
            ..base.clone()
        };
        assert_ne!(g, rewritten.generation());
        // One microsecond, not one nanosecond: Windows keeps SystemTime in 100 ns ticks,
        // so a 1 ns step is rounded away there (NTFS mtimes have the same resolution).
        let touched = SourceStamp {
            modified: Some(t + Duration::from_micros(1)),
            ..base.clone()
        };
        assert_ne!(g, touched.generation());
        let other_offset = SourceStamp {
            offset_index: 1,
            ..base
        };
        assert_ne!(g, other_offset.generation());
    }

    #[test]
    fn etag_format() {
        let k = ThumbnailKey {
            frame_id: 42,
            width: 320,
            quality: 75,
            generation: 0x1f,
        };
        assert_eq!(k.etag(), "\"frame-thumbnail-42-320-75-000000000000001f\"");
    }

    #[test]
    fn if_none_match_parsing() {
        let tag = "\"frame-thumbnail-1-320-75-00000000000000aa\"";
        assert!(if_none_match_matches(tag, tag));
        assert!(if_none_match_matches(&format!("W/{tag}"), tag));
        assert!(if_none_match_matches(&format!("\"other\", {tag}"), tag));
        assert!(if_none_match_matches("*", tag));
        assert!(!if_none_match_matches("\"other\"", tag));
        assert!(!if_none_match_matches("", tag));
        assert!(!if_none_match_matches(" , ", tag));
        // Unquoted values never match a quoted tag.
        assert!(!if_none_match_matches(tag.trim_matches('"'), tag));
    }

    #[tokio::test]
    async fn second_request_is_a_hit_and_renders_once() {
        let cache = FrameThumbnailCache::new(MEMORY_BUDGET_BYTES, 2);
        let reads = Arc::new(AtomicUsize::new(0));
        let source = jpeg(640, 360);
        for expected in [CacheStatus::Miss, CacheStatus::Hit] {
            let reads = reads.clone();
            let bytes = source.clone();
            let (thumb, status) = cache
                .get_or_render(key(1, 320), async move {
                    reads.fetch_add(1, Ordering::SeqCst);
                    Ok(bytes)
                })
                .await
                .unwrap();
            assert_eq!(status, expected);
            assert_eq!(thumb.width, 320);
        }
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        // A different width or generation is a different entry.
        let (_, status) = cache
            .get_or_render(key(1, 160), async { Ok(jpeg(640, 360)) })
            .await
            .unwrap();
        assert_eq!(status, CacheStatus::Miss);
    }

    #[tokio::test]
    async fn failures_are_not_cached() {
        let cache = FrameThumbnailCache::new(MEMORY_BUDGET_BYTES, 1);
        let err = cache
            .get_or_render(key(9, 320), async {
                Err(ThumbnailError::SourceMissing("gone".into()))
            })
            .await
            .unwrap_err();
        assert!(err.is_missing());
        assert!(cache.lookup(&key(9, 320)).await.is_none());
        let (_, status) = cache
            .get_or_render(key(9, 320), async { Ok(jpeg(64, 64)) })
            .await
            .unwrap();
        assert_eq!(status, CacheStatus::Miss);
    }

    #[tokio::test]
    async fn memory_stays_within_the_budget() {
        // Room for only a few thumbnails of roughly 2-4 KB each.
        let budget = 16 * 1024;
        let cache = FrameThumbnailCache::new(budget, 2);
        for frame_id in 0..64 {
            cache
                .get_or_render(key(frame_id, 128), async { Ok(jpeg(256, 256)) })
                .await
                .unwrap();
        }
        let size = cache.weighted_size().await;
        assert!(size <= budget, "cache holds {size} bytes, budget {budget}");
        let count = cache.entry_count().await;
        assert!(
            count > 0 && count < 64,
            "evictions expected, {count} entries"
        );
    }
}
