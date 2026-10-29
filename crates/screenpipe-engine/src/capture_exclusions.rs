// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Window-exclusion probe for the event-driven capture loop.
//!
//! On platforms whose capture backend can leave single windows out of a
//! monitor capture (ScreenCaptureKit on macOS), every screenshot needs the
//! current list of window ids that the user's ignore/include rules exclude.
//! Building that list enumerates every on-screen window, which can hang when
//! the window server stops answering.
//!
//! [`ExclusionProbe`] runs the enumeration on the blocking pool under a
//! deadline, so a hung enumeration never stalls the capture loop, and it
//! fails closed: when a refresh fails or overruns its deadline the exclusion
//! state becomes [`ExclusionState::Unknown`], and the loop takes no frame
//! pixels at all (no snapshot, no OCR, no change probe) until a later refresh
//! succeeds. Accessibility capture carries on meanwhile. A stale list is never
//! reused for pixels, because a window that appeared since the last good
//! refresh could be one the user asked to keep out.
//!
//! Where the backend cannot exclude windows (Windows, Linux) or no rule names
//! anything to exclude, the list is empty by construction and the state is
//! always known; ignored and private windows are then kept out by the
//! accessibility-based window gate in `do_capture`.

use screenpipe_a11y::tree::TreeWalkerConfig;
use screenpipe_a11y::url_filter::UrlPolicy;
use screenpipe_screen::capture_screenshot_by_window::{
    try_get_excluded_sck_window_ids, window_filters_are_empty, WindowFilters,
    WINDOW_ID_EXCLUSION_SUPPORTED,
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// Longest a window-exclusion refresh may take before its result is treated
/// as unknown.
pub const EXCLUSION_REFRESH_TIMEOUT: Duration = Duration::from_secs(2);

/// Why the exclusion list is not known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnknownReason {
    /// No refresh has completed yet.
    NotRefreshed,
    /// The window enumeration reported an error.
    Failed(String),
    /// The enumeration task panicked or was cancelled.
    TaskFailed(String),
    /// The enumeration did not answer within the deadline.
    TimedOut(Duration),
    /// An earlier enumeration that overran its deadline is still running, so
    /// no new one was started.
    PreviousStillRunning,
}

impl std::fmt::Display for UnknownReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotRefreshed => f.write_str("no refresh has completed yet"),
            Self::Failed(e) => write!(f, "window enumeration failed: {}", e),
            Self::TaskFailed(e) => write!(f, "window enumeration task failed: {}", e),
            Self::TimedOut(d) => write!(f, "window enumeration gave no answer within {:?}", d),
            Self::PreviousStillRunning => {
                f.write_str("an earlier window enumeration is still hanging")
            }
        }
    }
}

/// What the capture loop knows about the windows to keep out of frame pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExclusionState {
    /// Current list of window ids to exclude (sorted, without duplicates).
    Known(Vec<u32>),
    /// The list could not be determined; frame pixels must not be taken.
    Unknown(UnknownReason),
}

impl ExclusionState {
    /// The ids to pass to the capture call, or `None` when pixels must be
    /// withheld.
    pub fn pixel_exclusions(&self) -> Option<&[u32]> {
        match self {
            Self::Known(ids) => Some(ids),
            Self::Unknown(_) => None,
        }
    }
}

type Enumerate = Arc<dyn Fn() -> Result<Vec<u32>, String> + Send + Sync>;
type PendingEnumeration = JoinHandle<Result<Vec<u32>, String>>;

/// Per-monitor exclusion list with a bounded, fail-closed refresh.
pub struct ExclusionProbe {
    monitor_id: u32,
    /// `None` when there is nothing to enumerate (the list is always empty).
    enumerate: Option<Enumerate>,
    timeout: Duration,
    state: ExclusionState,
    /// An enumeration that overran its deadline and has not finished. At most
    /// one is outstanding per probe, so a hung window server ties up one
    /// blocking thread, not one per capture.
    pending: Option<PendingEnumeration>,
    /// When the current unknown period started (for the recovery log).
    unknown_since: Option<Instant>,
    /// Refreshes that left the state unknown in the current unknown period.
    failed_refreshes: u64,
}

impl ExclusionProbe {
    /// Probe for a capture loop with the given window rules.
    pub fn for_tree_walker_config(monitor_id: u32, config: &TreeWalkerConfig) -> Self {
        Self::for_rules(
            monitor_id,
            &config.ignored_windows,
            &config.included_windows,
            &config.ignored_urls,
            &config.url_policy,
        )
    }

