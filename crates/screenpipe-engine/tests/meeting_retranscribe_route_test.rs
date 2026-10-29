// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! `POST /meetings/:id/retranscribe` over HTTP, in process: the response
//! carries `transcript_updated_at` and `summary_refresh_event_emitted`
//! (api-039). The summary refresh after a real replacement is covered by
//! the `retranscribe` unit tests, which need no audio files.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use screenpipe_audio::audio_manager::AudioManagerBuilder;
use screenpipe_db::DatabaseManager;
use screenpipe_engine::SCServer;
use serde_json::{json, Value};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tower::ServiceExt;

async fn setup() -> (Router, Arc<DatabaseManager>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(
        DatabaseManager::new("sqlite::memory:", Default::default())
            .await
            .unwrap(),
    );
    let audio_manager = Arc::new(
        AudioManagerBuilder::new()
            .output_path(dir.path().to_path_buf())
            // No real devices or models in tests.
            .is_disabled(true)
            .build(db.clone())
            .await
            .unwrap(),
    );
    let server = SCServer::new(
        db.clone(),
        SocketAddr::from(([127, 0, 0, 1], 3035)),
        PathBuf::from(dir.path()),
        false,
        false,
        audio_manager,
        false,
        "balanced".to_string(),
    );
    (server.create_router().await, db, dir)
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            req = req.header("Content-Type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    (status, value)
}

#[tokio::test]
async fn retranscribe_reports_transcript_update_and_summary_refresh_fields() {
    let (app, db, _dir) = setup().await;

    let (status, started) = call(
        &app,
        "POST",
        "/meetings/start",
        Some(json!({"title": "Retranscribe me"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{started}");
    let id = started["id"].as_i64().unwrap();

    // The route refuses a meeting shorter than a second, so start this one
    // ten minutes ago instead of waiting.
    let earlier = (chrono::Utc::now() - chrono::Duration::minutes(10))
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    db.update_meeting(id, Some(&earlier), None, None, None, None, None)
        .await
        .unwrap();

    // Still running: refused.
    let (status, _) = call(
        &app,
        "POST",
        &format!("/meetings/{id}/retranscribe"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, _) = call(&app, "POST", "/meetings/stop", Some(json!({"id": id}))).await;
    assert_eq!(status, StatusCode::OK);

    // Stopped, but no audio was recorded: nothing is replaced, so there is no
    // transcript update and no refresh request. Both fields are present.
    let (status, body) = call(
        &app,
        "POST",
        &format!("/meetings/{id}/retranscribe"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["meeting_id"], json!(id));
    assert_eq!(body["replaced_segments"], json!(0));
    let fields = body.as_object().unwrap();
    assert!(fields.contains_key("transcript_updated_at"), "{body}");
    assert!(
        fields.contains_key("summary_refresh_event_emitted"),
        "{body}"
    );
    assert_eq!(body["transcript_updated_at"], Value::Null);
    assert_eq!(body["summary_refresh_event_emitted"], json!(false));
}
