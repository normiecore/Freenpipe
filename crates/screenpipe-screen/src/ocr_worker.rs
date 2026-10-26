// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! A dedicated OCR thread with a bounded request queue, and the image checks
//! that run before pixels reach a platform OCR engine.
//!
//! Windows runs its WinRT `OcrEngine` on one long-lived thread named
//! [`WINDOWS_OCR_THREAD_NAME`] (see `microsoft.rs`). Live capture and the
//! on-demand `POST /frames/:id/text` route share that thread's queue. The
//! worker here is platform-neutral, so its queueing, cancellation, restart and
//! validation rules are unit-tested on every OS.
//!
//! Rules the worker keeps:
//! - The queue is bounded. A request that does not fit is rejected at once
//!   with [`OcrWorkerError::QueueFull`]; the caller stores its frame without
//!   OCR text instead of waiting, so the capture loop never blocks on OCR.
//! - A request whose caller has gone away (timed out, HTTP client
//!   disconnected) is dropped without running.
//! - A panic inside the handler answers that caller with an error and ends
//!   the thread; requests still queued behind it are answered with
//!   [`OcrWorkerError::ExitedBeforeTaking`] (their reply channel closes), and
//!   the next request starts a fresh thread.
//! - A thread stuck on one request for longer than
//!   [`WorkerConfig::wedge_after`] is abandoned and replaced, up to
//!   [`WorkerConfig::max_abandoned`] stuck threads at a time. Requests queued
//!   behind the stuck one are taken out of its queue at that moment and
//!   answered with [`OcrWorkerError::ExitedBeforeTaking`], so their images
//!   are freed instead of waiting on a call that may never return.

use std::collections::VecDeque;
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, PoisonError, TryLockError};
use std::time::{Duration, Instant};

use image::{DynamicImage, GenericImageView};
use tokio::sync::oneshot;
use tracing::{debug, error, info, warn};

/// Name of the thread that owns the WinRT OCR engine.
pub const WINDOWS_OCR_THREAD_NAME: &str = "screenpipe-windows-ocr";

/// Requests that may wait behind the one being processed.
pub const DEFAULT_OCR_QUEUE_CAPACITY: usize = 4;

/// How long live capture waits for OCR before storing the frame without
/// text. Kept well under the capture loop's own per-capture deadline so a
/// slow OCR never turns into a dropped frame.
pub const LIVE_OCR_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the on-demand OCR route waits for an answer.
pub const ON_DEMAND_OCR_TIMEOUT: Duration = Duration::from_secs(30);

/// On-demand OCR requests allowed in flight at once (the rest get a "busy"
/// answer), so live capture keeps most of the shared queue.
pub const ON_DEMAND_OCR_MAX_INFLIGHT: usize = 2;

/// A source image wider or taller than this is refused outright instead of
/// being scaled: nothing a screen capture produces is this large, so such a
/// frame is corrupt and must not trigger a huge allocation.
pub const SOURCE_SIDE_LIMIT: u32 = 32_768;

/// Most pixels handed to an OCR engine in one request. 5K and 6K monitors fit
/// unscaled; 8K frames are scaled to about 6.6K, which keeps text legible
/// while bounding the memory a full queue can hold.
pub const OCR_PIXEL_BUDGET: u64 = 25_000_000;

/// Longest side assumed for the WinRT engine until the worker has read the
/// real `OcrEngine::MaxImageDimension`. The worker re-checks every image
/// against the real value, so this only decides how much the caller scales.
pub const FALLBACK_MAX_IMAGE_DIMENSION: u32 = 10_000;

/// Restarts allowed within [`RESTART_WINDOW`] before new threads are refused
/// for a while (keeps a handler that always crashes from spinning).
const MAX_RESTARTS_PER_WINDOW: usize = 5;
const RESTART_WINDOW: Duration = Duration::from_secs(60);

/// Minimum spacing of the "queue full" warning.
const QUEUE_FULL_WARN_EVERY: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// Image validation
// ---------------------------------------------------------------------------

/// Why an image was refused before it reached the OCR engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcrImageError {
    /// The pixel buffer is empty.
    EmptyBuffer,
    /// Width or height is zero.
    ZeroSize { width: u32, height: u32 },
    /// The width exceeds the limit that applies at this stage.
    WidthTooLarge { width: u32, limit: u32 },
    /// The height exceeds the limit that applies at this stage.
    HeightTooLarge { height: u32, limit: u32 },
    /// The row stride is shorter than a row of pixels or not a whole number
    /// of 4-byte pixels.
    BadStride { stride: usize, width: u32 },
    /// The buffer does not hold the bytes the dimensions call for.
    LengthMismatch { expected: usize, actual: usize },
    /// Width times height (times bytes per pixel) does not fit in memory
    /// arithmetic.
    Overflow { width: u32, height: u32 },
}

impl fmt::Display for OcrImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyBuffer => write!(f, "image has no pixel data"),
            Self::ZeroSize { width, height } => {
                write!(f, "image has a zero dimension ({}x{})", width, height)
            }
            Self::WidthTooLarge { width, limit } => {
                write!(f, "image width {} is over the limit of {}", width, limit)
            }
            Self::HeightTooLarge { height, limit } => {
                write!(f, "image height {} is over the limit of {}", height, limit)
            }
            Self::BadStride { stride, width } => write!(
                f,
                "row stride of {} bytes does not fit {} BGRA pixels per row",
                stride, width
            ),
            Self::LengthMismatch { expected, actual } => write!(
                f,
                "BGRA buffer holds {} bytes but the dimensions need {}",
                actual, expected
            ),
            Self::Overflow { width, height } => write!(
                f,
                "image size {}x{} overflows the buffer arithmetic",
                width, height
            ),
        }
    }
}

impl std::error::Error for OcrImageError {}

/// Size to hand to the OCR engine for a `width` x `height` source: the
/// source itself when it fits, otherwise the largest size with the same
/// aspect ratio whose sides stay within `max_side` and whose pixel count stays
/// within `pixel_budget`. Zero-sized, absurdly large and overflowing sources
/// are refused.
pub fn plan_ocr_dimensions(
    width: u32,
    height: u32,
    max_side: u32,
    pixel_budget: u64,
) -> Result<(u32, u32), OcrImageError> {
    if width == 0 || height == 0 {
        return Err(OcrImageError::ZeroSize { width, height });
    }
    if width > SOURCE_SIDE_LIMIT {
        return Err(OcrImageError::WidthTooLarge {
            width,
            limit: SOURCE_SIDE_LIMIT,
        });
    }
    if height > SOURCE_SIDE_LIMIT {
        return Err(OcrImageError::HeightTooLarge {
            height,
            limit: SOURCE_SIDE_LIMIT,
        });
    }
    let pixels = u64::from(width) * u64::from(height);
    usize::try_from(pixels)
        .ok()
        .and_then(|p| p.checked_mul(4))
        .ok_or(OcrImageError::Overflow { width, height })?;

    let max_side = if max_side == 0 {
        FALLBACK_MAX_IMAGE_DIMENSION
    } else {
        max_side
    };
    let pixel_budget = pixel_budget.max(1);

    let mut scale: f64 = 1.0;
    scale = scale.min(f64::from(max_side) / f64::from(width));
    scale = scale.min(f64::from(max_side) / f64::from(height));
    if pixels > pixel_budget {
        scale = scale.min((pixel_budget as f64 / pixels as f64).sqrt());
    }
    if scale >= 1.0 {
        return Ok((width, height));
    }

    let mut w = ((f64::from(width) * scale).floor() as u32).clamp(1, max_side);
    let mut h = ((f64::from(height) * scale).floor() as u32).clamp(1, max_side);
    // Floating point can leave the product a hair over the budget.
    while u64::from(w) * u64::from(h) > pixel_budget && (w > 1 || h > 1) {
        if w >= h {
            w -= 1;
        } else {
            h -= 1;
        }
    }
    Ok((w, h))
}

