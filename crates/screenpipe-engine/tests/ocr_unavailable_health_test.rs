// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! With no OCR engine or language installed (simulated through the
//! `FREENPIPE_OCR_FAULT` test hook), the startup OCR probe makes `/health`
//! report `vision_reason = ocr_unavailable` while capture is otherwise
//! healthy, and the on-demand OCR route answers 503 without caching an empty
//! result. Its own test binary with a single test, because the OCR state and
//! the hook's environment variable are process-wide.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use screenpipe_audio::audio_manager::AudioManagerBuilder;
use screenpipe_db::DatabaseManager;
use screenpipe_engine::core::{WorkerContext, BACKGROUND_WORKERS};
use screenpipe_engine::SCServer;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

async fn json_of(response: axum::response::Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn ocr_unavailable_is_reported_and_capture_carries_on() {
    // Simulate a machine without any OCR language pack / tesseract.
    std::env::set_var(
        screenpipe_screen::ocr_availability::OCR_FAULT_ENV,
        "unavailable",
    );

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let db = Arc::new(
        DatabaseManager::new("sqlite::memory:", Default::default())
            .await
            .unwrap(),
    );

    // Run the registered startup probe to completion.
    let probe = BACKGROUND_WORKERS
        .iter()
        .find(|w| w.name == "ocr_availability_probe")
        .expect("the OCR probe is registered as a background worker");
    (probe.start)(WorkerContext {
        db: db.clone(),
        data_dir: dir.clone(),
        shutdown: CancellationToken::new(),
    })
    .await;
    assert!(screenpipe_screen::metrics::ocr_engine_unavailable());

    let audio_manager = Arc::new(
        AudioManagerBuilder::new()
            .is_disabled(true)
            .output_path(dir.join("audio"))
            .build(db.clone())
            .await
            .unwrap(),
    );
    let server = SCServer::new(
        db.clone(),
        SocketAddr::from(([127, 0, 0, 1], 3035)),
        dir.clone(),
        false, // vision enabled
        true,  // audio disabled: no devices in a test process
        audio_manager,
        false,
        "balanced".to_string(),
    );
    // Capture is otherwise healthy: a frame was just attempted and written.
    server.vision_metrics.record_capture_attempt();
    server
        .vision_metrics
        .record_db_write(std::time::Duration::from_millis(5));

    let router = server.create_router().await;
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let health = json_of(response).await;

    assert_eq!(health["frame_status"], "ok", "{}", health);
    assert_eq!(health["vision_reason"], "ocr_unavailable", "{}", health);
    let reasons = health["unhealthy_reasons"].as_array().unwrap();
    assert!(
        reasons
            .iter()
            .any(|r| r["code"] == "vision_ocr_unavailable" && r["severity"] == "warning"),
        "{}",
        health
    );

    // A stored frame without text: on-demand OCR answers 503 with the frame
    // id instead of a 200 with no positions, so the timeline does not keep
    // "no text" for a frame that was never read.
    let snapshot = dir.join("frame.jpg");
    image::RgbImage::from_pixel(64, 32, image::Rgb([250, 250, 250]))
        .save(&snapshot)
        .unwrap();
    let frame_id = db
        .insert_snapshot_frame(
            "monitor_1",
            chrono::Utc::now(),
            snapshot.to_str().unwrap(),
            Some("Notepad"),
            Some("notes.txt"),
            None,
            true,
            Some("click"),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/frames/{}/text", frame_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = json_of(response).await;
    assert_eq!(body["frame_id"], frame_id, "{}", body);
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|e| e.contains("OCR unavailable")),
        "{}",
        body
    );
    assert!(db
        .get_frame_text_positions(frame_id)
        .await
        .unwrap()
        .is_empty());

    std::env::remove_var(screenpipe_screen::ocr_availability::OCR_FAULT_ENV);
}
