// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! `/search` v2 through the real engine router with API auth on (W3-02):
//! parsed content, starred filters and flags, related tags, the parsed-only
//! filters and the new error answers.
//!
//! Fixtures are synthetic rows in an in-memory database; nothing binds a
//! port or touches a real data dir.

use std::{net::SocketAddr, sync::Arc};

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use chrono::{DateTime, Duration, TimeZone, Utc};
use screenpipe_audio::audio_manager::AudioManagerBuilder;
use screenpipe_db::ext::semantic::{
    NewSemanticItem, NewSemanticRun, SemanticAppPlatform, SemanticFingerprint,
    SemanticIdentityQuality, SemanticItemKind, SemanticRunStatus,
};
use screenpipe_db::ext::starred::{StarredSaveOutcome, StarredSessionWrite};
use screenpipe_db::{DatabaseManager, TagContentType};
use screenpipe_engine::SCServer;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const KEY: &str = "fp-search-v2-test-key";
const SESSION_A: &str = "6f1c1d4e-2b8a-4f0e-9a51-0d6c7f3b2a10";
const SESSION_B: &str = "0b7e9a2c-5d43-4e8f-8c21-7a9d3e6f1b54";
const UNKNOWN_SESSION: &str = "c3a8f5e1-7b2d-4c69-a0e4-58d1b2f97c3e";

struct Fixture {
    router: Router,
    db: Arc<DatabaseManager>,
    _dir: tempfile::TempDir,
    /// Frame ids by minute after 09:00.
    frames: Vec<i64>,
    chat_frame: i64,
    notes_frame: i64,
    ada: i64,
}

/// 2026-10-01 09:00 UTC plus `m` minutes.
fn at(m: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap() + Duration::minutes(m)
}

fn fp(tag: &str) -> SemanticFingerprint {
    Sha256::digest(tag.as_bytes()).into()
}

fn item(
    local: &str,
    parent: Option<&str>,
    kind: SemanticItemKind,
    title: Option<&str>,
    body: Option<&str>,
    actor: Option<&str>,
) -> NewSemanticItem {
    let mut item = NewSemanticItem::new(
        local,
        kind,
        format!("key:{local}"),
        SemanticIdentityQuality::Derived,
        fp(&format!("entity:{local}")),
        fp(&format!("version:{local}")),
    );
    item.parent_local_id = parent.map(str::to_string);
    item.title = title.map(str::to_string);
    item.body = body.map(str::to_string);
    item.actor = actor.map(str::to_string);
    item
}

fn run(input: &str, items: Vec<NewSemanticItem>) -> NewSemanticRun {
    NewSemanticRun {
        input_fingerprint: fp(&format!("input:{input}")),
        parser_id: "chat.test".into(),
        parser_version: "1.2.0".into(),
        schema_version: 1,
        app_platform: SemanticAppPlatform::Windows,
        app_id: Some("Chat.exe".into()),
        app_executable: Some("C:\\Program Files\\Chat\\Chat.exe".into()),
        app_version: Some("4.2".into()),
        input_content_hash: [7; 8],
        status: SemanticRunStatus::Handled,
        text_bytes: 120,
        parse_duration_us: 900,
        actor_source_key: None,
        items,
    }
}

async fn frame(db: &DatabaseManager, ts: DateTime<Utc>, app: &str, text: &str) -> i64 {
    db.insert_snapshot_frame(
        "monitor_1",
        ts,
        &format!("/tmp/fp-search-v2-route/{}.jpg", ts.timestamp_millis()),
        Some(app),
        Some("Main window"),
        None,
        true,
        Some("click"),
        Some(text),
        Some("accessibility"),
        None,
        None,
        None,
    )
    .await
    .expect("insert frame")
}