/// Checks a BGRA pixel layout before it is copied into an OCR bitmap.
/// `stride` is the distance in bytes between the starts of two rows.
pub fn validate_bgra_layout(
    width: u32,
    height: u32,
    stride: usize,
    buffer_len: usize,
    max_side: u32,
) -> Result<(), OcrImageError> {
    if buffer_len == 0 {
        return Err(OcrImageError::EmptyBuffer);
    }
    if width == 0 || height == 0 {
        return Err(OcrImageError::ZeroSize { width, height });
    }
    if width > max_side {
        return Err(OcrImageError::WidthTooLarge {
            width,
            limit: max_side,
        });
    }
    if height > max_side {
        return Err(OcrImageError::HeightTooLarge {
            height,
            limit: max_side,
        });
    }
    let row = (width as usize)
        .checked_mul(4)
        .ok_or(OcrImageError::Overflow { width, height })?;
    if stride < row || stride % 4 != 0 {
        return Err(OcrImageError::BadStride { stride, width });
    }
    let needed = stride
        .checked_mul(height as usize - 1)
        .and_then(|v| v.checked_add(row))
        .ok_or(OcrImageError::Overflow { width, height })?;
    if buffer_len < needed {
        return Err(OcrImageError::LengthMismatch {
            expected: needed,
            actual: buffer_len,
        });
    }
    Ok(())
}

/// A tightly packed BGRA image (4 bytes per pixel, rows back to back) that
/// has passed validation.
#[derive(Clone, PartialEq, Eq)]
pub struct BgraImage {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl fmt::Debug for BgraImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BgraImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.data.len())
            .finish()
    }
}

impl BgraImage {
    /// Builds an image from raw BGRA rows, repacking them when `stride` has
    /// padding. Fails without panicking on any inconsistent layout.
    pub fn from_parts(
        width: u32,
        height: u32,
        stride: usize,
        data: Vec<u8>,
        max_side: u32,
    ) -> Result<Self, OcrImageError> {
        validate_bgra_layout(width, height, stride, data.len(), max_side)?;
        let row = width as usize * 4;
        let data = if stride == row {
            let mut data = data;
            data.truncate(row * height as usize);
            data
        } else {
            let mut packed = Vec::with_capacity(row * height as usize);
            for y in 0..height as usize {
                let start = y * stride;
                packed.extend_from_slice(&data[start..start + row]);
            }
            packed
        };
        let expected = row * height as usize;
        if data.len() != expected {
            return Err(OcrImageError::LengthMismatch {
                expected,
                actual: data.len(),
            });
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    /// Converts a decoded frame to BGRA, scaled down when it is larger than
    /// `max_side` or `pixel_budget` allow. The size is checked before any
    /// pixel buffer is allocated.
    pub fn from_dynamic(
        image: &DynamicImage,
        max_side: u32,
        pixel_budget: u64,
    ) -> Result<Self, OcrImageError> {
        let (width, height) = image.dimensions();
        let (w, h) = plan_ocr_dimensions(width, height, max_side, pixel_budget)?;
        let mut rgba = if (w, h) == (width, height) {
            image.to_rgba8()
        } else {
            image
                .resize_exact(w, h, image::imageops::FilterType::Triangle)
                .into_rgba8()
        };
        for px in rgba.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        let (w, h) = rgba.dimensions();
        Self::from_parts(
            w,
            h,
            w as usize * 4,
            rgba.into_raw(),
            max_side.max(w).max(h),
        )
    }

    /// Returns the image scaled down to fit `max_side` and `pixel_budget`, or
    /// unchanged when it already fits.
    pub fn fit_within(self, max_side: u32, pixel_budget: u64) -> Result<Self, OcrImageError> {
        let (w, h) = plan_ocr_dimensions(self.width, self.height, max_side, pixel_budget)?;
        if (w, h) == (self.width, self.height) {
            return Ok(self);
        }
        // Channel order does not matter for resampling, so the BGRA bytes go
        // through the RGBA resizer unchanged.
        let (width, height) = (self.width, self.height);
        let buffer = image::RgbaImage::from_raw(width, height, self.data).ok_or(
            OcrImageError::LengthMismatch {
                expected: width as usize * height as usize * 4,
                actual: 0,
            },
        )?;
        let scaled = image::imageops::resize(&buffer, w, h, image::imageops::FilterType::Triangle);
        Self::from_parts(
            w,
            h,
            w as usize * 4,
            scaled.into_raw(),
            max_side.max(w).max(h),
        )
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The packed BGRA bytes (`width * height * 4` of them).
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why an OCR request produced no text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OcrWorkerError {
    /// The queue was full; the request was not taken.
    QueueFull { capacity: usize },
    /// The worker thread could not be started (or restarted too often).
    StartFailed(String),
    /// The worker thread stopped before it took this request.
    ExitedBeforeTaking,
    /// The handler panicked on this request; the thread is being replaced.
    Panicked(String),
    /// The worker is stuck on an earlier request and no replacement may be
    /// started right now.
    Stuck { busy_secs: u64 },
    /// No answer arrived within the caller's deadline.
    TimedOut { after_ms: u64 },
    /// Too many on-demand requests are running already.
    Busy { limit: usize },
    /// The image was refused before it reached the engine.
    InvalidImage(OcrImageError),
    /// No OCR engine or recognizer language is available.
    Unavailable(String),
    /// The OCR engine reported an error.
    Engine(String),
}

impl OcrWorkerError {
    /// True when the request was shed (load, missing engine or a bad image)
    /// rather than failing unexpectedly. The frame is stored without OCR text
    /// and this is not a capture failure.
    pub fn is_skip(&self) -> bool {
        matches!(
            self,
            Self::QueueFull { .. }
                | Self::Busy { .. }
                | Self::TimedOut { .. }
                | Self::Stuck { .. }
                | Self::Unavailable(_)
                | Self::InvalidImage(_)
        )
    }

    /// True when trying again later may succeed.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::QueueFull { .. }
                | Self::Busy { .. }
                | Self::TimedOut { .. }
                | Self::Stuck { .. }
                | Self::ExitedBeforeTaking
                | Self::Panicked(_)
                | Self::StartFailed(_)
        )
    }
}

impl fmt::Display for OcrWorkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueueFull { capacity } => write!(
                f,
                "OCR request skipped: {} requests are already waiting",
                capacity
            ),
            Self::StartFailed(e) => write!(f, "OCR worker thread could not start: {}", e),
            Self::ExitedBeforeTaking => {
                write!(f, "OCR worker stopped before it reached this request")
            }
            Self::Panicked(msg) => write!(f, "OCR worker crashed on this request: {}", msg),
            Self::Stuck { busy_secs } => write!(
                f,
                "OCR worker has been busy with one request for {}s and cannot be replaced yet",
                busy_secs
            ),
            Self::TimedOut { after_ms } => write!(f, "OCR gave no answer within {} ms", after_ms),
            Self::Busy { limit } => write!(
                f,
                "{} on-demand OCR requests are already running; try again shortly",
                limit
            ),
            Self::InvalidImage(e) => write!(f, "OCR refused the image: {}", e),
            Self::Unavailable(reason) => write!(f, "OCR unavailable: {}", reason),
            Self::Engine(e) => write!(f, "OCR engine error: {}", e),
        }
    }
}

