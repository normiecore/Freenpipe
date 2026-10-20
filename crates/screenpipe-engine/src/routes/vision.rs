// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Per-monitor vision control routes (`/vision/device/*`).
//!
//! The recording popover and the tray pause and resume single monitors here,
//! the same way `/audio/device/*` works for audio devices:
//!
//! - `POST /vision/device/stop` `{"monitor_id": n}` pauses a monitor: it is
//!   added to the user-paused set and its capture task stops. The manager and
//!   the monitor watcher never restart a paused monitor.
//! - `POST /vision/device/start` `{"monitor_id": n}` lifts the pause and starts
//!   capture again when vision capture is running.
//! - `GET /vision/device/status` lists every selected monitor of the latest
//!   enumeration as `{id, name, active, user_disabled}`; `[]` when vision
//!   capture is disabled.
//!
//! Pauses live in memory for the life of the process (see
//! [`VisionDeviceControl`]); they survive capture restarts and monitor
//! hot-plug but not a restart of the app.

use std::sync::Arc;

use axum::extract::{Json, State};
use axum::http::StatusCode;
use oasgen::{oasgen, OaSchema};
use serde::{Deserialize, Serialize};

use super::ext::{documented_routes, DocumentedRoutes, ExtRouter};
use crate::server::AppState;
use crate::vision_manager::{DeviceControlError, VisionDeviceControl};

/// Body of `POST /vision/device/start` and `POST /vision/device/stop`.
#[derive(OaSchema, Debug, Clone, Deserialize)]
pub struct VisionDeviceControlRequest {
    /// Numeric monitor id, as `/vision/device/status` reports it.
    pub monitor_id: u32,
}

/// Outcome of a pause or resume request.
#[derive(OaSchema, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisionDeviceControlResponse {
    pub success: bool,
    pub message: String,
}

/// Recording state of a single monitor, as the recording popover shows it.
#[derive(OaSchema, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisionDeviceStatusEntry {
    /// Numeric monitor id; pass it back as `monitor_id`.
    pub id: u32,
    pub name: String,
    /// Frames are being captured from this monitor.
    pub active: bool,
    /// The user paused this monitor.
    pub user_disabled: bool,
}

type ControlResult =
    Result<Json<VisionDeviceControlResponse>, (StatusCode, Json<VisionDeviceControlResponse>)>;

/// Which way a control request goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeviceAction {
    Pause,
    Resume,
}

fn refusal(status: StatusCode, message: String) -> (StatusCode, Json<VisionDeviceControlResponse>) {
    (
        status,
        Json(VisionDeviceControlResponse {
            success: false,
            message,
        }),
    )
}

/// Handler body shared by both control routes, separate from axum state so
/// tests can drive it with their own registry.
async fn control_device(
    control: &VisionDeviceControl,
    vision_disabled: bool,
    monitor_id: u32,
    action: DeviceAction,
) -> ControlResult {
    let verb = match action {
        DeviceAction::Pause => "pause",
        DeviceAction::Resume => "resume",
    };
    if vision_disabled {
        return Err(refusal(
            StatusCode::CONFLICT,
            format!(
                "could not {} screen recording on monitor {}: screen capture is turned off",
                verb, monitor_id
            ),
        ));
    }
    let outcome = match action {
        DeviceAction::Pause => control.pause_monitor(monitor_id).await,
        DeviceAction::Resume => control.resume_monitor(monitor_id).await,
    };
    match outcome {
        Ok(()) => Ok(Json(VisionDeviceControlResponse {
            success: true,
            message: match action {
                DeviceAction::Pause => format!("screen recording paused on monitor {}", monitor_id),
                DeviceAction::Resume => {
                    format!("screen recording resumed on monitor {}", monitor_id)
                }
            },
        })),
        Err(e) => {
            let status = match e {
                DeviceControlError::NotFound(_) => StatusCode::NOT_FOUND,
                DeviceControlError::NotSelected(_) => StatusCode::CONFLICT,
                DeviceControlError::Failed(_) => StatusCode::INTERNAL_SERVER_ERROR,
            };
            Err(refusal(
                status,
                format!(
                    "could not {} screen recording on monitor {}: {}",
                    verb, monitor_id, e
                ),
            ))
        }
    }
}

/// Status rows for `GET /vision/device/status`.
fn device_status(
    control: &VisionDeviceControl,
    vision_disabled: bool,
) -> Vec<VisionDeviceStatusEntry> {
    if vision_disabled {
        return Vec::new();
    }
    control
        .status()
        .into_iter()
        .map(|m| VisionDeviceStatusEntry {
            id: m.id,
            name: m.name,
            active: m.active,
            user_disabled: m.user_disabled,
        })
        .collect()
}

/// Resume screen capture on one monitor the user paused.
#[oasgen]
async fn start_vision_device(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<VisionDeviceControlRequest>,
) -> ControlResult {
    let control = VisionDeviceControl::global();
    control_device(
        &control,
        state.vision_disabled,
        payload.monitor_id,
        DeviceAction::Resume,
    )
    .await
}

/// Pause screen capture on one monitor until the user resumes it.
#[oasgen]
async fn stop_vision_device(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<VisionDeviceControlRequest>,
) -> ControlResult {
    let control = VisionDeviceControl::global();
    control_device(
        &control,
        state.vision_disabled,
        payload.monitor_id,
        DeviceAction::Pause,
    )
    .await
}

/// Recording state of every selected monitor.
#[oasgen]
async fn vision_device_status(
    State(state): State<Arc<AppState>>,
) -> Json<Vec<VisionDeviceStatusEntry>> {
    Json(device_status(
        &VisionDeviceControl::global(),
        state.vision_disabled,
    ))
}