    /// Probe for the given ignore, include and URL rules. With a website
    /// allow list in `url_policy`, unfocused browser windows are excluded
    /// too: their URL cannot be checked (see
    /// `screenpipe_screen::capture_screenshot_by_window::window_excluded_from_capture`).
    pub fn for_rules(
        monitor_id: u32,
        ignored_windows: &[String],
        included_windows: &[String],
        ignored_urls: &[String],
        url_policy: &UrlPolicy,
    ) -> Self {
        let filters = WindowFilters::new(ignored_windows, included_windows, ignored_urls)
            .with_url_policy(url_policy.clone());
        if !WINDOW_ID_EXCLUSION_SUPPORTED || window_filters_are_empty(&filters) {
            return Self::without_exclusions(monitor_id);
        }
        let filters = Arc::new(filters);
        Self::with_source(
            monitor_id,
            move || try_get_excluded_sck_window_ids(&filters),
            EXCLUSION_REFRESH_TIMEOUT,
        )
    }

    /// Probe whose list is always empty and always known.
    pub fn without_exclusions(monitor_id: u32) -> Self {
        Self {
            monitor_id,
            enumerate: None,
            timeout: EXCLUSION_REFRESH_TIMEOUT,
            state: ExclusionState::Known(Vec::new()),
            pending: None,
            unknown_since: None,
            failed_refreshes: 0,
        }
    }

    /// Probe that enumerates with `source` under `timeout`. The state starts
    /// unknown, so nothing is captured before the first refresh.
    pub fn with_source(
        monitor_id: u32,
        source: impl Fn() -> Result<Vec<u32>, String> + Send + Sync + 'static,
        timeout: Duration,
    ) -> Self {
        Self {
            monitor_id,
            enumerate: Some(Arc::new(source)),
            timeout,
            state: ExclusionState::Unknown(UnknownReason::NotRefreshed),
            pending: None,
            unknown_since: Some(Instant::now()),
            failed_refreshes: 0,
        }
    }

    /// Current state, as left by the last refresh.
    pub fn state(&self) -> &ExclusionState {
        &self.state
    }

    /// True when the list never needs an enumeration.
    pub fn is_static(&self) -> bool {
        self.enumerate.is_none()
    }

    /// Re-enumerate the excluded windows. Never waits longer than the
    /// deadline; on failure, timeout or a still-hanging earlier enumeration
    /// the state becomes unknown.
    pub async fn refresh(&mut self) -> &ExclusionState {
        let Some(enumerate) = self.enumerate.clone() else {
            return &self.state;
        };
        if let Some(previous) = self.pending.take() {
            if !previous.is_finished() {
                self.pending = Some(previous);
                self.mark_unknown(UnknownReason::PreviousStillRunning);
                return &self.state;
            }
            // It finished after its deadline; that answer is stale by now.
        }

        let mut task = tokio::task::spawn_blocking(move || enumerate());
        match tokio::time::timeout(self.timeout, &mut task).await {
            Ok(Ok(Ok(mut ids))) => {
                ids.sort_unstable();
                ids.dedup();
                self.mark_known(ids);
            }
            Ok(Ok(Err(e))) => self.mark_unknown(UnknownReason::Failed(e)),
            Ok(Err(join)) => self.mark_unknown(UnknownReason::TaskFailed(join.to_string())),
            Err(_) => {
                self.pending = Some(task);
                self.mark_unknown(UnknownReason::TimedOut(self.timeout));
            }
        }
        &self.state
    }

    fn mark_known(&mut self, ids: Vec<u32>) {
        if let Some(since) = self.unknown_since.take() {
            if self.failed_refreshes > 0 {
                info!(
                    monitor_id = self.monitor_id,
                    "window exclusion list for monitor {} is known again after {:.1}s \
                     ({} refresh(es) without an answer); frame pixels are captured again",
                    self.monitor_id,
                    since.elapsed().as_secs_f64(),
                    self.failed_refreshes
                );
            }
        }
        self.failed_refreshes = 0;
        if !matches!(&self.state, ExclusionState::Known(current) if *current == ids) {
            debug!(
                "monitor {}: {} window(s) excluded from capture",
                self.monitor_id,
                ids.len()
            );
        }
        self.state = ExclusionState::Known(ids);
    }

    fn mark_unknown(&mut self, reason: UnknownReason) {
        self.failed_refreshes += 1;
        if self.unknown_since.is_none() {
            self.unknown_since = Some(Instant::now());
        }
        if self.failed_refreshes == 1 {
            warn!(
                monitor_id = self.monitor_id,
                "window exclusion list for monitor {} is unknown ({}); frame pixels are \
                 withheld until a refresh succeeds, accessibility capture continues",
                self.monitor_id,
                reason
            );
        } else {
            debug!(
                "monitor {}: window exclusion list still unknown ({}), refresh #{}",
                self.monitor_id, reason, self.failed_refreshes
            );
        }
        self.state = ExclusionState::Unknown(reason);
    }
}