impl std::error::Error for OcrWorkerError {}

impl From<OcrImageError> for OcrWorkerError {
    fn from(e: OcrImageError) -> Self {
        Self::InvalidImage(e)
    }
}

/// Finds an [`OcrWorkerError`] inside an `anyhow` chain.
pub fn worker_error_of(e: &anyhow::Error) -> Option<&OcrWorkerError> {
    e.chain().find_map(|c| c.downcast_ref::<OcrWorkerError>())
}

// ---------------------------------------------------------------------------
// The worker
// ---------------------------------------------------------------------------

/// Tuning of a [`DedicatedWorker`].
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// OS thread name (shows up in logs, debuggers and crash dumps).
    pub thread_name: String,
    /// Requests that may wait behind the one being processed (at least 1).
    pub queue_capacity: usize,
    /// A request running longer than this marks the thread as stuck.
    pub wedge_after: Duration,
    /// Stuck threads that may be left behind at once; further stuck states
    /// are reported as [`OcrWorkerError::Stuck`].
    pub max_abandoned: usize,
}

impl WorkerConfig {
    /// The settings of the Windows OCR thread.
    pub fn windows_ocr() -> Self {
        Self {
            thread_name: WINDOWS_OCR_THREAD_NAME.to_string(),
            queue_capacity: DEFAULT_OCR_QUEUE_CAPACITY,
            wedge_after: Duration::from_secs(60),
            max_abandoned: 2,
        }
    }
}

/// Counters of a [`DedicatedWorker`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkerStats {
    pub submitted: u64,
    pub completed: u64,
    pub rejected_queue_full: u64,
    pub dropped_cancelled: u64,
    pub panics: u64,
    pub threads_started: u64,
    pub restarts: u64,
    pub abandoned_stuck: u64,
    /// Requests waiting in the queue right now.
    pub queued: u64,
}

#[derive(Default)]
struct StatCells {
    submitted: AtomicU64,
    completed: AtomicU64,
    rejected_queue_full: AtomicU64,
    dropped_cancelled: AtomicU64,
    panics: AtomicU64,
    threads_started: AtomicU64,
    restarts: AtomicU64,
    abandoned_stuck: AtomicU64,
    queued: AtomicU64,
    queue_full_since_warn: AtomicU64,
}

const THREAD_RUNNING: u8 = 0;
const THREAD_ABANDONED: u8 = 1;
const THREAD_EXITED: u8 = 2;

/// State one worker thread shares with the submit side.
struct ThreadShared {
    state: AtomicU8,
    /// Milliseconds since the worker epoch (plus one) when the current
    /// request started; zero while idle.
    busy_since_ms: AtomicU64,
}

/// Builds the per-thread request handler. Runs on the worker thread, so the
/// handler (and anything it owns, such as COM objects) never leaves it.
pub type HandlerFactory<J, R> = Arc<dyn Fn() -> Box<dyn FnMut(J) -> R> + Send + Sync>;

/// Wraps a function that builds a handler into a [`HandlerFactory`].
pub fn handler_factory<J, R, F, H>(make: F) -> HandlerFactory<J, R>
where
    F: Fn() -> H + Send + Sync + 'static,
    H: FnMut(J) -> R + 'static,
{
    Arc::new(move || Box::new(make()) as Box<dyn FnMut(J) -> R>)
}

type ReplyTx<R> = oneshot::Sender<Result<R, OcrWorkerError>>;
type ReplyRx<R> = oneshot::Receiver<Result<R, OcrWorkerError>>;

/// Keeps the queued gauge right however an envelope ends (taken, refused by
/// the channel, or dropped with a dead thread's queue).
struct QueuedMark(Arc<StatCells>);

impl QueuedMark {
    fn new(stats: &Arc<StatCells>) -> Self {
        stats.queued.fetch_add(1, Ordering::Relaxed);
        Self(stats.clone())
    }
}

impl Drop for QueuedMark {
    fn drop(&mut self) {
        self.0.queued.fetch_sub(1, Ordering::Relaxed);
    }
}

struct Envelope<J, R> {
    job: J,
    reply: ReplyTx<R>,
    queued: QueuedMark,
}

/// The receiving end of a worker thread's queue. The thread holds the lock
/// only while it waits for the next request, never while it runs one, so the
/// submit side can empty the queue of a thread it abandons.
type SharedRx<J, R> = Arc<Mutex<Receiver<Envelope<J, R>>>>;

struct Slot<J, R> {
    tx: SyncSender<Envelope<J, R>>,
    rx: SharedRx<J, R>,
    shared: Arc<ThreadShared>,
}

struct Inner<J, R> {
    slot: Option<Slot<J, R>>,
    generation: u64,
    recent_starts: VecDeque<Instant>,
}

/// Runs requests of type `J` one at a time on a dedicated, named OS thread
/// and answers each with an `R`. See the module docs for the rules.
pub struct DedicatedWorker<J: Send + 'static, R: Send + 'static> {
    config: WorkerConfig,
    factory: HandlerFactory<J, R>,
    inner: Mutex<Inner<J, R>>,
    stats: Arc<StatCells>,
    abandoned_live: Arc<AtomicUsize>,
    epoch: Instant,
    last_queue_full_warn: Mutex<Option<Instant>>,
}

impl<J: Send + 'static, R: Send + 'static> DedicatedWorker<J, R> {
    /// Creates the worker. No thread starts until the first request.
    pub fn new(config: WorkerConfig, factory: HandlerFactory<J, R>) -> Self {
        let config = WorkerConfig {
            queue_capacity: config.queue_capacity.max(1),
            ..config
        };
        Self {
            config,
            factory,
            inner: Mutex::new(Inner {
                slot: None,
                generation: 0,
                recent_starts: VecDeque::new(),
            }),
            stats: Arc::new(StatCells::default()),
            abandoned_live: Arc::new(AtomicUsize::new(0)),
            epoch: Instant::now(),
            last_queue_full_warn: Mutex::new(None),
        }
    }

    pub fn config(&self) -> &WorkerConfig {
        &self.config
    }

