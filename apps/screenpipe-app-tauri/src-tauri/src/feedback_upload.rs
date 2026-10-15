// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! "Send logs & feedback", without sending anything.
//!
//! The command names match the ones the reference app exposes so the
//! frontend contract stays familiar, but the behaviour is local:
//!
//! - [`start_feedback_upload`] starts a background job that writes a scrubbed
//!   zip to `<data dir>/feedback/<YYYYMMDD-HHMMSS>.zip` and then emits
//!   `feedback-upload-completed` with `{ jobId, sent: false, path, ... }` and a
//!   prefilled GitHub issue link. Nothing is uploaded; the user decides
//!   whether to attach the zip to an issue.
//! - [`redact_pii_for_feedback`] runs the same local scrubber on any text.
//!
//! See `bundle.rs` for what goes into the zip and `redact.rs` for the
//! scrubbing rules.

mod bundle;
mod issue;
pub mod redact;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tracing::{info, warn};

use crate::log_files::{get_data_dir, get_screenpipe_data_dir};
use bundle::{BundleInput, BundleSummary, FEEDBACK_DIR, KEEP_BUNDLES};
use issue::IssueDetails;
use redact::FeedbackRedactor;

/// Event emitted when a job finishes, successfully or not.
pub const COMPLETED_EVENT: &str = "feedback-upload-completed";

/// Only one bundle is written at a time.
static JOB_RUNNING: AtomicBool = AtomicBool::new(false);

/// What the UI hands over. Every field is optional on the wire.
#[derive(Debug, Clone, Default, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackUploadRequest {
    /// What the user wrote.
    #[serde(default)]
    pub feedback_text: String,
    /// Where the report was started: `feedback`, `server-not-running`,
    /// `onboarding-stuck`, ...
    #[serde(default)]
    pub source: Option<String>,
    /// Recent browser console output kept by the frontend.
    #[serde(default)]
    pub console_log: Option<String>,
    /// Recent chat history, when the user opted in.
    #[serde(default)]
    pub chat_history: Option<String>,
    /// `data:image/(png|jpeg|webp);base64,...`
    #[serde(default)]
    pub screenshot_data_url: Option<String>,
    /// A local `.mp4`/`.mov` screen recording (50 MB at most).
    #[serde(default)]
    pub video_path: Option<String>,
    /// Masked settings snapshot from the UI. Only used when the settings
    /// store cannot be read; it is masked again either way.
    #[serde(default)]
    pub app_settings: Option<Value>,
}

/// Returned as soon as the job is queued.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackUploadStarted {
    pub job_id: String,
}

/// Payload of [`COMPLETED_EVENT`].
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackUploadCompleted {
    pub job_id: String,
    /// Always false: nothing leaves the machine.
    pub sent: bool,
    /// Absolute path of the zip, when one was written.
    pub path: Option<String>,
    pub file_name: Option<String>,
    pub size_bytes: Option<u64>,
    /// Prefilled `issues/new` link for the Freenpipe tracker.
    pub issue_url: Option<String>,
    /// Set when the bundle could not be written.
    pub error: Option<String>,
}

impl FeedbackUploadCompleted {
    fn success(job_id: &str, summary: &BundleSummary, issue_url: String) -> Self {
        Self {
            job_id: job_id.to_string(),
            sent: false,
            path: Some(summary.path.to_string_lossy().into_owned()),
            file_name: Some(summary.file_name.clone()),
            size_bytes: Some(summary.size_bytes),
            issue_url: Some(issue_url),
            error: None,
        }
    }

    fn failure(job_id: &str, error: String) -> Self {
        Self {
            job_id: job_id.to_string(),
            sent: false,
            path: None,
            file_name: None,
            size_bytes: None,
            issue_url: None,
            error: Some(error),
        }
    }
}

struct JobGuard;

impl JobGuard {
    fn acquire() -> Option<Self> {
        JOB_RUNNING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| JobGuard)
    }
}

impl Drop for JobGuard {
    fn drop(&mut self) {
        JOB_RUNNING.store(false, Ordering::Release);
    }
}

