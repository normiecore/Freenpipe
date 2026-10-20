// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! `/health` v2 and `/vision/metrics` served through the real router (no
//! port is bound; requests go straight into the axum service).

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use screenpipe_audio::audio_manager::AudioManagerBuilder;
use screenpipe_db::DatabaseManager;
use screenpipe_engine::SCServer;
use std::net::SocketAddr;
use std::sync::Arc;
use tower::ServiceExt;

async fn test_router() -> Router {
    let dir = tempfile::tempdir().unwrap().keep();
    let db = Arc::new(
        DatabaseManager::new("sqlite::memory:", Default::default())
            .await
            .unwrap(),
    );
    let audio_manager = Arc::new(
        AudioManagerBuilder::new()
            .is_disabled(true)
            .output_path(dir.join("audio"))
            .build(db.clone())
            .await
            .unwrap(),
    );
    let server = SCServer::new(
        db,
        SocketAddr::from(([127, 0, 0, 1], 3035)),
        dir,
        false, // vision enabled, so the vision fields are filled in
        true,  // audio disabled: no devices in a test process
        audio_manager,
        false,
        "balanced".to_string(),
    );
    server.create_router().await
}

async fn get_text(router: Router, uri: &str) -> (StatusCode, String) {
    let response = router
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

/// Assert that `keys` appear in `text` as JSON keys in this order. Each key
/// is searched after the previous one, so repeated names in nested objects
/// do not confuse the check.
fn assert_key_order(text: &str, keys: &[&str]) {
    let mut from = 0;
    for key in keys {
        let needle = format!("\"{}\":", key);
        match text[from..].find(&needle) {
            Some(at) => from += at + needle.len(),
            None => panic!("{} missing or out of order in {}", key, text),
        }
    }
}

#[tokio::test]
async fn health_reports_v2_fields_and_ordered_recording_coverage() {
    let (status, text) = get_text(test_router().await, "/health").await;
    assert_eq!(status, StatusCode::OK);
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();

    // Top-level order of the fields this WP adds, relative to the base ones.
    assert_key_order(
        &text,
        &[
            "status",
            "status_code",
            "dependencies",
            "runtime",
            "startup",
            "unhealthy_reasons",
            "last_frame_timestamp",
            "frame_status",
            "vision_reason",
            "loop_stage",
            "loop_stage_age_secs",
            "audio_capture_mode",
            "audio_status",
            "message",
            "capture_status",
            "recording_coverage",
            "pool_stats",
            "write_queue_degraded",
            "write_queue_consecutive_fatal",
            "write_queue_consecutive_contention",
            "write_pool_reopens",
            "persistent_failure_signals",
            "vision_db_write_stalled",
            "schedule_paused",
        ],
    );

    // recording_coverage carries its six fields in the shipped order.
    let coverage_at = text.find("\"recording_coverage\":").unwrap();
    assert_key_order(
        &text[coverage_at..],
        &[
            "recording_secs",
            "active_stalled_secs",
            "active_paused_secs",
            "idle_secs",
            "active_secs",
            "coverage_ratio",
        ],
    );
    let coverage = value["recording_coverage"].as_object().unwrap();
    assert_eq!(coverage.len(), 6);

    assert!(value["unhealthy_reasons"].is_array());
    assert!(value["vision_reason"].is_string());
    assert_eq!(value["audio_capture_mode"], "disabled");
    assert_eq!(value["capture_status"]["status"], "disabled");
    assert_eq!(value["runtime"]["transcription"], "not_required");
    assert!(value["startup"]["engine_started_at"].is_string());
    assert!(value["dependencies"]["ffmpeg"]["status"].is_string());
    assert_eq!(value["write_queue_degraded"], false);

    // The vision pipeline block gained the loss and heartbeat counters.
    let pipeline = &value["pipeline"];
    for key in [
        "silent_loss",
        "silent_loss_rate",
        "capture_loop_heartbeats",
        "frames_dropped_timeout",
        "frames_dropped_error",
        "last_capture_loop_heartbeat_ts",
        "ocr_empty",
    ] {
        assert!(pipeline.get(key).is_some(), "pipeline.{} missing", key);
    }
}

#[tokio::test]
async fn vision_metrics_include_silent_loss_and_loop_heartbeats() {
    let (status, text) = get_text(test_router().await, "/vision/metrics").await;
    assert_eq!(status, StatusCode::OK);
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 34);
    assert_key_order(
        &text,
        &[
            "ocr_cache_misses",
            "ocr_empty",
            "avg_ocr_latency_ms",
            "frames_dropped",
            "frames_dropped_timeout",
            "frames_dropped_error",
            "silent_loss",
            "silent_loss_rate",
            "avg_db_latency_ms",
            "last_capture_attempt_ts",
            "last_capture_loop_heartbeat_ts",
            "capture_loop_heartbeats",
            "capture_attempts",
            "dedup_skips",
            "frames_corrupt_black",
            "frames_corrupt_green",
        ],
    );
    assert_eq!(value["silent_loss"], 0);
    assert_eq!(value["capture_loop_heartbeats"], 0);
}

#[tokio::test]
async fn audio_metrics_include_last_stream_timeout_at() {
    let (status, text) = get_text(test_router().await, "/audio/metrics").await;
    assert_eq!(status, StatusCode::OK);
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(value.get("last_stream_timeout_at").is_some());
    assert!(value["last_stream_timeout_at"].is_null());
}