    /// Queues `job` without blocking. The returned receiver yields the
    /// handler's answer, or an error when the thread died first.
    pub fn submit(&self, job: J) -> Result<ReplyRx<R>, OcrWorkerError> {
        self.stats.submitted.fetch_add(1, Ordering::Relaxed);
        let (reply_tx, reply_rx) = oneshot::channel();
        let mut envelope = Envelope {
            job,
            reply: reply_tx,
            queued: QueuedMark::new(&self.stats),
        };

        // The lock only guards slot bookkeeping, which cannot panic; recover
        // from poisoning instead of failing every later request.
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        self.retire_unhealthy_thread(&mut inner)?;

        for _ in 0..2 {
            if inner.slot.is_none() {
                self.start_thread(&mut inner)?;
            }
            let Some(slot) = inner.slot.as_ref() else {
                return Err(OcrWorkerError::StartFailed(
                    "no worker thread after start".to_string(),
                ));
            };
            match slot.tx.try_send(envelope) {
                Ok(()) => return Ok(reply_rx),
                Err(TrySendError::Full(_refused)) => {
                    drop(inner);
                    self.note_queue_full();
                    return Err(OcrWorkerError::QueueFull {
                        capacity: self.config.queue_capacity,
                    });
                }
                Err(TrySendError::Disconnected(returned)) => {
                    // The thread exited between the health check and the
                    // send; start a fresh one and try once more.
                    inner.slot = None;
                    envelope = returned;
                }
            }
        }
        Err(OcrWorkerError::ExitedBeforeTaking)
    }

    /// Queues `job` and waits up to `timeout` for the answer. Dropping the
    /// returned future (or timing out) cancels the request if it has not
    /// started yet.
    pub async fn run(&self, job: J, timeout: Duration) -> Result<R, OcrWorkerError> {
        let rx = self.submit(job)?;
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_closed)) => Err(OcrWorkerError::ExitedBeforeTaking),
            Err(_elapsed) => Err(OcrWorkerError::TimedOut {
                after_ms: timeout.as_millis() as u64,
            }),
        }
    }

    /// Current counters.
    pub fn stats(&self) -> WorkerStats {
        let s = &self.stats;
        WorkerStats {
            submitted: s.submitted.load(Ordering::Relaxed),
            completed: s.completed.load(Ordering::Relaxed),
            rejected_queue_full: s.rejected_queue_full.load(Ordering::Relaxed),
            dropped_cancelled: s.dropped_cancelled.load(Ordering::Relaxed),
            panics: s.panics.load(Ordering::Relaxed),
            threads_started: s.threads_started.load(Ordering::Relaxed),
            restarts: s.restarts.load(Ordering::Relaxed),
            abandoned_stuck: s.abandoned_stuck.load(Ordering::Relaxed),
            queued: s.queued.load(Ordering::Relaxed),
        }
    }

    /// Whether a worker thread is running right now.
    pub fn is_running(&self) -> bool {
        let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner
            .slot
            .as_ref()
            .is_some_and(|s| s.shared.state.load(Ordering::SeqCst) == THREAD_RUNNING)
    }

    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64 + 1
    }

    /// Drops the slot of a thread that exited, and abandons one stuck on a
    /// request for too long.
    fn retire_unhealthy_thread(&self, inner: &mut Inner<J, R>) -> Result<(), OcrWorkerError> {
        let Some(slot) = inner.slot.as_ref() else {
            return Ok(());
        };
        if slot.shared.state.load(Ordering::SeqCst) != THREAD_RUNNING {
            debug!(
                thread = %self.config.thread_name,
                "OCR worker thread has exited; a new one will start"
            );
            inner.slot = None;
            return Ok(());
        }
        let started = slot.shared.busy_since_ms.load(Ordering::SeqCst);
        if started == 0 {
            return Ok(());
        }
        let busy = Duration::from_millis(self.now_ms().saturating_sub(started));
        if busy < self.config.wedge_after {
            return Ok(());
        }
        if self.abandoned_live.load(Ordering::SeqCst) >= self.config.max_abandoned {
            return Err(OcrWorkerError::Stuck {
                busy_secs: busy.as_secs(),
            });
        }
        // Count first, then claim, so the exiting thread's decrement can
        // never run before this increment.
        self.abandoned_live.fetch_add(1, Ordering::SeqCst);
        match slot.shared.state.compare_exchange(
            THREAD_RUNNING,
            THREAD_ABANDONED,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => {
                self.stats.abandoned_stuck.fetch_add(1, Ordering::Relaxed);
                let released = release_queued(&slot.rx);
                warn!(
                    thread = %self.config.thread_name,
                    busy_secs = busy.as_secs(),
                    released,
                    "OCR worker thread is stuck on one request; leaving it behind and starting a new one"
                );
            }
            Err(_) => {
                // It exited just now; nothing to abandon.
                self.abandoned_live.fetch_sub(1, Ordering::SeqCst);
            }
        }
        inner.slot = None;
        Ok(())
    }

    fn start_thread(&self, inner: &mut Inner<J, R>) -> Result<(), OcrWorkerError> {
        let now = Instant::now();
        while inner
            .recent_starts
            .front()
            .is_some_and(|t| now.duration_since(*t) > RESTART_WINDOW)
        {
            inner.recent_starts.pop_front();
        }
        if inner.generation > 0 && inner.recent_starts.len() > MAX_RESTARTS_PER_WINDOW {
            return Err(OcrWorkerError::StartFailed(format!(
                "restarted {} times in the last {}s; waiting before the next attempt",
                inner.recent_starts.len() - 1,
                RESTART_WINDOW.as_secs()
            )));
        }

        let (tx, rx) = mpsc::sync_channel::<Envelope<J, R>>(self.config.queue_capacity);
        let rx: SharedRx<J, R> = Arc::new(Mutex::new(rx));
        let shared = Arc::new(ThreadShared {
            state: AtomicU8::new(THREAD_RUNNING),
            busy_since_ms: AtomicU64::new(0),
        });
        let generation = inner.generation + 1;
        let ctx = ThreadContext {
            factory: self.factory.clone(),
            shared: shared.clone(),
            stats: self.stats.clone(),
            abandoned_live: self.abandoned_live.clone(),
            epoch: self.epoch,
            name: self.config.thread_name.clone(),
            generation,
        };
        std::thread::Builder::new()
            .name(self.config.thread_name.clone())
            .spawn({
                let rx = rx.clone();
                move || ctx.run(rx)
            })
            .map_err(|e| {
                error!(
                    thread = %self.config.thread_name,
                    "failed to start OCR worker thread: {}", e
                );
                OcrWorkerError::StartFailed(e.to_string())
            })?;

        inner.generation = generation;
        inner.recent_starts.push_back(now);
        inner.slot = Some(Slot { tx, rx, shared });
        self.stats.threads_started.fetch_add(1, Ordering::Relaxed);
        if generation > 1 {
            self.stats.restarts.fetch_add(1, Ordering::Relaxed);
            warn!(
                thread = %self.config.thread_name,
                generation,
                "OCR worker thread restarted"
            );
        } else {
            info!(
                thread = %self.config.thread_name,
                queue_capacity = self.config.queue_capacity,
                "OCR worker thread {} started",
                self.config.thread_name
            );
        }
        Ok(())
    }

    fn note_queue_full(&self) {
        self.stats
            .rejected_queue_full
            .fetch_add(1, Ordering::Relaxed);
        let skipped = self
            .stats
            .queue_full_since_warn
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        let mut last = self
            .last_queue_full_warn
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let due = last.is_none_or(|t| t.elapsed() >= QUEUE_FULL_WARN_EVERY);
        if due {
            *last = Some(Instant::now());
            self.stats.queue_full_since_warn.store(0, Ordering::Relaxed);
            warn!(
                thread = %self.config.thread_name,
                skipped,
                "OCR queue full; frames are stored without OCR text until it drains"
            );
        } else {
            debug!(thread = %self.config.thread_name, "OCR queue full; request skipped");
        }
    }
}