impl Drop for ExclusionProbe {
    fn drop(&mut self) {
        // A blocking enumeration cannot be interrupted; dropping the handle
        // lets it finish on its own and discards the answer.
        self.pending.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::sync::Mutex;

    /// A source that blocks until the test releases it (dropping the sender),
    /// standing in for a window server that stopped answering.
    fn hanging_source() -> (
        impl Fn() -> Result<Vec<u32>, String> + Send + Sync + 'static,
        mpsc::Sender<()>,
        Arc<AtomicUsize>,
    ) {
        let (tx, rx) = mpsc::channel::<()>();
        let rx = Arc::new(Mutex::new(rx));
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let source = move || {
            counter.fetch_add(1, Ordering::SeqCst);
            let _ = rx
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(30));
            Ok(vec![7])
        };
        (source, tx, calls)
    }

    #[tokio::test]
    async fn a_successful_refresh_sorts_and_dedups() {
        let mut probe =
            ExclusionProbe::with_source(1, || Ok(vec![9, 3, 9, 1]), EXCLUSION_REFRESH_TIMEOUT);
        assert!(
            probe.state().pixel_exclusions().is_none(),
            "unknown before the first refresh"
        );
        let state = probe.refresh().await.clone();
        assert_eq!(state, ExclusionState::Known(vec![1, 3, 9]));
        assert_eq!(probe.state().pixel_exclusions(), Some(&[1, 3, 9][..]));
    }

    #[tokio::test]
    async fn a_failed_enumeration_fails_closed() {
        let mut probe = ExclusionProbe::with_source(
            1,
            || Err("window server unavailable".to_string()),
            EXCLUSION_REFRESH_TIMEOUT,
        );
        let state = probe.refresh().await.clone();
        assert!(matches!(
            state,
            ExclusionState::Unknown(UnknownReason::Failed(_))
        ));
        assert!(probe.state().pixel_exclusions().is_none());
    }

    #[tokio::test]
    async fn a_stale_list_is_not_reused_after_a_failure() {
        let fail = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = fail.clone();
        let mut probe = ExclusionProbe::with_source(
            1,
            move || {
                if flag.load(Ordering::SeqCst) {
                    Err("enumeration failed".into())
                } else {
                    Ok(vec![4])
                }
            },
            EXCLUSION_REFRESH_TIMEOUT,
        );
        assert_eq!(probe.refresh().await, &ExclusionState::Known(vec![4]));
        fail.store(true, Ordering::SeqCst);
        assert!(probe.refresh().await.pixel_exclusions().is_none());
        fail.store(false, Ordering::SeqCst);
        assert_eq!(probe.refresh().await, &ExclusionState::Known(vec![4]));
    }

    #[tokio::test]
    async fn a_hung_enumeration_times_out_without_stalling_and_is_not_restarted() {
        let (source, release, calls) = hanging_source();
        let mut probe =
            ExclusionProbe::with_source(2, source, std::time::Duration::from_millis(100));

        let started = Instant::now();
        let state = probe.refresh().await.clone();
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        assert!(matches!(
            state,
            ExclusionState::Unknown(UnknownReason::TimedOut(_))
        ));

        // While the first enumeration still hangs, no second one starts.
        let state = probe.refresh().await.clone();
        assert_eq!(
            state,
            ExclusionState::Unknown(UnknownReason::PreviousStillRunning)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Once it gives up, the next refresh enumerates afresh.
        drop(release);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !probe.pending.as_ref().is_some_and(|p| p.is_finished()) {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the hung enumeration returns once released");
        let state = probe.refresh().await.clone();
        assert_eq!(state, ExclusionState::Known(vec![7]));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn without_exclusions_is_always_known_and_never_enumerates() {
        let mut probe = ExclusionProbe::without_exclusions(3);
        assert!(probe.is_static());
        assert_eq!(probe.refresh().await, &ExclusionState::Known(vec![]));
    }

    #[test]
    fn no_rules_means_nothing_to_enumerate() {
        let config = TreeWalkerConfig::default();
        let probe = ExclusionProbe::for_tree_walker_config(1, &config);
        assert!(probe.is_static());
        assert_eq!(probe.state(), &ExclusionState::Known(vec![]));
    }

    #[test]
    fn an_allow_list_alone_needs_an_enumeration_where_windows_can_be_excluded() {
        let config = TreeWalkerConfig {
            url_policy: UrlPolicy::new(
                &[],
                &[screenpipe_a11y::url_filter::DomainRule {
                    domain: "example.org".to_string(),
                    include_subdomains: true,
                    excluded_subdomains: vec![],
                }],
            ),
            ..TreeWalkerConfig::default()
        };
        let probe = ExclusionProbe::for_tree_walker_config(1, &config);
        assert_eq!(probe.is_static(), !WINDOW_ID_EXCLUSION_SUPPORTED);
    }

    #[test]
    fn rules_need_an_enumeration_only_where_windows_can_be_excluded() {
        let config = TreeWalkerConfig {
            ignored_windows: vec!["1Password".to_string()],
            ..TreeWalkerConfig::default()
        };
        let probe = ExclusionProbe::for_tree_walker_config(1, &config);
        assert_eq!(probe.is_static(), !WINDOW_ID_EXCLUSION_SUPPORTED);
    }
}
