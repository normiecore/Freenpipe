// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Meetings API v2 over HTTP, in process: calendar binding on start and PUT,
//! the summary save and status routes, the room-change route, transcript
//! ordering and the `q` filter over `meeting_app`. These mirror the W2-08
//! harness acceptance checks without starting a recording engine.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use screenpipe_audio::audio_manager::AudioManagerBuilder;
use screenpipe_db::DatabaseManager;
use screenpipe_engine::routes::meetings::{register_room_change_offer, ROOM_CHANGE_DECISION_ERROR};
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

async fn end_reason(db: &DatabaseManager, id: i64) -> Option<String> {
    let row: (Option<String>,) = sqlx::query_as("SELECT end_reason FROM meetings WHERE id = ?1")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    row.0
}

#[tokio::test]
async fn calendar_event_binds_on_start_and_a_conflicting_put_keeps_the_title() {
    let (app, db, _dir) = setup().await;

    let (status, first) = call(
        &app,
        "POST",
        "/meetings/start",
        Some(json!({"title": "Weekly sync", "calendar_event_id": "evt1"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let first_id = first["id"].as_i64().unwrap();
    assert_eq!(
        db.get_meeting_calendar_event_id(first_id).await.unwrap(),
        Some("evt1".to_string())
    );

    let (status, _) = call(
        &app,
        "POST",
        "/meetings/stop",
        Some(json!({"id": first_id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, second) = call(
        &app,
        "POST",
        "/meetings/start",
        Some(json!({"title": "Hallway chat"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let second_id = second["id"].as_i64().unwrap();

    // Enriching the second meeting from the same event: no error, no rename.
    let (status, updated) = call(
        &app,
        "PUT",
        &format!("/meetings/{second_id}"),
        Some(json!({
            "title": "Weekly sync",
            "attendees": "ana@example.com",
            "calendar_event_id": "evt1",
            "note": "kept",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["title"], "Hallway chat");
    assert_eq!(updated["attendees"], Value::Null);
    assert_eq!(updated["note"], "kept");
    assert_eq!(
        db.get_meeting_calendar_event_id(second_id).await.unwrap(),
        None
    );

    // A free event binds through PUT and its title applies.
    let (status, updated) = call(
        &app,
        "PUT",
        &format!("/meetings/{second_id}"),
        Some(json!({"title": "Design review", "calendar_event_id": "evt2"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["title"], "Design review");
    assert_eq!(
        db.get_meeting_calendar_event_id(second_id).await.unwrap(),
        Some("evt2".to_string())
    );

    let (status, _) = call(
        &app,
        "PUT",
        "/meetings/987654",
        Some(json!({"calendar_event_id": "evt3"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn summary_routes_save_validate_and_report_status() {
    let (app, db, _dir) = setup().await;
    let id = db
        .insert_meeting("zoom.us", "ui_scan", None, None)
        .await
        .unwrap();
    db.update_meeting(id, None, None, None, None, Some("agenda by me"), None)
        .await
        .unwrap();

    let (status, saved) = call(
        &app,
        "POST",
        &format!("/meetings/{id}/summary"),
        Some(json!({"summary": "x"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["note"], "agenda by me\n\n## Summary\nx");

    // Refreshing replaces the section and can retitle.
    let (status, saved) = call(
        &app,
        "POST",
        &format!("/meetings/{id}/summary"),
        Some(json!({"summary": "## Summary\ny", "title": "Q3 planning"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["note"], "agenda by me\n\n## Summary\ny");
    assert_eq!(saved["title"], "Q3 planning");

    for bad in [
        json!({"summary": ""}),
        json!({"summary": "   "}),
        json!({}),
        json!({"summary": "I'm sorry, but there is no transcript to summarize."}),
    ] {
        let (status, body) =
            call(&app, "POST", &format!("/meetings/{id}/summary"), Some(bad)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["error"].is_string());
    }
    let after = db.get_meeting_by_id(id).await.unwrap();
    assert_eq!(after.note.as_deref(), Some("agenda by me\n\n## Summary\ny"));

    let (status, _) = call(
        &app,
        "POST",
        "/meetings/424242/summary",
        Some(json!({"summary": "x"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = call(&app, "GET", &format!("/meetings/{id}/summary-status"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "idle");
    assert_eq!(body["status"], "idle");
    assert_eq!(body["auto_summary_enabled"], false);

    let (status, _) = call(
        &app,
        "GET",
        &format!("/meetings/{id}/summary-status?not_before=yesterday"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    sqlx::query(
        "INSERT INTO pipe_executions (pipe_name, status, trigger_type, started_at, \
         trigger_event, trigger_key) VALUES ('meeting-summary', 'running', 'event', ?1, \
         'meeting_ended', ?2)",
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(id.to_string())
    .execute(&db.pool)
    .await
    .unwrap();
    let (_, body) = call(&app, "GET", &format!("/meetings/{id}/summary-status"), None).await;
    assert_eq!(body["state"], "running");
    assert!(body["execution_id"].is_i64());
    let (_, body) = call(
        &app,
        "GET",
        &format!("/meetings/{id}/summary-status?pipe=other-pipe"),
        None,
    )
    .await;
    assert_eq!(body["state"], "idle");

    let (status, _) = call(&app, "GET", "/meetings/424242/summary-status", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn room_change_switch_ends_the_meeting_and_starts_a_new_one() {
    let (app, db, _dir) = setup().await;
    let old = db
        .insert_meeting("Google Chrome", "ui_scan", Some("Standup"), None)
        .await
        .unwrap();

    let (status, body) = call(
        &app,
        "POST",
        "/meetings/room-change",
        Some(json!({"meetingId": old, "token": "tok-1", "decision": "maybe"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], ROOM_CHANGE_DECISION_ERROR);

    let (status, _) = call(
        &app,
        "POST",
        "/meetings/room-change",
        Some(json!({"meetingId": old, "decision": "keep"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "token is required");

    // keep: nothing changes.
    let (status, body) = call(
        &app,
        "POST",
        "/meetings/room-change",
        Some(json!({"meetingId": old, "token": "tok-1", "decision": "keep"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["decision"], "keep");
    assert_eq!(body["activeMeetingId"], old);
    assert!(db
        .get_meeting_by_id(old)
        .await
        .unwrap()
        .meeting_end
        .is_none());

    // switch
    let (status, body) = call(
        &app,
        "POST",
        "/meetings/room-change",
        Some(json!({"meetingId": old, "token": "tok-2", "decision": "switch"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["decision"], "switch");
    assert_eq!(body["meetingId"], old);
    let new_id = body["newMeetingId"].as_i64().unwrap();
    assert_ne!(new_id, old);
    assert_eq!(body["activeMeetingId"], new_id);

    let ended = db.get_meeting_by_id(old).await.unwrap();
    assert!(ended.meeting_end.is_some());
    assert_eq!(end_reason(&db, old).await.as_deref(), Some("room_changed"));
    let fresh = db.get_meeting_by_id(new_id).await.unwrap();
    assert!(fresh.meeting_end.is_none());
    assert_eq!(fresh.meeting_app, "Google Chrome");
    assert_eq!(fresh.detection_source, "audio_process_room_change");

    let (_, status_body) = call(&app, "GET", "/meetings/status", None).await;
    assert_eq!(status_body["activeMeetingId"], new_id);
    assert_eq!(status_body["stoppableMeetingId"], new_id);

    // Answering the old prompt again: the meeting is gone.
    let (status, _) = call(
        &app,
        "POST",
        "/meetings/room-change",
        Some(json!({"meetingId": old, "token": "tok-2", "decision": "switch"})),
    )
    .await;
    assert_eq!(status, StatusCode::GONE);
    let (status, _) = call(
        &app,
        "POST",
        "/meetings/room-change",
        Some(json!({"meetingId": 515151, "token": "t", "decision": "keep"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A registered prompt only accepts its own token.
    register_room_change_offer(new_id, "current");
    let (status, _) = call(
        &app,
        "POST",
        "/meetings/room-change",
        Some(json!({"meetingId": new_id, "token": "stale", "decision": "switch"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = call(
        &app,
        "POST",
        "/meetings/room-change",
        Some(json!({"meetingId": new_id, "token": "current", "decision": "keep"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // The new meeting is the stoppable one.
    let (status, _) = call(&app, "POST", "/meetings/stop", Some(json!({"id": new_id}))).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn transcript_order_and_list_q_over_meeting_app() {
    let (app, db, _dir) = setup().await;
    let id = db
        .insert_meeting("Microsoft Teams", "ui_scan", Some("Retro"), None)
        .await
        .unwrap();
    let now = chrono::Utc::now();
    for (i, text) in ["first", "second"].iter().enumerate() {
        db.insert_meeting_transcript_segment(
            id,
            "deepgram",
            None,
            &format!("item-{i}"),
            "Mic",
            "input",
            None,
            text,
            now + chrono::Duration::seconds(i as i64),
        )
        .await
        .unwrap();
    }

    let (status, rows) = call(&app, "GET", &format!("/meetings/{id}/transcript"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rows[0]["transcript"], "first");
    assert!(rows[0].get("audioStartTimeSecs").is_some());
    assert!(rows[0].get("sessionSpeakerId").is_some());
    let (_, rows) = call(
        &app,
        "GET",
        &format!("/meetings/{id}/transcript?order=descending"),
        None,
    )
    .await;
    assert_eq!(rows[0]["transcript"], "second");
    let (status, _) = call(
        &app,
        "GET",
        &format!("/meetings/{id}/transcript?order=sideways"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, list) = call(&app, "GET", "/meetings?q=teams", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], id);
}

#[test]
fn bundled_meeting_summary_pipe_saves_through_the_summary_endpoint() {
    let content = include_str!("../../screenpipe-core/assets/pipes/meeting-summary/pipe.md");
    let (config, body) = screenpipe_core::pipes::parse_frontmatter(content).unwrap();
    assert!(config.enabled);
    assert!(config
        .trigger
        .as_ref()
        .is_some_and(|t| t.events.iter().any(|e| e == "meeting_ended")));
    // No vendor preset: the run uses the user's own default AI preset.
    assert!(config.preset.is_empty(), "{:?}", config.preset);
    assert!(body.contains("/meetings/<ID>/summary"));
    assert!(body.contains("## Summary"));
    assert!(!body.contains("PATCH"));
}