/// Everything a worker thread owns.
struct ThreadContext<J, R> {
    factory: HandlerFactory<J, R>,
    shared: Arc<ThreadShared>,
    stats: Arc<StatCells>,
    abandoned_live: Arc<AtomicUsize>,
    epoch: Instant,
    name: String,
    generation: u64,
}

/// Marks the thread as exited however it ends (return or unwind).
struct ExitMark {
    shared: Arc<ThreadShared>,
    abandoned_live: Arc<AtomicUsize>,
}

impl Drop for ExitMark {
    fn drop(&mut self) {
        let previous = self.shared.state.swap(THREAD_EXITED, Ordering::SeqCst);
        if previous == THREAD_ABANDONED {
            self.abandoned_live.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// Answers every request waiting in an abandoned thread's queue with
/// [`OcrWorkerError::ExitedBeforeTaking`] and frees its image. Returns how
/// many were released. When the thread holds the lock it is no longer stuck
/// (it is waiting for the next request), and it works through the queue
/// itself before it exits.
fn release_queued<J, R>(rx: &Mutex<Receiver<Envelope<J, R>>>) -> usize {
    let rx = match rx.try_lock() {
        Ok(rx) => rx,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return 0,
    };
    let mut released = 0;
    while let Ok(envelope) = rx.try_recv() {
        let _ = envelope.reply.send(Err(OcrWorkerError::ExitedBeforeTaking));
        released += 1;
    }
    released
}

impl<J, R> ThreadContext<J, R> {
    fn run(self, rx: SharedRx<J, R>) {
        let exit = ExitMark {
            shared: self.shared.clone(),
            abandoned_live: self.abandoned_live.clone(),
        };
        debug!(
            thread = %self.name,
            generation = self.generation,
            "OCR worker thread running"
        );
        let mut handler = match catch_unwind(AssertUnwindSafe(|| (self.factory)())) {
            Ok(handler) => handler,
            Err(panic) => {
                error!(
                    thread = %self.name,
                    "OCR worker could not set up its engine: {}",
                    panic_text(panic.as_ref())
                );
                return;
            }
        };

        loop {
            // The lock guard is a temporary of this statement: it is released
            // as soon as a request arrives, before the request runs.
            let next = rx.lock().unwrap_or_else(PoisonError::into_inner).recv();
            let Ok(envelope) = next else { break };
            let Envelope { job, reply, queued } = envelope;
            drop(queued);
            if reply.is_closed() {
                self.stats.dropped_cancelled.fetch_add(1, Ordering::Relaxed);
                debug!(thread = %self.name, "OCR request dropped: its caller has gone");
                continue;
            }
            let started = self.epoch.elapsed().as_millis() as u64 + 1;
            self.shared.busy_since_ms.store(started, Ordering::SeqCst);
            let outcome = catch_unwind(AssertUnwindSafe(|| handler(job)));
            self.shared.busy_since_ms.store(0, Ordering::SeqCst);
            match outcome {
                Ok(answer) => {
                    self.stats.completed.fetch_add(1, Ordering::Relaxed);
                    let _ = reply.send(Ok(answer));
                }
                Err(panic) => {
                    let msg = panic_text(panic.as_ref());
                    self.stats.panics.fetch_add(1, Ordering::Relaxed);
                    error!(
                        thread = %self.name,
                        generation = self.generation,
                        "OCR worker panicked; replacing the thread: {}", msg
                    );
                    let _ = reply.send(Err(OcrWorkerError::Panicked(msg)));
                    // The handler's state may be half-updated: end the thread
                    // so the next request starts from a clean one.
                    break;
                }
            }
        }
        // Mark the thread as gone before tearing the handler down, so a
        // request submitted meanwhile starts a fresh thread instead of
        // queueing behind this one.
        drop(exit);
        drop(handler);
        debug!(thread = %self.name, generation = self.generation, "OCR worker thread stopped");
    }
}

fn panic_text(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

// ---------------------------------------------------------------------------
// On-demand concurrency
// ---------------------------------------------------------------------------

/// Counts requests in flight and refuses new ones past a limit. Used by the
/// on-demand OCR route so a burst of timeline requests cannot take every
/// slot of the queue live capture also needs.
pub struct InflightGate {
    current: AtomicUsize,
    limit: usize,
}

impl InflightGate {
    pub const fn new(limit: usize) -> Self {
        Self {
            current: AtomicUsize::new(0),
            limit,
        }
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Requests in flight right now.
    pub fn current(&self) -> usize {
        self.current.load(Ordering::SeqCst)
    }

    /// Takes a slot, or `None` when `limit` requests are already in flight.
    pub fn try_enter(&self) -> Option<InflightTicket<'_>> {
        let mut seen = self.current.load(Ordering::SeqCst);
        loop {
            if seen >= self.limit {
                return None;
            }
            match self
                .current
                .compare_exchange(seen, seen + 1, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => {
                    return Some(InflightTicket {
                        gate: self,
                        inflight: seen + 1,
                    })
                }
                Err(now) => seen = now,
            }
        }
    }
}

/// A slot held in an [`InflightGate`]; released on drop.
pub struct InflightTicket<'a> {
    gate: &'a InflightGate,
    inflight: usize,
}

impl InflightTicket<'_> {
    /// Requests in flight when this one entered, itself included.
    pub fn inflight(&self) -> usize {
        self.inflight
    }
}

impl Drop for InflightTicket<'_> {
    fn drop(&mut self) {
        self.gate.current.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage, Rgba, RgbaImage};
    use std::sync::atomic::AtomicBool;
    use std::sync::Condvar;

    // --- image validation -------------------------------------------------

    #[test]
    fn zero_dimensions_are_rejected() {
        assert_eq!(
            plan_ocr_dimensions(0, 10, 100, OCR_PIXEL_BUDGET),
            Err(OcrImageError::ZeroSize {
                width: 0,
                height: 10
            })
        );
        assert_eq!(
            plan_ocr_dimensions(10, 0, 100, OCR_PIXEL_BUDGET),
            Err(OcrImageError::ZeroSize {
                width: 10,
                height: 0
            })
        );
        assert!(matches!(
            validate_bgra_layout(0, 4, 0, 16, 100),
            Err(OcrImageError::ZeroSize { .. })
        ));
    }

    #[test]
    fn absurd_sources_are_rejected_not_scaled() {
        assert!(matches!(
            plan_ocr_dimensions(SOURCE_SIDE_LIMIT + 1, 10, 100, OCR_PIXEL_BUDGET),
            Err(OcrImageError::WidthTooLarge { .. })
        ));
        assert!(matches!(
            plan_ocr_dimensions(10, u32::MAX, 100, OCR_PIXEL_BUDGET),
            Err(OcrImageError::HeightTooLarge { .. })
        ));
    }

    #[test]
    fn small_images_keep_their_size() {
        assert_eq!(
            plan_ocr_dimensions(1920, 1080, 10_000, OCR_PIXEL_BUDGET),
            Ok((1920, 1080))
        );
    }

    #[test]
    fn oversized_images_scale_to_fit_with_aspect_ratio() {
        let (w, h) = plan_ocr_dimensions(20_000, 5_000, 10_000, OCR_PIXEL_BUDGET).unwrap();
        assert_eq!((w, h), (10_000, 2_500));

        let (w, h) = plan_ocr_dimensions(7_680, 4_320, 4_096, OCR_PIXEL_BUDGET).unwrap();
        assert!(w <= 4_096 && h <= 4_096);
        assert_eq!(w, 4_096);
        assert!((h as i64 - 2_304).abs() <= 1);
    }

    #[test]
    fn pixel_budget_caps_area() {
        let (w, h) = plan_ocr_dimensions(10_000, 10_000, 10_000, 1_000_000).unwrap();
        assert!(u64::from(w) * u64::from(h) <= 1_000_000);
        assert!(w >= 990 && h >= 990);
        assert_eq!(w, h);
    }

    #[test]
    fn a_zero_max_side_falls_back_to_the_default_limit() {
        assert_eq!(
            plan_ocr_dimensions(20_000, 100, 0, OCR_PIXEL_BUDGET),
            Ok((FALLBACK_MAX_IMAGE_DIMENSION, 50))
        );
    }

    #[test]
    fn layout_checks_catch_bad_buffers() {
        assert_eq!(
            validate_bgra_layout(2, 2, 8, 0, 100),
            Err(OcrImageError::EmptyBuffer)
        );
        assert_eq!(
            validate_bgra_layout(2, 2, 8, 15, 100),
            Err(OcrImageError::LengthMismatch {
                expected: 16,
                actual: 15
            })
        );
        assert_eq!(
            validate_bgra_layout(2, 2, 7, 16, 100),
            Err(OcrImageError::BadStride {
                stride: 7,
                width: 2
            })
        );
        assert_eq!(
            validate_bgra_layout(2, 2, 10, 20, 100),
            Err(OcrImageError::BadStride {
                stride: 10,
                width: 2
            })
        );
        assert_eq!(
            validate_bgra_layout(101, 2, 404, 808, 100),
            Err(OcrImageError::WidthTooLarge {
                width: 101,
                limit: 100
            })
        );
        assert_eq!(
            validate_bgra_layout(2, 101, 8, 808, 100),
            Err(OcrImageError::HeightTooLarge {
                height: 101,
                limit: 100
            })
        );
        // A huge (but 4-aligned) stride overflows the row arithmetic.
        assert!(matches!(
            validate_bgra_layout(2, 3, (usize::MAX / 4) * 4, 1, u32::MAX),
            Err(OcrImageError::Overflow { .. })
        ));
        assert_eq!(validate_bgra_layout(2, 2, 8, 16, 100), Ok(()));
        // The last row needs no padding after it.
        assert_eq!(validate_bgra_layout(2, 2, 12, 20, 100), Ok(()));
    }

    #[test]
    fn padded_rows_are_repacked() {
        // 2x2 image, stride 12 (4 bytes padding per row).
        let data = vec![
            1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0, //
            9, 10, 11, 12, 13, 14, 15, 16,
        ];
        let img = BgraImage::from_parts(2, 2, 12, data, 100).unwrap();
        assert_eq!(
            img.bytes(),
            &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
        );
        assert_eq!((img.width(), img.height()), (2, 2));
    }

    #[test]
    fn from_parts_never_panics_on_short_buffers() {
        for len in 0..16 {
            let r = BgraImage::from_parts(2, 2, 8, vec![0; len], 100);
            assert!(r.is_err(), "len {} accepted", len);
        }
    }

    #[test]
    fn conversion_swaps_to_bgra() {
        let mut rgba = RgbaImage::new(2, 1);
        rgba.put_pixel(0, 0, Rgba([10, 20, 30, 255]));
        rgba.put_pixel(1, 0, Rgba([40, 50, 60, 128]));
        let img = BgraImage::from_dynamic(
            &DynamicImage::ImageRgba8(rgba),
            FALLBACK_MAX_IMAGE_DIMENSION,
            OCR_PIXEL_BUDGET,
        )
        .unwrap();
        assert_eq!(img.bytes(), &[30, 20, 10, 255, 60, 50, 40, 128]);

        let rgb = RgbImage::from_pixel(3, 2, Rgb([1, 2, 3]));
        let img =
            BgraImage::from_dynamic(&DynamicImage::ImageRgb8(rgb), 100, OCR_PIXEL_BUDGET).unwrap();
        assert_eq!(img.bytes().len(), 3 * 2 * 4);
        assert_eq!(&img.bytes()[..4], &[3, 2, 1, 255]);
    }

    #[test]
    fn conversion_rejects_empty_images() {
        let empty = DynamicImage::ImageRgb8(RgbImage::new(0, 0));
        assert!(matches!(
            BgraImage::from_dynamic(&empty, 100, OCR_PIXEL_BUDGET),
            Err(OcrImageError::ZeroSize { .. })
        ));
    }

    #[test]
    fn conversion_scales_large_frames_down() {
        let big = DynamicImage::ImageRgb8(RgbImage::new(400, 100));
        let img = BgraImage::from_dynamic(&big, 200, OCR_PIXEL_BUDGET).unwrap();
        assert_eq!((img.width(), img.height()), (200, 50));
        assert_eq!(img.bytes().len(), 200 * 50 * 4);
    }

    #[test]
    fn fit_within_scales_on_the_worker_side() {
        let img = BgraImage::from_parts(40, 20, 160, vec![7; 40 * 20 * 4], 100).unwrap();
        let same = img.clone().fit_within(100, OCR_PIXEL_BUDGET).unwrap();
        assert_eq!(same, img);
        let smaller = img.fit_within(10, OCR_PIXEL_BUDGET).unwrap();
        assert_eq!((smaller.width(), smaller.height()), (10, 5));
        assert_eq!(smaller.bytes().len(), 10 * 5 * 4);
    }

    // --- worker -----------------------------------------------------------

    fn test_config(name: &str, capacity: usize) -> WorkerConfig {
        WorkerConfig {
            thread_name: name.to_string(),
            queue_capacity: capacity,
            wedge_after: Duration::from_secs(600),
            max_abandoned: 1,
        }
    }

    /// A gate the test opens to let blocked jobs finish.
    #[derive(Clone, Default)]
    struct Gate(Arc<(Mutex<bool>, Condvar)>);

    impl Gate {
        fn wait(&self) {
            let (lock, cv) = &*self.0;
            let mut open = lock.lock().unwrap();
            while !*open {
                open = cv.wait(open).unwrap();
            }
        }
        fn open(&self) {
            let (lock, cv) = &*self.0;
            *lock.lock().unwrap() = true;
            cv.notify_all();
        }
    }

    enum Job {
        Echo(u32),
        ThreadName,
        WaitFor(Gate, Arc<AtomicBool>),
        Panic,
        Sleep(Duration),
    }

    fn worker(name: &str, capacity: usize, calls: Arc<AtomicU64>) -> DedicatedWorker<Job, String> {
        let factory = handler_factory(move || {
            let calls = calls.clone();
            move |job: Job| {
                calls.fetch_add(1, Ordering::SeqCst);
                match job {
                    Job::Echo(n) => n.to_string(),
                    Job::ThreadName => std::thread::current().name().unwrap_or("").to_string(),
                    Job::WaitFor(gate, started) => {
                        started.store(true, Ordering::SeqCst);
                        gate.wait();
                        "released".to_string()
                    }
                    Job::Panic => panic!("handler exploded"),
                    Job::Sleep(d) => {
                        std::thread::sleep(d);
                        "slept".to_string()
                    }
                }
            }
        });
        DedicatedWorker::new(test_config(name, capacity), factory)
    }

    fn wait_until(what: &str, cond: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !cond() {
            assert!(Instant::now() < deadline, "timed out waiting for {}", what);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    const LONG: Duration = Duration::from_secs(10);

    #[tokio::test]
    async fn runs_jobs_on_the_named_thread() {
        let w = worker(WINDOWS_OCR_THREAD_NAME, 4, Arc::default());
        assert!(!w.is_running(), "no thread before the first request");
        assert_eq!(w.run(Job::Echo(7), LONG).await.unwrap(), "7");
        assert_eq!(
            w.run(Job::ThreadName, LONG).await.unwrap(),
            WINDOWS_OCR_THREAD_NAME
        );
        assert!(w.is_running());
        let s = w.stats();
        assert_eq!(s.threads_started, 1);
        assert_eq!(s.completed, 2);
        assert_eq!(s.restarts, 0);
        assert_eq!(s.queued, 0);
    }

    #[tokio::test]
    async fn a_full_queue_rejects_without_blocking() {
        let calls = Arc::new(AtomicU64::new(0));
        let w = worker("ocr-test-full", 2, calls.clone());
        let gate = Gate::default();
        let started = Arc::new(AtomicBool::new(false));
        let running = w
            .submit(Job::WaitFor(gate.clone(), started.clone()))
            .unwrap();
        wait_until("first job to start", || started.load(Ordering::SeqCst));

        let q1 = w.submit(Job::Echo(1)).unwrap();
        let q2 = w.submit(Job::Echo(2)).unwrap();
        let t0 = Instant::now();
        let refused = w.submit(Job::Echo(3));
        assert!(t0.elapsed() < Duration::from_secs(1));
        assert_eq!(
            refused.unwrap_err(),
            OcrWorkerError::QueueFull { capacity: 2 }
        );
        assert!(OcrWorkerError::QueueFull { capacity: 2 }.is_skip());
        assert_eq!(w.stats().queued, 2);
        assert_eq!(w.stats().rejected_queue_full, 1);

        gate.open();
        assert_eq!(running.await.unwrap().unwrap(), "released");
        assert_eq!(q1.await.unwrap().unwrap(), "1");
        assert_eq!(q2.await.unwrap().unwrap(), "2");
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!(w.stats().queued, 0);
    }

    #[tokio::test]
    async fn requests_whose_caller_left_are_dropped() {
        let calls = Arc::new(AtomicU64::new(0));
        let w = worker("ocr-test-cancel", 4, calls.clone());
        let gate = Gate::default();
        let started = Arc::new(AtomicBool::new(false));
        let running = w
            .submit(Job::WaitFor(gate.clone(), started.clone()))
            .unwrap();
        wait_until("first job to start", || started.load(Ordering::SeqCst));

        // A caller that times out goes away before its job is reached.
        let timed_out = w.run(Job::Echo(9), Duration::from_millis(20)).await;
        assert_eq!(timed_out, Err(OcrWorkerError::TimedOut { after_ms: 20 }));
        // A receiver dropped outright, too.
        drop(w.submit(Job::Echo(10)).unwrap());

        gate.open();
        running.await.unwrap().unwrap();
        assert_eq!(w.run(Job::Echo(11), LONG).await.unwrap(), "11");
        // Only the blocking job and the last echo ran.
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(w.stats().dropped_cancelled, 2);
    }

    #[tokio::test]
    async fn a_panic_errors_its_caller_and_the_thread_restarts() {
        let w = worker("ocr-test-panic", 4, Arc::default());
        assert_eq!(w.run(Job::Echo(1), LONG).await.unwrap(), "1");
        let err = w.run(Job::Panic, LONG).await.unwrap_err();
        assert!(matches!(&err, OcrWorkerError::Panicked(m) if m.contains("handler exploded")));
        assert!(err.is_retryable());

        wait_until("thread exit", || !w.is_running());
        assert_eq!(
            w.run(Job::ThreadName, LONG).await.unwrap(),
            "ocr-test-panic"
        );
        let s = w.stats();
        assert_eq!(s.panics, 1);
        assert_eq!(s.threads_started, 2);
        assert_eq!(s.restarts, 1);
    }

    #[tokio::test]
    async fn requests_queued_behind_a_crash_get_an_error_not_a_hang() {
        let w = worker("ocr-test-crash-queue", 4, Arc::default());
        let gate = Gate::default();
        let started = Arc::new(AtomicBool::new(false));
        let first = w
            .submit(Job::WaitFor(gate.clone(), started.clone()))
            .unwrap();
        wait_until("first job to start", || started.load(Ordering::SeqCst));
        let crash = w.submit(Job::Panic).unwrap();
        let behind = w.submit(Job::Echo(5)).unwrap();
        gate.open();

        assert_eq!(first.await.unwrap().unwrap(), "released");
        assert!(matches!(
            crash.await.unwrap(),
            Err(OcrWorkerError::Panicked(_))
        ));
        // The reply channel closes with the thread (run() reports that as
        // ExitedBeforeTaking) instead of leaving the caller waiting.
        let behind = tokio::time::timeout(LONG, behind)
            .await
            .expect("queued request must not hang");
        assert!(behind.is_err(), "reply channel closes with the thread");
        assert_eq!(w.run(Job::Echo(6), LONG).await.unwrap(), "6");
        assert_eq!(w.stats().queued, 0);
    }

    #[tokio::test]
    async fn a_handler_setup_panic_does_not_hang_callers() {
        let factory = handler_factory(|| {
            if std::hint::black_box(true) {
                panic!("no engine on this thread");
            }
            |x: u32| x
        });
        let w = DedicatedWorker::new(test_config("ocr-test-setup", 2), factory);
        let err = w.run(1, LONG).await.unwrap_err();
        assert_eq!(err, OcrWorkerError::ExitedBeforeTaking);
        // Repeated failures run into the restart limit instead of spinning.
        let mut last = None;
        for _ in 0..(MAX_RESTARTS_PER_WINDOW + 3) {
            last = Some(w.run(1, LONG).await.unwrap_err());
        }
        assert!(matches!(last, Some(OcrWorkerError::StartFailed(_))));
        assert!(w.stats().threads_started <= (MAX_RESTARTS_PER_WINDOW + 1) as u64);
    }

    /// A job for [`stuck_worker`]: `Some` blocks on the gate (raising the
    /// flag once it runs), `None` answers at once.
    type StuckJob = Option<(Gate, Arc<AtomicBool>)>;

    const STUCK_AFTER: Duration = Duration::from_millis(50);

    fn stuck_worker(name: &str) -> DedicatedWorker<StuckJob, &'static str> {
        let mut config = test_config(name, 4);
        config.wedge_after = STUCK_AFTER;
        config.max_abandoned = 1;
        let factory = handler_factory(|| {
            |job: StuckJob| match job {
                Some((gate, started)) => {
                    started.store(true, Ordering::SeqCst);
                    gate.wait();
                    "unstuck"
                }
                None => "quick",
            }
        });
        DedicatedWorker::new(config, factory)
    }

    /// Submits a blocking job and returns once the thread is running it and
    /// has been busy for longer than [`STUCK_AFTER`].
    fn submit_stuck(
        w: &DedicatedWorker<StuckJob, &'static str>,
        gate: &Gate,
    ) -> ReplyRx<&'static str> {
        let started = Arc::new(AtomicBool::new(false));
        let rx = w.submit(Some((gate.clone(), started.clone()))).unwrap();
        wait_until("blocking job to start", || started.load(Ordering::SeqCst));
        // The busy mark is stored before the handler runs, so from here the
        // thread counts as stuck once STUCK_AFTER has passed.
        std::thread::sleep(STUCK_AFTER * 2);
        rx
    }

    #[tokio::test]
    async fn a_stuck_thread_is_replaced_up_to_the_limit() {
        let w = stuck_worker("ocr-test-stuck");

        let g1 = Gate::default();
        let stuck = submit_stuck(&w, &g1);

        // The stuck thread is left behind; a fresh one answers.
        assert_eq!(w.run(None, LONG).await.unwrap(), "quick");
        assert_eq!(w.stats().abandoned_stuck, 1);
        assert_eq!(w.stats().threads_started, 2);

        // The replacement gets stuck too, and the limit of one abandoned
        // thread is reached: report it instead of leaking more threads.
        let g2 = Gate::default();
        let stuck2 = submit_stuck(&w, &g2);
        assert!(matches!(
            w.submit(None).unwrap_err(),
            OcrWorkerError::Stuck { .. }
        ));

        // Once the first one finishes and exits, a replacement is allowed.
        g1.open();
        assert_eq!(stuck.await.unwrap().unwrap(), "unstuck");
        wait_until("abandoned thread exit", || {
            w.abandoned_live.load(Ordering::SeqCst) == 0
        });
        assert_eq!(w.run(None, LONG).await.unwrap(), "quick");
        g2.open();
        assert_eq!(stuck2.await.unwrap().unwrap(), "unstuck");
    }

    #[tokio::test]
    async fn requests_queued_behind_a_stuck_thread_are_released() {
        let w = stuck_worker("ocr-test-stuck-queue");
        let gate = Gate::default();
        let started = Arc::new(AtomicBool::new(false));
        let stuck = w.submit(Some((gate.clone(), started.clone()))).unwrap();
        wait_until("blocking job to start", || started.load(Ordering::SeqCst));
        let mut behind = w.submit(None).unwrap();
        assert_eq!(w.stats().queued, 1);
        std::thread::sleep(STUCK_AFTER * 2);

        // The next request abandons the stuck thread; the one queued behind
        // it is answered (and its job freed) before that request returns.
        assert_eq!(w.run(None, LONG).await.unwrap(), "quick");
        assert_eq!(
            behind.try_recv().expect("queued request answered at once"),
            Err(OcrWorkerError::ExitedBeforeTaking)
        );
        assert_eq!(w.stats().queued, 0);

        // The left-behind thread still answers its own caller and exits.
        gate.open();
        assert_eq!(stuck.await.unwrap().unwrap(), "unstuck");
        wait_until("abandoned thread exit", || {
            w.abandoned_live.load(Ordering::SeqCst) == 0
        });
    }

    #[tokio::test]
    async fn sleeping_jobs_finish_in_order() {
        let w = worker("ocr-test-order", 4, Arc::default());
        let a = w.submit(Job::Sleep(Duration::from_millis(10))).unwrap();
        let b = w.submit(Job::Echo(2)).unwrap();
        assert_eq!(a.await.unwrap().unwrap(), "slept");
        assert_eq!(b.await.unwrap().unwrap(), "2");
    }

    /// The Windows OCR worker is a `Lazy` static with image jobs and text
    /// replies (`microsoft.rs`, compiled only on Windows); prove the same
    /// shape is `Send + Sync` here so Linux builds catch a regression.
    #[test]
    fn a_worker_with_image_jobs_can_live_in_a_static() {
        #[allow(dead_code)]
        enum ImageJob {
            Recognize {
                image: BgraImage,
                languages: Vec<String>,
            },
            Probe,
        }
        #[allow(dead_code)]
        enum TextReply {
            Text(Result<(String, String, Option<f64>), OcrWorkerError>),
            Probe(Vec<String>),
        }
        fn static_ready<T: Send + Sync + 'static>() {}
        static_ready::<DedicatedWorker<ImageJob, TextReply>>();
        static_ready::<InflightGate>();

        static WORKER: once_cell::sync::Lazy<DedicatedWorker<ImageJob, TextReply>> =
            once_cell::sync::Lazy::new(|| {
                DedicatedWorker::new(
                    test_config("ocr-test-static", 1),
                    handler_factory(|| |_job: ImageJob| TextReply::Probe(Vec::new())),
                )
            });
        assert!(!WORKER.is_running());
    }

    // --- inflight gate ----------------------------------------------------

    #[test]
    fn inflight_gate_limits_and_releases() {
        let gate = InflightGate::new(2);
        let a = gate.try_enter().unwrap();
        assert_eq!(a.inflight(), 1);
        let b = gate.try_enter().unwrap();
        assert_eq!(b.inflight(), 2);
        assert!(gate.try_enter().is_none());
        drop(a);
        assert_eq!(gate.current(), 1);
        let c = gate.try_enter().unwrap();
        assert_eq!(c.inflight(), 2);
        drop(b);
        drop(c);
        assert_eq!(gate.current(), 0);
    }

    #[test]
    fn worker_errors_are_found_in_anyhow_chains() {
        let e = anyhow::Error::new(OcrWorkerError::QueueFull { capacity: 4 }).context("live OCR");
        assert_eq!(
            worker_error_of(&e),
            Some(&OcrWorkerError::QueueFull { capacity: 4 })
        );
        assert!(worker_error_of(&anyhow::anyhow!("plain")).is_none());
        assert!(OcrWorkerError::Unavailable("x".into()).is_skip());
        assert!(!OcrWorkerError::Engine("x".into()).is_skip());
        assert!(!OcrWorkerError::Unavailable("x".into()).is_retryable());
    }
}
