// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Visual-change probe for the event-driven capture loop.
//!
//! Between input events the loop takes a cheap screenshot now and then and
//! diffs it against the last one to notice changes that came without input
//! (video, notifications, a build finishing). That screenshot used to run
//! inline, so a capture backend that stopped answering (ScreenCaptureKit
//! waiting on the window server, a wedged Graphics Capture session) froze the
//! whole loop.
//!
//! [`VisualProbe`] takes the screenshot in its own task under a deadline. When
//! the deadline passes the check is skipped and the loop moves on; the
//! late task is kept, and no second probe starts while it is still running,
//! so a hung backend costs one task per monitor rather than one per check.

use image::DynamicImage;
use screenpipe_screen::monitor::SafeMonitor;
use screenpipe_screen::utils::capture_monitor_image;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// Longest a change-probe screenshot may take before the check is skipped.
pub const VISUAL_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Result of one probe.
#[derive(Debug)]
pub enum ProbeOutcome {
    /// The screenshot, ready to diff.
    Image(DynamicImage),
    /// The backend answered with an error.
    Failed(String),
    /// No answer within the deadline; this check is skipped.
    TimedOut,
    /// An earlier probe that overran its deadline is still running, so this
    /// check is skipped without starting another.
    Busy,
}

type PendingProbe = JoinHandle<anyhow::Result<DynamicImage>>;

/// Per-monitor change probe with a deadline.
pub struct VisualProbe {
    monitor_id: u32,
    timeout: Duration,
    pending: Option<PendingProbe>,
    /// Probes skipped in the current run of timeouts.
    unanswered: u64,
}

impl VisualProbe {
    pub fn new(monitor_id: u32) -> Self {
        Self::with_timeout(monitor_id, VISUAL_PROBE_TIMEOUT)
    }

    pub fn with_timeout(monitor_id: u32, timeout: Duration) -> Self {
        Self {
            monitor_id,
            timeout,
            pending: None,
            unanswered: 0,
        }
    }

    /// Screenshot `monitor`, leaving out `excluded` window ids.
    pub async fn screenshot(
        &mut self,
        monitor: &Arc<SafeMonitor>,
        excluded: &[u32],
    ) -> ProbeOutcome {
        let monitor = monitor.clone();
        let excluded = excluded.to_vec();
        self.run(async move {
            capture_monitor_image(&monitor, &excluded)
                .await
                .map(|(image, _)| image)
        })
        .await
    }

    /// Run one probe future under the deadline.
    pub async fn run<F>(&mut self, probe: F) -> ProbeOutcome
    where
        F: Future<Output = anyhow::Result<DynamicImage>> + Send + 'static,
    {
        if let Some(previous) = self.pending.take() {
            if !previous.is_finished() {
                self.pending = Some(previous);
                self.unanswered += 1;
                debug!(
                    "monitor {}: change probe skipped, the previous one is still running",
                    self.monitor_id
                );
                return ProbeOutcome::Busy;
            }
            // It finished after its deadline; its image is stale by now.
        }

        let mut task = tokio::spawn(probe);
        match tokio::time::timeout(self.timeout, &mut task).await {
            Ok(joined) => {
                if self.unanswered > 0 {
                    info!(
                        monitor_id = self.monitor_id,
                        "change probe on monitor {} answers again after {} skipped check(s)",
                        self.monitor_id,
                        self.unanswered
                    );
                    self.unanswered = 0;
                }
                match joined {
                    Ok(Ok(image)) => ProbeOutcome::Image(image),
                    Ok(Err(e)) => ProbeOutcome::Failed(e.to_string()),
                    Err(e) => ProbeOutcome::Failed(format!("probe task failed: {}", e)),
                }
            }
            Err(_) => {
                self.pending = Some(task);
                self.unanswered += 1;
                if self.unanswered == 1 {
                    warn!(
                        monitor_id = self.monitor_id,
                        "change probe on monitor {} gave no screenshot within {:?} (capture \
                         backend not responding); skipping the check, the capture loop keeps \
                         running",
                        self.monitor_id,
                        self.timeout
                    );
                } else {
                    debug!(
                        "monitor {}: change probe timed out again ({} in a row)",
                        self.monitor_id, self.unanswered
                    );
                }
                ProbeOutcome::TimedOut
            }
        }
    }
}

impl Drop for VisualProbe {
    fn drop(&mut self) {
        if let Some(task) = self.pending.take() {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image() -> DynamicImage {
        DynamicImage::new_rgb8(4, 4)
    }

    #[tokio::test]
    async fn a_prompt_probe_returns_its_image() {
        let mut probe = VisualProbe::new(1);
        let outcome = probe.run(async { Ok(image()) }).await;
        assert!(matches!(outcome, ProbeOutcome::Image(_)));
        let outcome = probe.run(async { Err(anyhow::anyhow!("no frame")) }).await;
        assert!(matches!(outcome, ProbeOutcome::Failed(e) if e.contains("no frame")));
    }

    #[tokio::test]
    async fn a_hung_probe_is_skipped_and_not_stacked() {
        let mut probe = VisualProbe::with_timeout(1, Duration::from_millis(50));
        let (release, wait) = tokio::sync::oneshot::channel::<()>();
        let started = std::time::Instant::now();
        let outcome = probe
            .run(async move {
                let _ = wait.await;
                Ok(image())
            })
            .await;
        assert!(matches!(outcome, ProbeOutcome::TimedOut));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the loop is not held"
        );

        // Still hanging: the next check is skipped without a second task.
        let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = ran.clone();
        let outcome = probe
            .run(async move {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(image())
            })
            .await;
        assert!(matches!(outcome, ProbeOutcome::Busy));
        assert!(!ran.load(std::sync::atomic::Ordering::SeqCst));

        // Once the hung probe returns, probing resumes.
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !probe.pending.as_ref().is_some_and(|p| p.is_finished()) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let outcome = probe.run(async { Ok(image()) }).await;
        assert!(matches!(outcome, ProbeOutcome::Image(_)));
        assert_eq!(probe.unanswered, 0);
    }

    #[tokio::test]
    async fn dropping_the_probe_aborts_a_hung_task() {
        let mut probe = VisualProbe::with_timeout(1, Duration::from_millis(20));
        let (_release, wait) = tokio::sync::oneshot::channel::<()>();
        let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel::<()>();
        let outcome = probe
            .run(async move {
                let _notify_on_drop = dropped_tx;
                let _ = wait.await;
                Ok(image())
            })
            .await;
        assert!(matches!(outcome, ProbeOutcome::TimedOut));
        drop(probe);
        let ended = tokio::time::timeout(Duration::from_secs(5), dropped_rx).await;
        assert!(
            matches!(ended, Ok(Err(_))),
            "the hung probe task was aborted"
        );
    }
}