/// Write a local diagnostics bundle in the background. Returns `{ jobId }`
/// immediately; the result arrives as `feedback-upload-completed`.
#[tauri::command]
#[specta::specta]
pub async fn start_feedback_upload(
    app: AppHandle,
    request: FeedbackUploadRequest,
) -> Result<FeedbackUploadStarted, String> {
    let guard = JobGuard::acquire()
        .ok_or_else(|| "a diagnostics bundle is already being written".to_string())?;
    let job_id = uuid::Uuid::new_v4().to_string();
    let started = FeedbackUploadStarted {
        job_id: job_id.clone(),
    };
    tauri::async_runtime::spawn(async move {
        let _guard = guard;
        let outcome = run_job(&app, &job_id, request).await;
        match &outcome.error {
            None => info!(
                "diagnostics bundle written: {} ({} bytes)",
                outcome.file_name.as_deref().unwrap_or("?"),
                outcome.size_bytes.unwrap_or(0)
            ),
            Some(e) => warn!("diagnostics bundle failed: {e}"),
        }
        if let Err(e) = app.emit(COMPLETED_EVENT, &outcome) {
            warn!("could not emit {COMPLETED_EVENT}: {e}");
        }
    });
    Ok(started)
}

/// Scrub `text` with the local redactor. `settingsJson` (the app settings
/// as a JSON string) adds the user's `piiRedactionLabels` and their own
/// name and e-mail to what is removed.
#[tauri::command]
#[specta::specta]
pub async fn redact_pii_for_feedback(
    text: String,
    settings_json: Option<String>,
) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let settings = redact::parse_settings_json(settings_json.as_deref());
        let redactor = FeedbackRedactor::new(settings.as_ref(), &machine_values());
        redactor.redact(&text)
    })
    .await
    .map_err(|e| e.to_string())
}

/// Values tied to this machine that should not appear in a bundle.
fn machine_values() -> Vec<(String, &'static str)> {
    let mut values = Vec::new();
    let host = tauri_plugin_os::hostname();
    // Short or generic host names would match ordinary words.
    if host.chars().count() >= 4 && !host.eq_ignore_ascii_case("localhost") {
        values.push((host, "[HOST]"));
    }
    values
}

/// Settings as stored on disk, unmasked (masking happens when written).
fn stored_settings(app: &AppHandle) -> Option<Value> {
    crate::store::get_store(app, None)
        .ok()
        .and_then(|store| store.get("settings"))
        .filter(Value::is_object)
}

async fn run_job(app: &AppHandle, job_id: &str, request: FeedbackUploadRequest) -> FeedbackUploadCompleted {
    let data_dir = match get_screenpipe_data_dir(app) {
        Ok(dir) => dir,
        Err(e) => return FeedbackUploadCompleted::failure(job_id, format!("no data directory: {e}")),
    };
    let extra_log_dirs: Vec<PathBuf> = get_data_dir(app)
        .ok()
        .filter(|dir| dir != &data_dir)
        .into_iter()
        .collect();

    let stored = stored_settings(app);
    let redactor = FeedbackRedactor::new(stored.as_ref(), &machine_values());
    let settings = stored.or_else(|| request.app_settings.clone());
    let health = health_snapshot(app).await;

    let app_version = app.package_info().version.to_string();
    let os_platform = tauri_plugin_os::platform().to_string();
    let os_version = tauri_plugin_os::version().to_string();
    let os_arch = tauri_plugin_os::arch().to_string();
    let source = request
        .source
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("feedback")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(48)
        .collect::<String>();

    let input = BundleInput {
        data_dir: data_dir.clone(),
        extra_log_dirs,
        created_at: chrono::Local::now(),
        app_version: app_version.clone(),
        os_platform: os_platform.clone(),
        os_version: os_version.clone(),
        os_arch: os_arch.clone(),
        source: source.clone(),
        feedback_text: request.feedback_text.clone(),
        console_log: request.console_log.clone(),
        chat_history: request.chat_history.clone(),
        screenshot_data_url: request.screenshot_data_url.clone(),
        video_path: request
            .video_path
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(PathBuf::from),
        settings,
        health,
    };

    let written = tokio::task::spawn_blocking(move || {
        let summary = bundle::write_bundle(&input, &redactor)?;
        let pruned = bundle::prune_bundles(&input.data_dir.join(FEEDBACK_DIR), KEEP_BUNDLES);
        if !pruned.is_empty() {
            info!("removed {} old diagnostics bundle(s)", pruned.len());
        }
        // Scrubbed copy of the feedback for the issue body.
        let feedback = redactor.redact(input.feedback_text.trim());
        Ok::<_, std::io::Error>((summary, feedback))
    })
    .await;

    match written {
        Ok(Ok((summary, feedback))) => {
            let issue_url = issue::new_issue_url(&IssueDetails {
                feedback: &feedback,
                app_version: &app_version,
                os_platform: &os_platform,
                os_version: &os_version,
                os_arch: &os_arch,
                source: &source,
                bundle_file_name: Some(&summary.file_name),
            });
            FeedbackUploadCompleted::success(job_id, &summary, issue_url)
        }
        Ok(Err(e)) => FeedbackUploadCompleted::failure(job_id, format!("could not write the bundle: {e}")),
        Err(e) => FeedbackUploadCompleted::failure(job_id, format!("bundle job stopped: {e}")),
    }
}

