// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

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

/// `/health` top-level fields in the shipped serialisation order.
const HEALTH_FIELDS: [&str; 36] = [
    "status",
    "status_code",
    "dependencies",
    "runtime",
    "startup",
    "unhealthy_reasons",
    "last_frame_timestamp",
    "last_audio_timestamp",
    "frame_status",
    "vision_reason",
    "loop_stage",
    "loop_stage_age_secs",
    "audio_capture_mode",
    "audio_status",
    "message",
    "verbose_instructions",
    "device_status_details",
    "capture_status",
    "monitors",
    "pipeline",
    "audio_pipeline",
    "accessibility",
    "ui_recorder",
    "recording_coverage",
    "pool_stats",
    "write_queue_degraded",
    "write_queue_consecutive_fatal",
    "write_queue_consecutive_contention",
    "write_pool_reopens",
    "persistent_failure_signals",
    "vision_db_write_stalled",
    "audio_db_write_stalled",
    "drm_content_paused",
    "schedule_paused",
    "hostname",
    "version",
];

/// `/vision/metrics` fields in the shipped serialisation order.
const VISION_METRICS_FIELDS: [&str; 34] = [
    "uptime_secs",
    "frames_captured",
    "frames_skipped",
    "ocr_completed",
    "ocr_cache_hits",
    "ocr_cache_misses",
    "ocr_empty",
    "avg_ocr_latency_ms",
    "ocr_gate_skips",
    "ocr_gate_crop_ocr",
    "ocr_gate_detects",
    "avg_ocr_gate_detect_latency_ms",
    "frames_video_written",
    "frames_db_written",
    "frames_dropped",
    "frames_dropped_timeout",
    "frames_dropped_error",
    "silent_loss",
    "silent_loss_rate",
    "avg_db_latency_ms",
    "frame_drop_rate",
    "capture_fps_actual",
    "time_to_first_frame_ms",
    "ocr_queue_depth",
    "video_queue_depth",
    "pipeline_stall_count",
    "last_db_write_ts",
    "last_capture_attempt_ts",
    "last_capture_loop_heartbeat_ts",
    "capture_loop_heartbeats",
    "capture_attempts",
    "dedup_skips",
    "frames_corrupt_black",
    "frames_corrupt_green",
];

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

    // Every top-level field present appears in the shipped serialisation
    // order (fields that are skipped when empty may be absent).
    let present: Vec<&str> = HEALTH_FIELDS
        .iter()
        .copied()
        .filter(|k| value.get(*k).is_some())
        .collect();
    for always in [
        "status",
        "dependencies",
        "runtime",
        "startup",
        "unhealthy_reasons",
        "vision_reason",
        "loop_stage",
        "loop_stage_age_secs",
        "audio_capture_mode",
        "capture_status",
        "recording_coverage",
        "pool_stats",
        "write_queue_degraded",
        "write_queue_consecutive_fatal",
        "write_queue_consecutive_contention",
        "write_pool_reopens",
        "persistent_failure_signals",
    ] {
        assert!(present.contains(&always), "{} missing in {}", always, text);
    }
    assert_eq!(
        value.as_object().unwrap().len(),
        present.len(),
        "unexpected top-level field in {}",
        text
    );
    assert_key_order(&text, &present);

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
    assert_eq!(
        value.as_object().unwrap().len(),
        VISION_METRICS_FIELDS.len()
    );
    assert_key_order(&text, &VISION_METRICS_FIELDS);
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
