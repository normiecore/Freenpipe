// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! With no OCR engine or language installed (simulated through the
//! `FREENPIPE_OCR_FAULT` test hook), the startup OCR probe makes `/health`
//! report `vision_reason = ocr_unavailable` while capture is otherwise
//! healthy. Its own test binary, because the OCR state is process-wide.

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

#[tokio::test]
async fn startup_probe_reports_ocr_unavailable_on_health() {
    // Simulate a machine without any OCR language pack / tesseract.
    std::env::set_var(
        screenpipe_screen::ocr_availability::OCR_FAULT_ENV,
        "unavailable",
    );

    let dir = tempfile::tempdir().unwrap().keep();
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
        db,
        SocketAddr::from(([127, 0, 0, 1], 3035)),
        dir,
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
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let health: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

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

    std::env::remove_var(screenpipe_screen::ocr_availability::OCR_FAULT_ENV);
}