/// Routes described in `/openapi.json`.
pub fn documented() -> DocumentedRoutes {
    documented_routes!(oasgen::Server::axum()
        .post("/vision/device/start", start_vision_device)
        .post("/vision/device/stop", stop_vision_device)
        .get("/vision/device/status", vision_device_status))
}

/// Routes outside the OpenAPI spec: none.
pub fn router() -> ExtRouter {
    ExtRouter::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vision_manager::MonitorSummary;

    fn summary(id: u32, name: &str, selected: bool) -> MonitorSummary {
        MonitorSummary {
            id,
            stable_id: format!("{name}_1920x1080_0,0"),
            name: name.to_string(),
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            is_primary: id == 1,
            selected,
        }
    }

    fn control_with(monitors: Vec<MonitorSummary>) -> VisionDeviceControl {
        let control = VisionDeviceControl::new();
        control.publish_monitors(monitors);
        control
    }

    #[test]
    fn status_rows_use_the_interop_keys() {
        let control = control_with(vec![summary(1, "\\\\.\\DISPLAY1", true)]);
        let rows = device_status(&control, false);
        let json = serde_json::to_value(&rows).unwrap();
        assert_eq!(
            json,
            serde_json::json!([
                {"id": 1, "name": "\\\\.\\DISPLAY1", "active": false, "user_disabled": false}
            ])
        );
    }

    #[test]
    fn status_lists_selected_monitors_only_and_is_empty_when_vision_is_off() {
        let control = control_with(vec![summary(1, "A", true), summary(2, "B", false)]);
        let ids: Vec<u32> = device_status(&control, false)
            .iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, vec![1]);
        assert!(device_status(&control, true).is_empty());
    }

    #[test]
    fn request_body_needs_monitor_id() {
        let ok: VisionDeviceControlRequest =
            serde_json::from_str(r#"{"monitor_id": 65537}"#).unwrap();
        assert_eq!(ok.monitor_id, 65537);
        assert!(serde_json::from_str::<VisionDeviceControlRequest>(r#"{"id": 1}"#).is_err());
    }

    #[tokio::test]
    async fn stop_then_start_without_a_running_manager() {
        let control = control_with(vec![summary(1, "A", true), summary(2, "B", true)]);

        let paused = control_device(&control, false, 2, DeviceAction::Pause)
            .await
            .unwrap()
            .0;
        assert!(paused.success);
        assert!(paused.message.contains("paused"), "{}", paused.message);
        assert!(paused.message.contains('2'));

        let rows = device_status(&control, false);
        let row = rows.iter().find(|r| r.id == 2).unwrap();
        assert!(row.user_disabled);
        assert!(!row.active);
        assert!(!rows.iter().find(|r| r.id == 1).unwrap().user_disabled);

        // Pausing twice is not an error.
        assert!(control_device(&control, false, 2, DeviceAction::Pause)
            .await
            .is_ok());

        let resumed = control_device(&control, false, 2, DeviceAction::Resume)
            .await
            .unwrap()
            .0;
        assert!(resumed.success);
        assert!(resumed.message.contains("resumed"), "{}", resumed.message);
        assert!(!device_status(&control, false)
            .iter()
            .any(|r| r.user_disabled));
    }

    #[tokio::test]
    async fn unknown_monitors_are_404_and_disabled_vision_is_409() {
        let control = control_with(vec![summary(1, "A", true)]);
        for action in [DeviceAction::Pause, DeviceAction::Resume] {
            let (status, body) = control_device(&control, false, 99, action)
                .await
                .unwrap_err();
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert!(!body.0.success);
            assert!(body.0.message.contains("99"), "{}", body.0.message);

            let (status, body) = control_device(&control, true, 1, action).await.unwrap_err();
            assert_eq!(status, StatusCode::CONFLICT);
            assert!(!body.0.success);
        }
        assert!(control.paused_ids().is_empty());
    }

    #[test]
    fn documented_routes_cover_the_three_paths() {
        let mut paths = documented().paths();
        paths.sort();
        assert_eq!(
            paths,
            vec![
                "/vision/device/start".to_string(),
                "/vision/device/status".to_string(),
                "/vision/device/stop".to_string(),
            ]
        );
    }

    /// The routes are mounted on the real server router and answer with the
    /// interop shapes (what `api /vision/device/status` sees).
    #[tokio::test]
    async fn routes_are_served_by_the_engine_router() {
        use axum::body::{to_bytes, Body};
        use axum::http::Request;
        use screenpipe_audio::audio_manager::AudioManagerBuilder;
        use screenpipe_db::DatabaseManager;
        use tower::ServiceExt;

        let db = Arc::new(
            DatabaseManager::new("sqlite::memory:", Default::default())
                .await
                .unwrap(),
        );
        let data_dir = tempfile::tempdir().unwrap();
        let audio_manager = Arc::new(
            AudioManagerBuilder::new()
                .output_path(data_dir.path().to_path_buf())
                .is_disabled(true)
                .build(db.clone())
                .await
                .unwrap(),
        );
        let router = crate::SCServer::new(
            db,
            std::net::SocketAddr::from(([127, 0, 0, 1], 3035)),
            data_dir.path().to_path_buf(),
            false,
            true,
            audio_manager,
            false,
            "balanced".to_string(),
        )
        .create_router()
        .await;

        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/vision/device/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        for row in &rows {
            let mut keys: Vec<&str> = row
                .as_object()
                .unwrap()
                .keys()
                .map(|k| k.as_str())
                .collect();
            keys.sort();
            assert_eq!(keys, vec!["active", "id", "name", "user_disabled"]);
        }

        // An id no enumeration produces: refused with the control body shape.
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/vision/device/stop")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"monitor_id": 4000000000}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let refused: VisionDeviceControlResponse = serde_json::from_slice(&body).unwrap();
        assert!(!refused.success);
    }
}
