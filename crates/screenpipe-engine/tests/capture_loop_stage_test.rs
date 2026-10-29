// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! `/health` reports the capture loop that has been in its current stage the
//! longest (`loop_stage`, `loop_stage_age_secs`), as published by the loops'
//! stage guards. Served through the real router; no port is bound.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use screenpipe_audio::audio_manager::AudioManagerBuilder;
use screenpipe_db::DatabaseManager;
use screenpipe_engine::SCServer;
use screenpipe_screen::CaptureLoopStage;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower::ServiceExt;

#[tokio::test]
async fn health_reports_the_most_stuck_capture_loop_stage() {
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
        false, // vision enabled, so loop_stage is filled in
        true,  // audio disabled: no devices in a test process
        audio_manager,
        false,
        "balanced".to_string(),
    );
    let metrics = server.vision_metrics.clone();

    // Monitor 1's loop froze in the exclusion probe a while ago; monitor 2's
    // loop just started waiting for a trigger.
    let frozen_at = Instant::now()
        .checked_sub(Duration::from_secs(5))
        .expect("the test host has been up for more than 5 s");
    let stuck = metrics.track_capture_loop_at(1, frozen_at);
    stuck.enter_at(CaptureLoopStage::ExclusionProbe, frozen_at);
    let turning = metrics.track_capture_loop(2);
    turning.enter(CaptureLoopStage::TriggerWait);

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
    // A test process records nothing, so the status itself may be degraded;
    // the body is the full health document either way.
    assert!(
        matches!(
            response.status(),
            StatusCode::OK | StatusCode::SERVICE_UNAVAILABLE
        ),
        "{}",
        response.status()
    );
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(value["loop_stage"], "exclusion-probe", "{value}");
    let age = value["loop_stage_age_secs"]
        .as_u64()
        .expect("loop_stage_age_secs is a number");
    assert!(age >= 5, "age {age} in {value}");

    // Both loops unregister when their guards drop (the loop task ended).
    drop(stuck);
    drop(turning);
    assert!(metrics.loop_stages.most_stuck().is_none());
}
