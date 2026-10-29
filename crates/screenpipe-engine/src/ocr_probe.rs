// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Probes once at engine start whether the platform OCR engine can run
//! (Windows recognizer languages, tesseract elsewhere) and publishes the
//! answer, so `/health` reports `vision_reason = ocr_unavailable` from the
//! start instead of only after the first OCR attempt. Capture is not touched:
//! without OCR, frames and accessibility text are still recorded.

use crate::core::{WorkerContext, WorkerFuture};

/// Background worker entry (see [`crate::core::BACKGROUND_WORKERS`]).
pub fn start_worker(ctx: WorkerContext) -> WorkerFuture {
    Box::pin(async move {
        tokio::select! {
            _ = ctx.shutdown.cancelled() => {}
            _ = screenpipe_screen::ocr_availability::probe_and_publish() => {}
        }
    })
}