async fn star(db: &DatabaseManager, id: &str, start: DateTime<Utc>, end: DateTime<Utc>) {
    let outcome = db
        .save_starred_session(&StarredSessionWrite {
            id,
            start,
            end,
            hd_requested: false,
            expected_revision: 0,
        })
        .await
        .unwrap();
    assert!(
        matches!(outcome, StarredSaveOutcome::Saved(_)),
        "{outcome:?}"
    );
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(
        DatabaseManager::new("sqlite::memory:", Default::default())
            .await
            .unwrap(),
    );
    let audio_manager = Arc::new(
        AudioManagerBuilder::new()
            .output_path(dir.path().join("audio"))
            .is_disabled(true)
            .build(db.clone())
            .await
            .unwrap(),
    );
    let mut server = SCServer::new(
        db.clone(),
        SocketAddr::from(([127, 0, 0, 1], 3035)),
        dir.path().to_path_buf(),
        false,
        false,
        audio_manager,
        false,
        "balanced".to_string(),
    );
    server.api_auth = true;
    server.api_auth_key = Some(KEY.to_string());
    let router = server.create_router().await;

    // Screen text "alpha N" at 09:00 .. 09:09.
    let mut frames = Vec::new();
    for m in 0..10 {
        frames.push(frame(&db, at(m), "Editor", &format!("alpha {m}")).await);
    }
    // Starred: [09:02, 09:04) and [09:07, 09:08).
    star(&db, SESSION_A, at(2), at(4)).await;
    star(&db, SESSION_B, at(7), at(8)).await;

    // Parsed content on two frames at 09:20 and 09:21.
    let chat_frame = frame(&db, at(20), "Chat", "chat frame").await;
    let notes_frame = frame(&db, at(21), "Notes", "notes frame").await;
    db.store_semantic_run_for_frame(
        chat_frame,
        &run(
            "chat",
            vec![
                item(
                    "c",
                    None,
                    SemanticItemKind::Conversation,
                    Some("Launch"),
                    None,
                    None,
                ),
                item(
                    "m1",
                    Some("c"),
                    SemanticItemKind::Message,
                    None,
                    Some("ship xylophone on friday"),
                    Some("Ada"),
                ),
                item(
                    "m2",
                    Some("c"),
                    SemanticItemKind::Message,
                    None,
                    Some("agreed"),
                    Some("Bob"),
                ),
            ],
        ),
    )
    .await
    .unwrap()
    .expect("chat run");
    db.store_semantic_run_for_frame(
        notes_frame,
        &run(
            "notes",
            vec![item(
                "d",
                None,
                SemanticItemKind::Document,
                Some("Budget"),
                Some("xylophone budget for the quarter"),
                None,
            )],
        ),
    )
    .await
    .unwrap()
    .expect("notes run");
    let ada = db
        .search_semantic_actors("Ada", 10, 0)
        .await
        .unwrap()
        .into_iter()
        .find(|a| a.name == "Ada")
        .expect("Ada")
        .id;

    // Tags for the related map.
    for (minute, tags) in [
        (1, vec!["project:atlas", "person:ada"]),
        (2, vec!["project:atlas", "person:ada", "person:bob"]),
        (5, vec!["project:atlas", "urgent"]),
    ] {
        db.add_tags(
            frames[minute],
            TagContentType::Vision,
            tags.into_iter().map(str::to_string).collect(),
        )
        .await
        .unwrap();
    }

    Fixture {
        router,
        db,
        _dir: dir,
        frames,
        chat_frame,
        notes_frame,
        ada,
    }
}

impl Fixture {
    async fn api_text(&self, uri: &str) -> (StatusCode, String) {
        let request = Request::builder()
            .method("GET")
            .uri(uri)
            .header("authorization", format!("Bearer {KEY}"))
            .body(Body::empty())
            .unwrap();
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn api(&self, uri: &str) -> (StatusCode, Value) {
        let request = Request::builder()
            .method("GET")
            .uri(uri)
            .header("authorization", format!("Bearer {KEY}"))
            .body(Body::empty())
            .unwrap();
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}

fn types(body: &Value) -> Vec<String> {
    body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit["type"].as_str().unwrap().to_string())
        .collect()
}

fn frame_ids(body: &Value) -> Vec<i64> {
    body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit["content"]["frame_id"].as_i64().unwrap())
        .collect()
}

// ── parsed content ──────────────────────────────────────────────────────

