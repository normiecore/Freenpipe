// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Search v2 in the database layer (W3-02): the search scope (time windows
//! for starred sessions, the input-context filter, the parsed filters),
//! parsed-content search and related tags.
//!
//! Every test builds synthetic rows in its own temporary database.

use chrono::{DateTime, Duration, TimeZone, Utc};
use screenpipe_db::ext::search_parsed::RelatedTagSources;
use screenpipe_db::ext::semantic::{
    NewSemanticItem, NewSemanticRun, SemanticAppPlatform, SemanticFingerprint,
    SemanticIdentityQuality, SemanticItemKind, SemanticRunStatus,
};
use screenpipe_db::{
    AudioDevice, ContentType, DatabaseManager, DeviceType, InsertUiEvent, SearchResult,
    SearchScope, SearchWindows, TagContentType, UiEventType,
};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

async fn open_db() -> (TempDir, DatabaseManager) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("db.sqlite");
    let db = DatabaseManager::new(path.to_str().unwrap(), Default::default())
        .await
        .expect("DatabaseManager::new");
    (dir, db)
}

/// 2026-10-01 09:00 UTC plus `m` minutes.
fn at(m: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap() + Duration::minutes(m)
}

async fn frame(db: &DatabaseManager, ts: DateTime<Utc>, app: &str, text: &str) -> i64 {
    db.insert_snapshot_frame(
        "monitor_1",
        ts,
        &format!("/tmp/fp-search-v2/{}.jpg", ts.timestamp_millis()),
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

/// One frame a minute from 09:00 to 09:09, text `alpha N`.
async fn ten_frames(db: &DatabaseManager) -> Vec<i64> {
    let mut ids = Vec::new();
    for m in 0..10 {
        ids.push(frame(db, at(m), "Editor", &format!("alpha {m}")).await);
    }
    ids
}

/// Starred time: [09:02, 09:04) and [09:07, 09:08).
fn starred() -> SearchWindows {
    SearchWindows::new([(at(2), at(4)), (at(7), at(8))])
}

fn scope_with(windows: SearchWindows) -> SearchScope {
    SearchScope {
        windows: Some(windows),
        ..Default::default()
    }
}

async fn search(
    db: &DatabaseManager,
    q: &str,
    content_type: ContentType,
    limit: u32,
    offset: u32,
    scope: &SearchScope,
) -> Vec<SearchResult> {
    db.search_with_scope(
        q,
        content_type,
        limit,
        offset,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &[],
        scope,
    )
    .await
    .expect("search")
}

async fn count(db: &DatabaseManager, q: &str, content_type: ContentType, scope: &SearchScope) -> usize {
    db.count_search_results_with_scope(
        q,
        content_type,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &[],
        scope,
        None,
    )
    .await
    .expect("count")
}

fn frame_ids(results: &[SearchResult]) -> Vec<i64> {
    results
        .iter()
        .map(|r| match r {
            SearchResult::OCR(o) => o.frame_id,
            SearchResult::UI(u) => u.id,
            SearchResult::Parsed(p) => p.frame_id,
            other => panic!("unexpected hit {other:?}"),
        })
        .collect()
}

// ── the default scope ───────────────────────────────────────────────────

#[tokio::test]
async fn the_default_scope_returns_what_search_with_tags_returns() {
    let (_dir, db) = open_db().await;
    ten_frames(&db).await;
    for content_type in [ContentType::All, ContentType::OCR, ContentType::Accessibility] {
        for (limit, offset) in [(3, 0), (3, 3), (20, 0)] {
            let before = db
                .search_with_tags(
                    "alpha",
                    content_type.clone(),
                    limit,
                    offset,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    &[],
                )
                .await
                .unwrap();
            let now = search(
                &db,
                "alpha",
                content_type.clone(),
                limit,
                offset,
                &SearchScope::default(),
            )
            .await;
            assert_eq!(frame_ids(&before), frame_ids(&now), "{content_type:?}");
        }
        let total_before = db
            .count_search_results_with_tags(
                "alpha",
                content_type.clone(),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                &[],
            )
            .await
            .unwrap();
        assert_eq!(
            total_before,
            count(&db, "alpha", content_type.clone(), &SearchScope::default()).await
        );
    }
}

// ── time windows (starred sessions) ─────────────────────────────────────

#[tokio::test]
async fn windows_filter_frames_before_pagination() {
    let (_dir, db) = open_db().await;
    let ids = ten_frames(&db).await;
    let scope = scope_with(starred());
    let inside = vec![ids[7], ids[3], ids[2]];

    let page1 = search(&db, "alpha", ContentType::OCR, 2, 0, &scope).await;
    let page2 = search(&db, "alpha", ContentType::OCR, 2, 2, &scope).await;
    assert_eq!(frame_ids(&page1), inside[..2].to_vec(), "a full first page");
    assert_eq!(frame_ids(&page2), inside[2..].to_vec());
    assert_eq!(count(&db, "alpha", ContentType::OCR, &scope).await, 3);

    // The other frame-backed content types and the empty query agree.
    let ax = search(&db, "", ContentType::Accessibility, 20, 0, &scope).await;
    assert_eq!(frame_ids(&ax), inside);
    assert_eq!(count(&db, "", ContentType::Accessibility, &scope).await, 3);
    let all = search(&db, "alpha", ContentType::All, 20, 0, &scope).await;
    assert!(all
        .iter()
        .all(|r| matches!(r, SearchResult::OCR(_) | SearchResult::UI(_))));
    assert!(frame_ids(&all).iter().all(|id| inside.contains(id)));
    assert_eq!(count(&db, "alpha", ContentType::All, &scope).await, 3);

    // No windows at all matches nothing.
    let none = scope_with(SearchWindows::default());
    assert!(search(&db, "alpha", ContentType::All, 20, 0, &none).await.is_empty());
    assert_eq!(count(&db, "alpha", ContentType::OCR, &none).await, 0);
}

#[tokio::test]
async fn windows_are_half_open_and_compare_instants_not_spellings() {
    let (_dir, db) = open_db().await;
    let at_start = frame(&db, at(2), "Editor", "edge start").await;
    let at_end = frame(&db, at(4), "Editor", "edge end").await;
    let inside = frame(&db, at(3), "Editor", "edge inside").await;
    let just_before = frame(&db, at(2) - Duration::milliseconds(1), "Editor", "edge before").await;
    // Another writer's spelling of an instant inside the window.
    sqlx::query("UPDATE frames SET timestamp = '2026-10-01T09:03:00.000Z' WHERE id = ?1")
        .bind(inside)
        .execute(db.write_pool())
        .await
        .unwrap();
    let scope = scope_with(SearchWindows::new([(at(2), at(4))]));
    let hits = frame_ids(&search(&db, "edge", ContentType::OCR, 20, 0, &scope).await);
    assert!(hits.contains(&at_start), "the start is inside");
    assert!(hits.contains(&inside), "a Z-suffixed timestamp still matches");
    assert!(!hits.contains(&at_end), "the end is outside");
    assert!(!hits.contains(&just_before));
    assert_eq!(hits.len(), 2);
}

#[tokio::test]
async fn windows_filter_audio_and_input_and_never_match_memories() {
    let (_dir, db) = open_db().await;
    let device = AudioDevice {
        name: "test-mic (input)".to_string(),
        device_type: DeviceType::Input,
    };
    for m in [1, 3, 7, 9] {
        let chunk = db
            .insert_audio_chunk(&format!("/tmp/fp-search-v2/audio-{m}.mp4"), Some(at(m)))
            .await
            .unwrap();
        db.insert_audio_transcription(
            chunk,
            &format!("spoken words at minute {m}"),
            0,
            "test",
            &device,
            None,
            Some(0.0),
            Some(2.0),
            Some(at(m)),
        )
        .await
        .unwrap();
    }
    let scope = scope_with(starred());
    let audio = search(&db, "spoken", ContentType::Audio, 20, 0, &scope).await;
    let minutes: Vec<DateTime<Utc>> = audio.iter().map(|r| r.sort_timestamp()).collect();
    assert_eq!(minutes, vec![at(7), at(3)]);
    assert_eq!(count(&db, "spoken", ContentType::Audio, &scope).await, 2);
    assert_eq!(
        count(&db, "spoken", ContentType::Audio, &SearchScope::default()).await,
        4
    );

    for m in [2, 5] {
        db.insert_ui_event(&ui_event(at(m), UiEventType::Text, Some("typed words"), None))
            .await
            .unwrap();
    }
    let input = search(&db, "", ContentType::Input, 20, 0, &scope).await;
    assert_eq!(input.len(), 1);
    assert_eq!(input[0].sort_timestamp(), at(2));
    assert_eq!(count(&db, "", ContentType::Input, &scope).await, 1);

    sqlx::query(
        "INSERT INTO memories (content, source, tags, importance, created_at, updated_at) \
         VALUES ('remember the budget', 'user', '[]', 0.5, '2026-10-01T09:03:00.000Z', \
                 '2026-10-01T09:03:00.000Z')",
    )
    .execute(db.write_pool())
    .await
    .unwrap();
    assert_eq!(
        search(&db, "", ContentType::Memory, 20, 0, &SearchScope::default())
            .await
            .len(),
        1
    );
    assert!(search(&db, "", ContentType::Memory, 20, 0, &scope).await.is_empty());
    assert_eq!(count(&db, "", ContentType::Memory, &scope).await, 0);
}

fn ui_event(
    ts: DateTime<Utc>,
    event_type: UiEventType,
    text: Option<&str>,
    element: Option<&str>,
) -> InsertUiEvent {
    InsertUiEvent {
        timestamp: ts,
        session_id: Some("search-v2".to_string()),
        relative_ms: 0,
        event_type,
        x: Some(10),
        y: Some(20),
        delta_x: None,
        delta_y: None,
        button: None,
        click_count: Some(2),
        key_code: None,
        modifiers: None,
        text_content: text.map(str::to_string),
        app_name: Some("Editor".to_string()),
        app_pid: Some(42),
        window_title: Some("Main window".to_string()),
        browser_url: None,
        element_role: element.map(|_| "AXButton".to_string()),
        element_name: element.map(str::to_string),
        element_value: None,
        element_description: None,
        element_automation_id: None,
        element_bounds: None,
        element_ancestors: None,
        frame_id: None,
    }
}

// ── input context ───────────────────────────────────────────────────────

#[tokio::test]
async fn input_context_only_keeps_text_and_named_elements() {
    let (_dir, db) = open_db().await;
    let typed = db
        .insert_ui_event(&ui_event(at(1), UiEventType::Text, Some("hello"), None))
        .await
        .unwrap();
    let clicked = db
        .insert_ui_event(&ui_event(at(2), UiEventType::Click, None, Some("Save")))
        .await
        .unwrap();
    db.insert_ui_event(&ui_event(at(3), UiEventType::Move, None, None))
        .await
        .unwrap();
    db.insert_ui_event(&ui_event(at(4), UiEventType::Scroll, Some("   "), Some("")))
        .await
        .unwrap();

    let all = search(&db, "", ContentType::Input, 20, 0, &SearchScope::default()).await;
    assert_eq!(all.len(), 4);
    let scope = SearchScope {
        input_context_only: true,
        ..Default::default()
    };
    let kept: Vec<i64> = search(&db, "", ContentType::Input, 20, 0, &scope)
        .await
        .iter()
        .map(|r| match r {
            SearchResult::Input(e) => e.id,
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(kept, vec![clicked, typed]);
    assert_eq!(count(&db, "", ContentType::Input, &scope).await, 2);
    // The click count comes back with the event.
    match &search(&db, "", ContentType::Input, 1, 0, &scope).await[0] {
        SearchResult::Input(e) => assert_eq!(e.click_count, Some(2)),
        other => panic!("unexpected {other:?}"),
    }
}

// ── parsed content ──────────────────────────────────────────────────────

fn fp(tag: &str) -> SemanticFingerprint {
    Sha256::digest(tag.as_bytes()).into()
}

fn item(
    local: &str,
    parent: Option<&str>,
    kind: SemanticItemKind,
    tag: &str,
    title: Option<&str>,
    body: Option<&str>,
    actor: Option<&str>,
) -> NewSemanticItem {
    let mut item = NewSemanticItem::new(
        local,
        kind,
        format!("key:{tag}"),
        SemanticIdentityQuality::Derived,
        fp(&format!("entity:{tag}")),
        fp(&format!("version:{tag}")),
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
        status: if items.is_empty() {
            SemanticRunStatus::Empty
        } else {
            SemanticRunStatus::Handled
        },
        text_bytes: 120,
        parse_duration_us: 900,
        actor_source_key: None,
        items,
    }
}

struct ParsedFixture {
    chat: i64,
    notes: i64,
    plain: i64,
    empty_run: i64,
    ada: i64,
}

async fn parsed_fixture(db: &DatabaseManager) -> ParsedFixture {
    let chat = frame(db, at(1), "Chat", "chat frame").await;
    let notes = frame(db, at(2), "Notes", "notes frame").await;
    let plain = frame(db, at(3), "Editor", "plain frame").await;
    let empty_run = frame(db, at(4), "Chat", "nothing parsed").await;
    db.store_semantic_run_for_frame(
        chat,
        &run(
            "chat",
            vec![
                item("c", None, SemanticItemKind::Conversation, "conv", Some("Launch"), None, None),
                item(
                    "m1",
                    Some("c"),
                    SemanticItemKind::Message,
                    "m1",
                    None,
                    Some("ship it on friday"),
                    Some("Ada"),
                ),
                item(
                    "m2",
                    Some("c"),
                    SemanticItemKind::Message,
                    "m2",
                    None,
                    Some("agreed"),
                    Some("Bob"),
                ),
            ],
        ),
    )
    .await
    .unwrap()
    .expect("chat run stored");
    db.store_semantic_run_for_frame(
        notes,
        &run(
            "notes",
            vec![item(
                "d",
                None,
                SemanticItemKind::Document,
                "doc",
                Some("Budget plan"),
                Some("quarterly budget numbers"),
                None,
            )],
        ),
    )
    .await
    .unwrap()
    .expect("notes run stored");
    db.store_semantic_run_for_frame(empty_run, &run("empty", vec![]))
        .await
        .unwrap()
        .expect("empty run stored");
    let ada = db
        .search_semantic_actors("Ada", 10, 0)
        .await
        .unwrap()
        .into_iter()
        .find(|a| a.name == "Ada")
        .expect("heuristic actor for Ada")
        .id;
    ParsedFixture {
        chat,
        notes,
        plain,
        empty_run,
        ada,
    }
}

fn parsed(results: &[SearchResult]) -> Vec<&screenpipe_db::ext::search_parsed::ParsedFrameResult> {
    results
        .iter()
        .map(|r| match r {
            SearchResult::Parsed(p) => p,
            other => panic!("expected parsed hits only, got {other:?}"),
        })
        .collect()
}

#[tokio::test]
async fn parsed_search_returns_handled_runs_with_items_and_actors() {
    let (_dir, db) = open_db().await;
    let fx = parsed_fixture(&db).await;
    let none = SearchScope::default();

    let browse = search(&db, "", ContentType::Parsed, 20, 0, &none).await;
    let hits = parsed(&browse);
    assert_eq!(
        hits.iter().map(|p| p.frame_id).collect::<Vec<_>>(),
        vec![fx.notes, fx.chat],
        "newest first; no frame without a run, no empty run"
    );
    assert_eq!(count(&db, "", ContentType::Parsed, &none).await, 2);
    assert!(!hits.iter().any(|p| p.frame_id == fx.plain || p.frame_id == fx.empty_run));

    let chat = hits[1];
    assert_eq!(chat.app_name, "Chat");
    assert_eq!(chat.parser_id, "chat.test");
    assert_eq!(chat.parser_version, "1.2.0");
    assert_eq!(chat.app_platform, "windows");
    assert_eq!(chat.app_id.as_deref(), Some("Chat.exe"));
    assert_eq!(chat.parse_duration_us, 900);
    assert_eq!(chat.text_bytes, 120);
    let locals: Vec<&str> = chat.items.iter().map(|i| i.local_id.as_str()).collect();
    assert_eq!(locals, vec!["c", "m1", "m2"], "run order");
    assert_eq!(chat.items[1].kind, "message");
    assert_eq!(chat.items[1].parent_local_id.as_deref(), Some("c"));
    assert_eq!(chat.items[1].actor.as_deref(), Some("Ada"));
    assert_eq!(chat.items[1].assigned_actor_id, Some(fx.ada));
    assert_eq!(chat.items[1].assigned_actor_name.as_deref(), Some("Ada"));
    assert_eq!(chat.items[1].assignment_source.as_deref(), Some("heuristic"));
    assert_eq!(chat.items[0].assigned_actor_id, None);

    // Text search goes through the item index.
    let budget = search(&db, "budget", ContentType::Parsed, 20, 0, &none).await;
    assert_eq!(parsed(&budget).iter().map(|p| p.frame_id).collect::<Vec<_>>(), vec![fx.notes]);
    assert_eq!(count(&db, "budget", ContentType::Parsed, &none).await, 1);
    assert!(search(&db, "nowhere", ContentType::Parsed, 20, 0, &none).await.is_empty());

    // Parsed items are never part of `all`.
    let all = search(&db, "", ContentType::All, 50, 0, &none).await;
    assert!(!all.is_empty());
    assert!(!all.iter().any(|r| matches!(r, SearchResult::Parsed(_))));
}

#[tokio::test]
async fn parsed_filters_by_frame_actor_and_windows() {
    let (_dir, db) = open_db().await;
    let fx = parsed_fixture(&db).await;

    let one = SearchScope {
        frame_id: Some(fx.chat),
        ..Default::default()
    };
    let hits = search(&db, "", ContentType::Parsed, 20, 0, &one).await;
    assert_eq!(parsed(&hits).iter().map(|p| p.frame_id).collect::<Vec<_>>(), vec![fx.chat]);
    assert_eq!(count(&db, "", ContentType::Parsed, &one).await, 1);
    let missing = SearchScope {
        frame_id: Some(fx.plain),
        ..Default::default()
    };
    assert!(search(&db, "", ContentType::Parsed, 20, 0, &missing).await.is_empty());

    let by_ada = SearchScope {
        actor_id: Some(fx.ada),
        ..Default::default()
    };
    let hits = search(&db, "", ContentType::Parsed, 20, 0, &by_ada).await;
    let hits = parsed(&hits);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].frame_id, fx.chat);
    let locals: Vec<&str> = hits[0].items.iter().map(|i| i.local_id.as_str()).collect();
    assert_eq!(locals, vec!["c", "m1"], "Ada's message and its conversation");
    assert_eq!(count(&db, "", ContentType::Parsed, &by_ada).await, 1);
    // Text and actor must meet on one item.
    assert!(search(&db, "agreed", ContentType::Parsed, 20, 0, &by_ada).await.is_empty());
    assert_eq!(
        search(&db, "friday", ContentType::Parsed, 20, 0, &by_ada).await.len(),
        1
    );

    let windows = scope_with(SearchWindows::new([(at(2), at(3))]));
    let hits = search(&db, "", ContentType::Parsed, 20, 0, &windows).await;
    assert_eq!(parsed(&hits).iter().map(|p| p.frame_id).collect::<Vec<_>>(), vec![fx.notes]);
    assert_eq!(count(&db, "", ContentType::Parsed, &windows).await, 1);
}

// ── related tags ────────────────────────────────────────────────────────

#[tokio::test]
async fn related_tags_group_co_occurring_tags_by_namespace() {
    let (_dir, db) = open_db().await;
    let ids = ten_frames(&db).await;
    let tag = |id: i64, tags: &[&str]| {
        let db = &db;
        let tags: Vec<String> = tags.iter().map(|t| t.to_string()).collect();
        async move { db.add_tags(id, TagContentType::Vision, tags).await.unwrap() }
    };
    tag(ids[1], &["project:atlas", "person:ada", "urgent"]).await;
    tag(ids[2], &["project:atlas", "person:ada", "person:bob"]).await;
    tag(ids[3], &["project:atlas", "person:bob"]).await;
    tag(ids[5], &["project:atlas", "person:ada"]).await;
    tag(ids[6], &["person:carol"]).await;

    let frames = RelatedTagSources {
        frames: true,
        ..Default::default()
    };
    let wanted = vec!["project:atlas".to_string()];
    let related = db
        .related_search_tags(&wanted, frames, None, None, None)
        .await
        .unwrap();
    assert_eq!(related["person"], vec!["person:ada", "person:bob"]);
    assert_eq!(related["other"], vec!["urgent"]);
    assert!(!related.contains_key("project"), "requested tags are not listed");

    // The time range and the windows narrow the items it is computed over.
    let related = db
        .related_search_tags(&wanted, frames, Some(at(3)), None, None)
        .await
        .unwrap();
    assert_eq!(related["person"], vec!["person:ada", "person:bob"]);
    assert!(!related.contains_key("other"));
    let related = db
        .related_search_tags(&wanted, frames, None, None, Some(&SearchWindows::new([(at(3), at(4))])))
        .await
        .unwrap();
    assert_eq!(related["person"], vec!["person:bob"]);

    // No tag on audio, and nothing without requested tags.
    let audio = RelatedTagSources {
        audio: true,
        ..Default::default()
    };
    assert!(db
        .related_search_tags(&wanted, audio, None, None, None)
        .await
        .unwrap()
        .is_empty());
    assert!(db
        .related_search_tags(&[], frames, None, None, None)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn related_tags_read_memory_tags_and_skip_malformed_ones() {
    let (_dir, db) = open_db().await;
    for tags in [
        r#"["project:atlas","person:ada"]"#,
        r#"["project:atlas","person:ada","person:bob"]"#,
        r#"not json"#,
        r#"{"project:atlas":1}"#,
    ] {
        sqlx::query(
            "INSERT INTO memories (content, source, tags, importance, created_at, updated_at) \
             VALUES ('m', 'user', ?1, 0.5, '2026-10-01T09:03:00.000Z', '2026-10-01T09:03:00.000Z')",
        )
        .bind(tags)
        .execute(db.write_pool())
        .await
        .unwrap();
    }
    let memories = RelatedTagSources {
        memories: true,
        ..Default::default()
    };
    let related = db
        .related_search_tags(&["project:atlas".to_string()], memories, None, None, None)
        .await
        .unwrap();
    assert_eq!(related["person"], vec!["person:ada", "person:bob"]);
    // Memories never fall inside windows.
    assert!(db
        .related_search_tags(
            &["project:atlas".to_string()],
            memories,
            None,
            None,
            Some(&SearchWindows::new([(at(0), at(60))])),
        )
        .await
        .unwrap()
        .is_empty());
}
