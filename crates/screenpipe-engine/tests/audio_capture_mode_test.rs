// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Audio routes under `audioCaptureMode` (audio-010, audio-026) and the
//! device status event (audio-018), served through the real router. No port
//! is bound and no audio device is opened: the audio manager is built with
//! audio off, which is exactly the `disabled` mode under test.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use futures::StreamExt;
use screenpipe_audio::audio_manager::AudioManagerBuilder;
use screenpipe_config::AudioCaptureMode;
use screenpipe_db::DatabaseManager;
use screenpipe_engine::SCServer;
use screenpipe_events::AudioDeviceStatusChangedEvent;
use tower::ServiceExt;

const DISABLED_MESSAGE: &str = "Audio capture is disabled in settings";

/// A router whose audio manager runs with `audioCaptureMode=disabled`.
/// `server_audio_disabled` is what the server was started with; the routes
/// must refuse in both cases, because the app can apply new audio options
/// to a running server.
async fn router(server_audio_disabled: bool) -> Router {
    let dir = tempfile::tempdir().unwrap().keep();
    let db = Arc::new(
        DatabaseManager::new("sqlite::memory:", Default::default())
            .await
            .unwrap(),
    );
    let audio_manager = Arc::new(
        AudioManagerBuilder::new()
            .audio_capture_mode(AudioCaptureMode::Disabled)
            .output_path(dir.join("audio"))
            .build(db.clone())
            .await
            .unwrap(),
    );
    assert!(audio_manager.is_disabled().await);
    SCServer::new(
        db,
        SocketAddr::from(([127, 0, 0, 1], 3035)),
        dir,
        true,
        server_audio_disabled,
        audio_manager,
        false,
        "balanced".to_string(),
    )
    .create_router()
    .await
}

async fn post_json(router: Router, uri: &str, body: serde_json::Value) -> (StatusCode, String) {
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn audio_start_returns_the_disabled_error() {
    for server_audio_disabled in [true, false] {
        let (status, text) = post_json(
            router(server_audio_disabled).await,
            "/audio/start",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{text}");
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["success"], false);
        assert_eq!(value["message"], DISABLED_MESSAGE);
    }
}

#[tokio::test]
async fn device_start_returns_the_disabled_error() {
    for server_audio_disabled in [true, false] {
        let (status, text) = post_json(
            router(server_audio_disabled).await,
            "/audio/device/start",
            serde_json::json!({ "device_name": "Test Microphone (input)" }),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{text}");
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["message"], DISABLED_MESSAGE);
    }
}

#[tokio::test]
async fn health_reports_disabled_capture_mode() {
    let response = router(true)
        .await
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["audio_capture_mode"], "disabled");
    assert_eq!(value["audio_status"], "disabled");
}

#[tokio::test]
async fn device_stop_publishes_a_status_change() {
    // `/ws/events` streams every event on the bus; subscribing to the bus
    // sees what a websocket client would receive.
    let mut events = screenpipe_events::subscribe_to_event::<AudioDeviceStatusChangedEvent>(
        "audio_device_status_changed",
    );
    let device = "W3-14 Status Test Mic (input)";
    let (status, text) = post_json(
        router(false).await,
        "/audio/device/stop",
        serde_json::json!({ "device_name": device }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");

    let event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.next().await.expect("event bus closed");
            if event.data.device_name == device {
                return event.data;
            }
        }
    })
    .await
    .expect("no audio_device_status_changed event for the stopped device");
    assert!(!event.is_running);
    assert!(event.is_user_disabled);
    assert_eq!(
        serde_json::to_value(event.reason).unwrap(),
        serde_json::json!("user_paused")
    );
}