/// Engine and recording state at the moment the bundle is made. The engine
/// is asked over loopback with a short timeout; a dead engine is itself
/// useful information.
async fn health_snapshot(app: &AppHandle) -> Value {
    let boot = serde_json::to_value(crate::health::get_boot_phase_snapshot()).unwrap_or(Value::Null);
    let recording = format!("{:?}", crate::health::get_recording_status());
    let audio: Vec<Value> = crate::health::get_audio_device_status()
        .into_iter()
        .map(|d| json!({ "name": d.name, "running": d.is_running }))
        .collect();

    let api = crate::recording::local_api_context_from_app(app);
    let engine = match api
        .apply_auth(reqwest::Client::new().get(api.url("/health")))
        .timeout(Duration::from_secs(3))
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status().as_u16();
            match response.json::<Value>().await {
                Ok(body) => json!({ "httpStatus": status, "body": body }),
                Err(e) => json!({ "httpStatus": status, "error": format!("unreadable body: {e}") }),
            }
        }
        Err(e) => json!({ "error": format!("engine did not answer: {e}") }),
    };

    json!({
        "takenAt": chrono::Local::now().to_rfc3339(),
        "bootPhase": boot,
        "recordingStatus": recording,
        "audioDevices": audio,
        "engine": engine,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_accepts_a_sparse_payload() {
        let r: FeedbackUploadRequest = serde_json::from_value(json!({})).unwrap();
        assert!(r.feedback_text.is_empty() && r.video_path.is_none());
        let r: FeedbackUploadRequest = serde_json::from_value(json!({
            "feedbackText": "hi",
            "source": "onboarding-stuck",
            "consoleLog": "c",
            "screenshotDataUrl": null,
            "appSettings": { "port": 3030 }
        }))
        .unwrap();
        assert_eq!(r.feedback_text, "hi");
        assert_eq!(r.source.as_deref(), Some("onboarding-stuck"));
        assert!(r.app_settings.is_some());
    }

    #[test]
    fn completion_payload_uses_the_event_field_names() {
        let done = FeedbackUploadCompleted::failure("job-1", "boom".into());
        let v = serde_json::to_value(&done).unwrap();
        assert_eq!(v["jobId"], json!("job-1"));
        assert_eq!(v["sent"], json!(false));
        assert_eq!(v["path"], Value::Null);
        assert_eq!(v["error"], json!("boom"));
        assert!(v.get("issueUrl").is_some());
    }

    #[test]
    fn only_one_job_at_a_time() {
        let first = JobGuard::acquire().expect("first job");
        assert!(JobGuard::acquire().is_none());
        drop(first);
        assert!(JobGuard::acquire().is_some());
    }
}