#[tokio::test]
async fn parsed_search_returns_parsed_items_and_all_returns_none() {
    let fx = fixture().await;

    let (status, body) = fx.api("/search?content_type=parsed&q=xylophone").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(types(&body), vec!["Parsed", "Parsed"]);
    assert_eq!(frame_ids(&body), vec![fx.notes_frame, fx.chat_frame]);
    assert_eq!(body["pagination"]["total"], 2);

    let chat = &body["data"][1]["content"];
    assert_eq!(chat["app_name"], "Chat");
    assert_eq!(chat["parser_id"], "chat.test");
    assert_eq!(chat["parser_version"], "1.2.0");
    assert_eq!(chat["app_platform"], "windows");
    assert_eq!(chat["app_id"], "Chat.exe");
    assert_eq!(chat["parse_duration_us"], 900);
    assert_eq!(chat["text_bytes"], 120);
    assert!(chat["run_id"].as_i64().unwrap() > 0);
    assert_eq!(
        chat["text"],
        "Launch\n  Ada: ship xylophone on friday\n  Bob: agreed"
    );
    let items = chat["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[1]["local_id"], "m1");
    assert_eq!(items[1]["parent_local_id"], "c");
    assert_eq!(items[1]["kind"], "message");
    assert_eq!(items[1]["identity_quality"], "derived");
    assert_eq!(items[1]["actor"], "Ada");
    let actors = chat["actors"].as_array().unwrap();
    assert_eq!(actors.len(), 2, "one reference per item with an actor");
    assert_eq!(actors[0]["local_id"], "m1");
    assert_eq!(actors[0]["actor_id"], fx.ada);
    assert_eq!(actors[0]["name"], "Ada");
    assert_eq!(actors[0]["observed_name"], "Ada");
    assert_eq!(actors[0]["assignment_source"], "heuristic");
    assert_eq!(actors[0]["item_id"], items[1]["item_id"]);

    // `all` (and the default) never return parsed items.
    for uri in ["/search?content_type=all&limit=50", "/search?limit=50"] {
        let (status, body) = fx.api(uri).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!types(&body).is_empty());
        assert!(!types(&body).iter().any(|t| t == "Parsed"), "{uri}: {body}");
    }
}

#[tokio::test]
async fn actor_and_frame_filters_narrow_parsed_results() {
    let fx = fixture().await;

    let (status, body) = fx
        .api(&format!("/search?content_type=parsed&actor_id={}", fx.ada))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(frame_ids(&body), vec![fx.chat_frame]);
    let content = &body["data"][0]["content"];
    let locals: Vec<&str> = content["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["local_id"].as_str().unwrap())
        .collect();
    assert_eq!(locals, vec!["c", "m1"], "Ada's item and its conversation");
    assert_eq!(content["actors"].as_array().unwrap().len(), 1);
    assert_eq!(content["text"], "Launch\n  Ada: ship xylophone on friday");

    let (_, body) = fx
        .api(&format!(
            "/search?content_type=parsed&frame_id={}",
            fx.notes_frame
        ))
        .await;
    assert_eq!(frame_ids(&body), vec![fx.notes_frame]);
    assert_eq!(body["pagination"]["total"], 1);

    for (uri, status) in [
        (
            "/search?content_type=ocr&frame_id=1",
            StatusCode::BAD_REQUEST,
        ),
        ("/search?actor_id=1", StatusCode::BAD_REQUEST),
        (
            "/search?content_type=parsed&actor_id=0",
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let (got, body) = fx.api(uri).await;
        assert_eq!(got, status, "{uri}: {body}");
        assert!(body["error"].as_str().is_some(), "{uri}: {body}");
    }
}

// ── starred ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn starred_only_returns_full_pages_of_starred_hits() {
    let fx = fixture().await;
    let inside = [fx.frames[7], fx.frames[3], fx.frames[2]];

    let (status, page1) = fx
        .api("/search?q=alpha&content_type=ocr&starred_only=true&limit=2&offset=0")
        .await;
    assert_eq!(status, StatusCode::OK, "{page1}");
    let (_, page2) = fx
        .api("/search?q=alpha&content_type=ocr&starred_only=true&limit=2&offset=2")
        .await;
    assert_eq!(
        frame_ids(&page1),
        inside[..2].to_vec(),
        "the filter runs before paging"
    );
    assert_eq!(frame_ids(&page2), inside[2..].to_vec());
    assert_eq!(page1["pagination"]["total"], 3);
    for hit in page1["data"]
        .as_array()
        .unwrap()
        .iter()
        .chain(page2["data"].as_array().unwrap())
    {
        assert_eq!(hit["starred"], true, "{hit}");
    }

    // One session only.
    let (_, body) = fx
        .api(&format!(
            "/search?q=alpha&content_type=ocr&starred_session_id={SESSION_B}"
        ))
        .await;
    assert_eq!(frame_ids(&body), vec![fx.frames[7]]);

    // Without the filter every hit says whether it is starred.
    let (_, body) = fx.api("/search?q=alpha&content_type=ocr&limit=20").await;
    assert_eq!(body["pagination"]["total"], 10);
    for hit in body["data"].as_array().unwrap() {
        let id = hit["content"]["frame_id"].as_i64().unwrap();
        assert_eq!(hit["starred"], inside.contains(&id), "{hit}");
    }
    // A second, cached request recomputes the same flags.
    let (_, again) = fx.api("/search?q=alpha&content_type=ocr&limit=20").await;
    assert_eq!(body, again);
}

#[tokio::test]
async fn starred_filter_errors_are_clear() {
    let fx = fixture().await;
    let (status, body) = fx.api("/search?starred_session_id=not-a-uuid").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = fx
        .api(&format!("/search?starred_session_id={UNKNOWN_SESSION}"))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // More than 100 separate starred intervals in range.
    for i in 0..101i64 {
        let id = format!("00000000-0000-4000-8000-{:012}", i);
        let start = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap() + Duration::minutes(i * 10);
        star(&fx.db, &id, start, start + Duration::minutes(5)).await;
    }
    let (status, body) = fx
        .api("/search?starred_only=true&start_time=2026-09-01T00:00:00Z&end_time=2026-09-02T00:00:00Z")
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("start_time"),
        "{body}"
    );
    // A narrower range is accepted.
    let (status, _) = fx
        .api("/search?starred_only=true&start_time=2026-09-01T00:00:00Z&end_time=2026-09-01T01:00:00Z")
        .await;
    assert_eq!(status, StatusCode::OK);
}

// ── related tags ────────────────────────────────────────────────────────

#[tokio::test]
async fn include_related_adds_the_related_map_only_with_tags() {
    let fx = fixture().await;
    let (status, body) = fx
        .api("/search?content_type=ocr&tags=project:atlas&include_related=true")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["pagination"]["total"], 3);
    assert_eq!(
        body["related"],
        serde_json::json!({"person": ["person:ada", "person:bob"], "other": ["urgent"]})
    );

    // Starred windows narrow the related items too.
    let (_, body) = fx
        .api("/search?content_type=ocr&tags=project:atlas&include_related=true&starred_only=true")
        .await;
    assert_eq!(
        body["related"],
        serde_json::json!({"person": ["person:ada", "person:bob"]})
    );

    // Without tags, or without the flag, the key is absent.
    for uri in [
        "/search?content_type=ocr&include_related=true",
        "/search?content_type=ocr&tags=project:atlas",
    ] {
        let (_, body) = fx.api(uri).await;
        assert!(body.get("related").is_none(), "{uri}: {body}");
    }
}

#[tokio::test]
async fn plain_search_keeps_its_envelope() {
    let fx = fixture().await;
    let (status, body) = fx.api("/search?q=alpha&content_type=ocr&limit=3").await;
    assert_eq!(status, StatusCode::OK);
    let keys: Vec<&String> = body.as_object().unwrap().keys().collect();
    assert_eq!(keys, vec!["data", "pagination"]);
    // `serde_json::Value` keeps keys sorted.
    let hit = body["data"][0].as_object().unwrap();
    let keys: Vec<&String> = hit.keys().collect();
    assert_eq!(keys, vec!["content", "starred", "type"]);

    // `fields` and `format` treat the flag like any other field. Frames at
    // 09:09, 09:08 and 09:07 come back; only 09:07 is starred.
    let (status, body) = fx
        .api("/search?q=alpha&content_type=ocr&limit=3&fields=type,content.frame_id,starred")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows: Vec<(String, i64, bool)> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["type"].as_str().unwrap().to_string(),
                row["content.frame_id"].as_i64().unwrap(),
                row["starred"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            ("OCR".to_string(), fx.frames[9], false),
            ("OCR".to_string(), fx.frames[8], false),
            ("OCR".to_string(), fx.frames[7], true),
        ]
    );
    let (status, csv) = fx
        .api_text(
            "/search?q=alpha&content_type=ocr&limit=3&format=csv&fields=content.frame_id,starred",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{csv}");
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines[0], "content.frame_id,starred");
    assert_eq!(lines[3], format!("{},true", fx.frames[7]));
}
