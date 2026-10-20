// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! The semantic projection store (W2-05): storing runs, items and run items,
//! frame attachment, FTS search, garbage collection and the actor identity
//! layer (heuristic assignment, alias uniqueness, merge, rename, reassign).
//!
//! Every test works on its own temporary database.

use std::time::Duration;

use chrono::{TimeZone, Utc};
use screenpipe_config::DbConfig;
use screenpipe_db::ext::semantic::{
    NewSemanticItem, NewSemanticRun, SemanticAppPlatform, SemanticAssignmentSource,
    SemanticFingerprint, SemanticFrameAttachment, SemanticFrameAttachmentBuffer,
    SemanticIdentityQuality, SemanticItemKind, SemanticItemQuery, SemanticRunStatus,
    SemanticStoreError, SEMANTIC_GC_DEFAULT_GRACE,
};
use screenpipe_db::DatabaseManager;
use sha2::{Digest, Sha256};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection};
use tempfile::TempDir;

async fn open_db() -> (TempDir, DatabaseManager) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("db.sqlite");
    let db = DatabaseManager::new(path.to_str().unwrap(), DbConfig::default())
        .await
        .expect("DatabaseManager::new");
    (dir, db)
}

fn fp(tag: &str) -> SemanticFingerprint {
    Sha256::digest(tag.as_bytes()).into()
}

/// A message item whose fingerprints derive from `tag`.
fn message(local: &str, tag: &str, body: &str, actor: Option<&str>) -> NewSemanticItem {
    let mut item = NewSemanticItem::new(
        local,
        SemanticItemKind::Message,
        format!("msg:{tag}"),
        SemanticIdentityQuality::Derived,
        fp(&format!("entity:{tag}")),
        fp(&format!("version:{tag}")),
    );
    item.body = Some(body.to_string());
    item.actor = actor.map(str::to_string);
    item
}

fn conversation(local: &str, tag: &str, title: &str) -> NewSemanticItem {
    let mut item = NewSemanticItem::new(
        local,
        SemanticItemKind::Conversation,
        format!("conv:{tag}"),
        SemanticIdentityQuality::Stable,
        fp(&format!("entity:{tag}")),
        fp(&format!("version:{tag}")),
    );
    item.title = Some(title.to_string());
    item
}

fn new_run(input: &str, items: Vec<NewSemanticItem>) -> NewSemanticRun {
    NewSemanticRun {
        input_fingerprint: fp(&format!("input:{input}")),
        parser_id: "chat.test".into(),
        parser_version: "1.0.0".into(),
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
        text_bytes: 42,
        parse_duration_us: 1500,
        actor_source_key: None,
        items,
    }
}

async fn insert_frame(db: &DatabaseManager, minute: u32) -> i64 {
    let ts = Utc.with_ymd_and_hms(2026, 10, 1, 9, minute, 0).unwrap();
    db.insert_snapshot_frame(
        "monitor_1",
        ts,
        &format!("/tmp/fp-test/snap-{minute}.jpg"),
        Some("Chat"),
        Some("General"),
        None,
        true,
        Some("test"),
        Some("frame text"),
        Some("accessibility"),
        None,
        None,
        None,
    )
    .await
    .expect("insert frame")
}

async fn count(db: &DatabaseManager, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
        .fetch_one(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

async fn fts_hits(db: &DatabaseManager, term: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM semantic_items_fts WHERE semantic_items_fts MATCH ?1")
        .bind(term)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

fn query(text: &str) -> SemanticItemQuery {
    SemanticItemQuery {
        text: text.to_string(),
        limit: 50,
        ..Default::default()
    }
}

#[tokio::test]
async fn store_writes_run_items_hierarchy_and_reuses_by_fingerprint() {
    let (_dir, db) = open_db().await;

    let mut first = message(
        "m1",
        "hello",
        "Hello team, standup at ten",
        Some("Ada Lovelace"),
    );
    first.parent_local_id = Some("c".into());
    first.source_node_ids = vec![3, 4, 70_000];
    first.occurred_at = Some(Utc.with_ymd_and_hms(2026, 10, 1, 8, 59, 30).unwrap());
    first.timestamp_precision = Some("minute".into());
    first
        .metadata
        .insert("channel".into(), serde_json::json!("general"));
    let mut second = message("m2", "reply", "Sounds good", Some("Grace Hopper"));
    second.parent_local_id = Some("c".into());
    let run = new_run(
        "frame-1",
        vec![conversation("c", "general", "#general"), first, second],
    );

    let stored = db.store_semantic_run(&run).await.unwrap();
    assert!(stored.created);
    assert_eq!(stored.status, SemanticRunStatus::Handled);
    assert_eq!(stored.item_ids.len(), 3);
    assert_eq!(stored.items_inserted, 3);
    assert_eq!(stored.actors_created, 2);
    assert_eq!(stored.assignments_created, 2);

    let detail = db
        .get_semantic_run_detail(stored.run_id)
        .await
        .unwrap()
        .expect("run detail");
    assert_eq!(detail.run.parser_id, "chat.test");
    assert_eq!(detail.run.app_platform, SemanticAppPlatform::Windows);
    assert_eq!(detail.run.item_count, 3);
    assert_eq!(detail.run.text_bytes, 42);
    assert_eq!(detail.run.input_content_hash, [7; 8]);
    let locals: Vec<&str> = detail.items.iter().map(|i| i.local_id.as_str()).collect();
    assert_eq!(locals, ["c", "m1", "m2"]);
    assert_eq!(
        detail
            .items
            .iter()
            .map(|i| i.sort_order)
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(detail.items[0].parent_local_id, None);
    assert_eq!(detail.items[1].parent_local_id.as_deref(), Some("c"));
    assert_eq!(detail.items[1].source_node_ids(), Some(vec![3, 4, 70_000]));
    assert_eq!(detail.items[0].source_node_ids(), Some(vec![]));
    let m1 = &detail.items[1].item;
    assert_eq!(m1.kind, SemanticItemKind::Message);
    assert_eq!(m1.identity_quality, SemanticIdentityQuality::Derived);
    assert_eq!(m1.occurred_at.as_deref(), Some("2026-10-01T08:59:30.000Z"));
    assert_eq!(m1.metadata["channel"], "general");
    assert_eq!(m1.actor.as_deref(), Some("Ada Lovelace"));
    let assigned = detail.items[1].assigned_actor.as_ref().unwrap();
    assert_eq!(assigned.name, "Ada Lovelace");
    assert_eq!(
        assigned.assignment_source,
        SemanticAssignmentSource::Heuristic
    );
    assert!(assigned.observed_alias_id.is_some());
    assert!(detail.items[0].assigned_actor.is_none());

    // The serialized form exposes fingerprints as hex and node ids as a list.
    let json = serde_json::to_value(&detail).unwrap();
    assert_eq!(json["run"]["input_fingerprint"].as_str().unwrap().len(), 64);
    assert_eq!(
        json["items"][1]["source_node_ids"],
        serde_json::json!([3, 4, 70000])
    );

    // The same input again reuses the run and writes nothing.
    let again = db.store_semantic_run(&run).await.unwrap();
    assert!(!again.created);
    assert_eq!(again.run_id, stored.run_id);
    assert_eq!(again.item_ids, stored.item_ids);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM semantic_runs").await, 1);
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM semantic_run_items").await,
        3
    );
    let cached = db
        .find_semantic_run_by_fingerprint(&run.input_fingerprint)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cached.id, stored.run_id);
    assert!(db
        .find_semantic_run_by_fingerprint(&fp("never stored"))
        .await
        .unwrap()
        .is_none());

    // A different input that shows one of the same messages shares its item.
    let later = new_run(
        "frame-2",
        vec![
            message("x1", "reply", "Sounds good", Some("Grace Hopper")),
            message("x2", "new", "Shipping the build", Some("Grace Hopper")),
        ],
    );
    let stored_later = db.store_semantic_run(&later).await.unwrap();
    assert!(stored_later.created);
    assert_eq!(stored_later.items_inserted, 1);
    assert_eq!(stored_later.item_ids[0], stored.item_ids[2]);
    assert_eq!(stored_later.actors_created, 0, "existing alias is reused");
    assert_eq!(stored_later.assignments_created, 1, "only the new item");
    assert_eq!(count(&db, "SELECT COUNT(*) FROM semantic_items").await, 4);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM semantic_actors").await, 2);
}

#[tokio::test]
async fn empty_runs_store_without_items() {
    let (_dir, db) = open_db().await;
    let stored = db
        .store_semantic_run(&new_run("blank", vec![]))
        .await
        .unwrap();
    assert_eq!(stored.status, SemanticRunStatus::Empty);
    assert!(stored.item_ids.is_empty());
    let detail = db
        .get_semantic_run_detail(stored.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.run.status, SemanticRunStatus::Empty);
    assert_eq!(detail.run.item_count, 0);
    assert!(detail.items.is_empty());
}

#[tokio::test]
async fn conflicting_writes_fail_and_roll_back() {
    let (_dir, db) = open_db().await;
    let run = new_run("frame-1", vec![message("m", "a", "alpha", None)]);
    db.store_semantic_run(&run).await.unwrap();

    // Same input fingerprint from another parser version.
    let mut other = run.clone();
    other.parser_version = "2.0.0".into();
    let err = db.store_semantic_run(&other).await.unwrap_err();
    assert!(matches!(err, SemanticStoreError::Conflict(_)), "{err}");

    // Same version fingerprint with a different kind: the whole run is rolled
    // back, including the run row and the items before the bad one.
    let mut clash = message("bad", "a", "alpha", None);
    clash.kind = SemanticItemKind::Task;
    let bad_run = new_run(
        "frame-2",
        vec![message("ok", "fresh", "fresh item", Some("Lin")), clash],
    );
    let err = db.store_semantic_run(&bad_run).await.unwrap_err();
    assert!(matches!(err, SemanticStoreError::Conflict(_)), "{err}");
    assert_eq!(count(&db, "SELECT COUNT(*) FROM semantic_runs").await, 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM semantic_items").await, 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM semantic_actors").await, 0);

    // Malformed input never reaches SQLite.
    let mut orphan = message("child", "c", "child", None);
    orphan.parent_local_id = Some("missing".into());
    let err = db
        .store_semantic_run(&new_run("frame-3", vec![orphan]))
        .await
        .unwrap_err();
    assert!(err.is_invalid_input(), "{err}");
    assert_eq!(count(&db, "SELECT COUNT(*) FROM semantic_runs").await, 1);

    // The store is still usable afterwards.
    db.store_semantic_run(&new_run("frame-4", vec![message("m", "b", "beta", None)]))
        .await
        .unwrap();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM semantic_runs").await, 2);
}

#[tokio::test]
async fn frames_attach_in_batches() {
    let (_dir, db) = open_db().await;
    let run_a = db
        .store_semantic_run(&new_run("a", vec![message("m", "a", "alpha", None)]))
        .await
        .unwrap();
    let run_b = db
        .store_semantic_run(&new_run("b", vec![message("m", "b", "beta", None)]))
        .await
        .unwrap();
    let f1 = insert_frame(&db, 1).await;
    let f2 = insert_frame(&db, 2).await;
    let f3 = insert_frame(&db, 3).await;

    let outcome = db
        .attach_semantic_runs_to_frames(&[
            SemanticFrameAttachment {
                frame_id: f1,
                run_id: run_a.run_id,
            },
            SemanticFrameAttachment {
                frame_id: f2,
                run_id: run_a.run_id,
            },
            // A later entry for the same frame wins.
            SemanticFrameAttachment {
                frame_id: f2,
                run_id: run_b.run_id,
            },
            // A run that does not exist is skipped instead of failing the
            // batch, and reported back.
            SemanticFrameAttachment {
                frame_id: f3,
                run_id: 9_999,
            },
            // A frame that does not exist is ignored.
            SemanticFrameAttachment {
                frame_id: 123_456,
                run_id: run_a.run_id,
            },
        ])
        .await
        .unwrap();
    assert_eq!(outcome.frames_updated, 2);
    // run_a still went onto f1, so the missing frame costs it nothing.
    assert_eq!((outcome.runs_discarded, outcome.items_discarded), (0, 0));
    assert_eq!(
        outcome.skipped,
        vec![SemanticFrameAttachment {
            frame_id: f3,
            run_id: 9_999,
        }]
    );

    let on_f1 = db.get_semantic_run_for_frame(f1).await.unwrap().unwrap();
    assert_eq!(on_f1.run.id, run_a.run_id);
    let on_f2 = db.get_semantic_run_for_frame(f2).await.unwrap().unwrap();
    assert_eq!(on_f2.run.id, run_b.run_id);
    assert!(db.get_semantic_run_for_frame(f3).await.unwrap().is_none());
    assert!(db
        .get_semantic_run_for_frame(123_456)
        .await
        .unwrap()
        .is_none());

    // The buffer coalesces and flushes through the same path.
    let mut buffer = SemanticFrameAttachmentBuffer::new(2, Duration::from_secs(3600));
    assert!(!buffer.push(f3, run_a.run_id).unwrap());
    assert!(buffer.push(f1, run_b.run_id).unwrap());
    assert_eq!(buffer.flush(&db).await.unwrap().frames_updated, 2);
    assert!(buffer.is_empty());
    assert_eq!(
        db.get_semantic_run_for_frame(f3)
            .await
            .unwrap()
            .unwrap()
            .run
            .id,
        run_a.run_id
    );
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM frames WHERE semantic_run_id IS NOT NULL"
        )
        .await,
        3
    );

    let err = db
        .attach_semantic_runs_to_frames(&[SemanticFrameAttachment {
            frame_id: 0,
            run_id: 1,
        }])
        .await
        .unwrap_err();
    assert!(err.is_invalid_input());

    // Invalid ids never enter the buffer, so they cannot cost the valid
    // entries of the same batch at flush time.
    assert!(buffer
        .push(-5, run_a.run_id)
        .unwrap_err()
        .is_invalid_input());
    assert!(buffer.is_empty());
}

#[tokio::test]
async fn buffer_flush_attaches_valid_entries_mixed_with_invalid_pushes() {
    let (_dir, db) = open_db().await;
    let run_a = db
        .store_semantic_run(&new_run("a", vec![message("m", "a", "alpha", None)]))
        .await
        .unwrap();
    let run_b = db
        .store_semantic_run(&new_run("b", vec![message("m", "b", "beta", None)]))
        .await
        .unwrap();
    let f1 = insert_frame(&db, 1).await;
    let f2 = insert_frame(&db, 2).await;
    let f3 = insert_frame(&db, 3).await;

    let mut buffer = SemanticFrameAttachmentBuffer::new(100, Duration::from_secs(3600));
    buffer.push(f1, run_a.run_id).unwrap();
    assert!(buffer.push(0, run_a.run_id).unwrap_err().is_invalid_input());
    buffer.push(f2, run_b.run_id).unwrap();
    assert!(buffer.push(f3, -1).unwrap_err().is_invalid_input());
    assert!(buffer
        .push(-7, run_b.run_id)
        .unwrap_err()
        .is_invalid_input());
    // A rejected push for a frame already queued keeps the queued entry.
    assert!(buffer.push(f1, 0).unwrap_err().is_invalid_input());
    buffer.push(f3, run_b.run_id).unwrap();
    assert_eq!(buffer.len(), 3);

    assert_eq!(buffer.flush(&db).await.unwrap().frames_updated, 3);
    assert!(buffer.is_empty());
    for (frame, run) in [(f1, run_a.run_id), (f2, run_b.run_id), (f3, run_b.run_id)] {
        let on_frame = db.get_semantic_run_for_frame(frame).await.unwrap().unwrap();
        assert_eq!(on_frame.run.id, run, "frame {frame}");
    }
}

#[tokio::test]
async fn buffer_flush_requeues_the_batch_when_the_database_fails() {
    let (_dir, db) = open_db().await;
    let stored = db
        .store_semantic_run(&new_run("a", vec![message("m", "a", "alpha", None)]))
        .await
        .unwrap();
    let f1 = insert_frame(&db, 1).await;
    let f2 = insert_frame(&db, 2).await;

    let mut buffer = SemanticFrameAttachmentBuffer::new(100, Duration::from_secs(3600));
    buffer.push(f1, stored.run_id).unwrap();
    buffer.push(f2, stored.run_id).unwrap();

    // Make the UPDATE fail inside SQLite, then lift the failure again.
    sqlx::query(
        "CREATE TRIGGER fp_test_block_attach BEFORE UPDATE OF semantic_run_id ON frames \
         BEGIN SELECT RAISE(ABORT, 'attach blocked'); END",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let err = buffer.flush(&db).await.unwrap_err();
    assert!(matches!(err, SemanticStoreError::Database(_)), "{err}");
    assert_eq!(buffer.len(), 2, "the whole batch is queued again");
    // A push made after the failure wins over the requeued entry.
    let other = db
        .store_semantic_run(&new_run("b", vec![message("m", "b", "beta", None)]))
        .await
        .unwrap();
    buffer.push(f2, other.run_id).unwrap();

    sqlx::query("DROP TRIGGER fp_test_block_attach")
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(buffer.flush(&db).await.unwrap().frames_updated, 2);
    assert!(buffer.is_empty());
    assert_eq!(
        db.get_semantic_run_for_frame(f1)
            .await
            .unwrap()
            .unwrap()
            .run
            .id,
        stored.run_id
    );
    assert_eq!(
        db.get_semantic_run_for_frame(f2)
            .await
            .unwrap()
            .unwrap()
            .run
            .id,
        other.run_id
    );
}

#[tokio::test]
async fn fts_matches_items_and_frames() {
    let (_dir, db) = open_db().await;
    let mut task = NewSemanticItem::new(
        "t",
        SemanticItemKind::Task,
        "task:invoice",
        SemanticIdentityQuality::Stable,
        fp("entity:task"),
        fp("version:task"),
    );
    task.title = Some("Send quarterly invoice".into());
    task.status = Some("open".into());
    task.metadata
        .insert("project".into(), serde_json::json!("zephyr"));
    let run = new_run(
        "frame-1",
        vec![
            conversation("c", "ops", "Operations room"),
            message(
                "m1",
                "deploy",
                "The deploy of build 100.100.0.42 finished",
                Some("Ada"),
            ),
            message("m2", "lunch", "Lunch at noon?", Some("Grace")),
            task,
        ],
    );
    let stored = db.store_semantic_run(&run).await.unwrap();
    let empty = db
        .store_semantic_run(&new_run("blank", vec![]))
        .await
        .unwrap();
    let f1 = insert_frame(&db, 10).await;
    let f2 = insert_frame(&db, 20).await;
    let f_empty = insert_frame(&db, 30).await;
    db.attach_semantic_runs_to_frames(&[
        SemanticFrameAttachment {
            frame_id: f1,
            run_id: stored.run_id,
        },
        SemanticFrameAttachment {
            frame_id: f2,
            run_id: stored.run_id,
        },
        SemanticFrameAttachment {
            frame_id: f_empty,
            run_id: empty.run_id,
        },
    ])
    .await
    .unwrap();

    // Body, title, actor, status and metadata are all indexed.
    let hits = db
        .search_semantic_items(&query("deploy finished"))
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].item.item_key, "msg:deploy");
    assert_eq!(hits[0].assigned_actor.as_ref().unwrap().name, "Ada");
    assert_eq!(
        db.search_semantic_items(&query("operations"))
            .await
            .unwrap()[0]
            .item
            .kind,
        SemanticItemKind::Conversation
    );
    assert_eq!(
        db.search_semantic_items(&query("grace"))
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        db.search_semantic_items(&query("zephyr"))
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        db.search_semantic_items(&query("open invoice"))
            .await
            .unwrap()
            .len(),
        1
    );
    // Punctuation is literal, not FTS syntax.
    assert_eq!(
        db.search_semantic_items(&query("100.100.0.42"))
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(db
        .search_semantic_items(&query("nonexistentword"))
        .await
        .unwrap()
        .is_empty());
    assert!(db
        .search_semantic_items(&query("   "))
        .await
        .unwrap()
        .is_empty());

    // Kind and actor filters.
    let mut q = query("invoice");
    q.kind = Some(SemanticItemKind::Message);
    assert!(db.search_semantic_items(&q).await.unwrap().is_empty());
    q.kind = Some(SemanticItemKind::Task);
    assert_eq!(db.search_semantic_items(&q).await.unwrap().len(), 1);
    let grace_id = db.search_semantic_actors("Grace", 10, 0).await.unwrap()[0].id;
    let mut q = query("lunch");
    q.actor_id = Some(grace_id);
    assert_eq!(db.search_semantic_items(&q).await.unwrap().len(), 1);
    let mut q = query("deploy");
    q.actor_id = Some(grace_id);
    assert!(db.search_semantic_items(&q).await.unwrap().is_empty());

    // Frame search walks frames -> handled run -> run items -> FTS, newest first.
    let frames = db.search_semantic_frames(&query("lunch")).await.unwrap();
    assert_eq!(
        frames.iter().map(|f| f.frame_id).collect::<Vec<_>>(),
        [f2, f1]
    );
    assert_eq!(frames[0].run_id, stored.run_id);
    assert_eq!(frames[0].item_ids, vec![stored.item_ids[2]]);
    let mut q = query("lunch");
    q.limit = 1;
    q.offset = 1;
    let page = db.search_semantic_frames(&q).await.unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].frame_id, f1);
    let mut q = query("lunch");
    q.actor_id = Some(db.search_semantic_actors("Ada", 10, 0).await.unwrap()[0].id);
    assert!(db.search_semantic_frames(&q).await.unwrap().is_empty());
}

#[tokio::test]
async fn gc_removes_unreferenced_runs_and_items_but_keeps_actors() {
    let (_dir, db) = open_db().await;
    let run1 = db
        .store_semantic_run(&new_run(
            "one",
            vec![
                message("a", "shared", "shared hello", Some("Ada")),
                message("b", "only-one", "ephemeral pineapple", Some("Grace")),
            ],
        ))
        .await
        .unwrap();
    let run2 = db
        .store_semantic_run(&new_run(
            "two",
            vec![message("a", "shared", "shared hello", Some("Ada"))],
        ))
        .await
        .unwrap();
    let f1 = insert_frame(&db, 1).await;
    let f2 = insert_frame(&db, 2).await;
    db.attach_semantic_runs_to_frames(&[
        SemanticFrameAttachment {
            frame_id: f1,
            run_id: run1.run_id,
        },
        SemanticFrameAttachment {
            frame_id: f2,
            run_id: run2.run_id,
        },
    ])
    .await
    .unwrap();

    // Everything is referenced: nothing to collect.
    let stats = db.gc_semantic_projection(Duration::ZERO).await.unwrap();
    assert_eq!((stats.runs_deleted, stats.items_deleted), (0, 0));

    // Frame 1 ages out: run 1 and its private item go, the shared item stays.
    sqlx::query("DELETE FROM frames WHERE id = ?1")
        .bind(f1)
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(fts_hits(&db, "pineapple").await, 1);
    let stats = db.gc_semantic_projection(Duration::ZERO).await.unwrap();
    assert_eq!((stats.runs_deleted, stats.items_deleted), (1, 1));
    assert!(db
        .get_semantic_run_detail(run1.run_id)
        .await
        .unwrap()
        .is_none());
    assert!(db
        .get_semantic_run_detail(run2.run_id)
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        fts_hits(&db, "pineapple").await,
        0,
        "fts row follows the item"
    );
    assert_eq!(fts_hits(&db, "shared").await, 1);
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM semantic_run_items").await,
        1
    );
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM semantic_item_actors").await,
        1
    );

    // Frame 2 goes too: the projection is empty, the actor identities remain.
    sqlx::query("DELETE FROM frames WHERE id = ?1")
        .bind(f2)
        .execute(&db.pool)
        .await
        .unwrap();
    let stats = db.gc_semantic_projection(Duration::ZERO).await.unwrap();
    assert_eq!((stats.runs_deleted, stats.items_deleted), (1, 1));
    for table in [
        "semantic_runs",
        "semantic_items",
        "semantic_run_items",
        "semantic_item_actors",
    ] {
        assert_eq!(
            count(&db, &format!("SELECT COUNT(*) FROM {table}")).await,
            0,
            "{table}"
        );
    }
    assert_eq!(fts_hits(&db, "shared").await, 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM semantic_actors").await, 2);
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM semantic_actor_aliases").await,
        2
    );

    // A later capture of Ada resolves to the surviving identity.
    let again = db
        .store_semantic_run(&new_run(
            "three",
            vec![message("a", "new", "back again", Some("ada"))],
        ))
        .await
        .unwrap();
    assert_eq!(again.actors_created, 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM semantic_actors").await, 2);
}

#[tokio::test]
async fn gc_grace_keeps_runs_awaiting_attachment() {
    let (_dir, db) = open_db().await;
    let run = db
        .store_semantic_run(&new_run(
            "pending",
            vec![message("m", "p", "pending item", None)],
        ))
        .await
        .unwrap();

    let stats = db
        .gc_semantic_projection(SEMANTIC_GC_DEFAULT_GRACE)
        .await
        .unwrap();
    assert_eq!((stats.runs_deleted, stats.items_deleted), (0, 0));
    assert!(db
        .get_semantic_run_detail(run.run_id)
        .await
        .unwrap()
        .is_some());

    let stats = db.gc_semantic_projection(Duration::ZERO).await.unwrap();
    assert_eq!((stats.runs_deleted, stats.items_deleted), (1, 1));

    // Attaching to a run GC already removed is reported back, not written
    // and not an FK failure.
    let frame = insert_frame(&db, 5).await;
    let outcome = db
        .attach_semantic_runs_to_frames(&[SemanticFrameAttachment {
            frame_id: frame,
            run_id: run.run_id,
        }])
        .await
        .unwrap();
    assert_eq!(outcome.frames_updated, 0);
    assert_eq!(
        outcome.skipped,
        vec![SemanticFrameAttachment {
            frame_id: frame,
            run_id: run.run_id,
        }]
    );
}

#[tokio::test]
async fn gc_of_a_reused_run_reports_the_frames_waiting_on_it() {
    let (_dir, db) = open_db().await;
    let parse = new_run("reused", vec![message("m", "r", "reusedbody", None)]);
    let first = db.store_semantic_run(&parse).await.unwrap();
    let old_frame = insert_frame(&db, 1).await;
    db.attach_semantic_runs_to_frames(&[SemanticFrameAttachment {
        frame_id: old_frame,
        run_id: first.run_id,
    }])
    .await
    .unwrap();

    // The run is older than the grace, and retention removes its only frame.
    sqlx::query("UPDATE semantic_runs SET created_at = '2026-01-01T00:00:00.000Z' WHERE id = ?1")
        .bind(first.run_id)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM frames WHERE id = ?1")
        .bind(old_frame)
        .execute(&db.pool)
        .await
        .unwrap();

    // The worker sees the same input on a new frame and reuses the run.
    let reused = db.store_semantic_run(&parse).await.unwrap();
    assert!(!reused.created);
    assert_eq!(reused.run_id, first.run_id);
    let new_frame = insert_frame(&db, 2).await;
    let mut buffer = SemanticFrameAttachmentBuffer::new(100, Duration::from_secs(3600));
    buffer.push(new_frame, reused.run_id).unwrap();

    // A periodic retention pass runs before the flush and collects the run.
    let stats = db
        .gc_semantic_projection(SEMANTIC_GC_DEFAULT_GRACE)
        .await
        .unwrap();
    assert_eq!((stats.runs_deleted, stats.items_deleted), (1, 1));

    // The flush says which frame lost its run instead of dropping it quietly.
    let outcome = buffer.flush(&db).await.unwrap();
    assert_eq!(outcome.frames_updated, 0);
    assert_eq!(
        outcome.skipped,
        vec![SemanticFrameAttachment {
            frame_id: new_frame,
            run_id: reused.run_id,
        }]
    );
    assert!(buffer.is_empty());

    // Storing the parse again gives the frame a fresh run.
    let again = db.store_semantic_run(&parse).await.unwrap();
    assert!(again.created);
    assert_ne!(again.run_id, first.run_id);
    buffer.push(new_frame, again.run_id).unwrap();
    let outcome = buffer.flush(&db).await.unwrap();
    assert_eq!(outcome.frames_updated, 1);
    assert!(outcome.skipped.is_empty());
    let on_frame = db
        .get_semantic_run_for_frame(new_frame)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(on_frame.run.id, again.run_id);
    assert_eq!(fts_hits(&db, "reusedbody").await, 1);
}

async fn delete_frames(db: &DatabaseManager, ids: &[i64]) {
    for id in ids {
        sqlx::query("DELETE FROM frames WHERE id = ?1")
            .bind(id)
            .execute(&db.pool)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn a_frame_deleted_with_its_run_is_not_reported() {
    let (_dir, db) = open_db().await;
    let frame = insert_frame(&db, 1).await;
    let stored = db
        .store_semantic_run(&new_run(
            "secret",
            vec![message("m", "secret", "deletedsecret", Some("Ada"))],
        ))
        .await
        .unwrap();
    let mut buffer = SemanticFrameAttachmentBuffer::new(100, Duration::from_secs(3600));
    buffer.push(frame, stored.run_id).unwrap();

    // The user deletes the range holding the frame before the flush, and the
    // delete runs a zero-grace pass.
    delete_frames(&db, &[frame]).await;
    let stats = db.gc_semantic_projection(Duration::ZERO).await.unwrap();
    assert_eq!((stats.runs_deleted, stats.items_deleted), (1, 1));
    assert_eq!(fts_hits(&db, "deletedsecret").await, 0);

    // Nothing comes back for the worker to store again.
    let outcome = buffer.flush(&db).await.unwrap();
    assert_eq!(outcome.frames_updated, 0);
    assert!(outcome.skipped.is_empty(), "{:?}", outcome.skipped);
    assert_eq!((outcome.runs_discarded, outcome.items_discarded), (0, 0));
    assert!(buffer.is_empty());
    assert_eq!(fts_hits(&db, "deletedsecret").await, 0);
    assert!(db
        .search_semantic_items(&query("deletedsecret"))
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn surviving_frames_whose_run_was_collected_are_reported() {
    let (_dir, db) = open_db().await;
    let gone = insert_frame(&db, 1).await;
    let kept = insert_frame(&db, 2).await;
    let gone_too = insert_frame(&db, 3).await;
    let kept_too = insert_frame(&db, 4).await;
    let secret = new_run(
        "secret",
        vec![message("m", "secret", "deletedsecret", None)],
    );
    let kept_parse = new_run("kept", vec![message("m", "kept", "keptbody", None)]);
    // One input seen on two frames, one inside the deleted range and one
    // outside it.
    let mixed_parse = new_run("mixed", vec![message("m", "mixed", "mixedbody", None)]);
    let secret_run = db.store_semantic_run(&secret).await.unwrap();
    let kept_run = db.store_semantic_run(&kept_parse).await.unwrap();
    let mixed_run = db.store_semantic_run(&mixed_parse).await.unwrap();
    let mut buffer = SemanticFrameAttachmentBuffer::new(100, Duration::from_secs(3600));
    buffer.push(gone, secret_run.run_id).unwrap();
    buffer.push(kept, kept_run.run_id).unwrap();
    buffer.push(gone_too, mixed_run.run_id).unwrap();
    buffer.push(kept_too, mixed_run.run_id).unwrap();

    // A delete-range takes `gone` and `gone_too`; its zero-grace pass also
    // collects the not yet attached runs of the frames outside the range.
    delete_frames(&db, &[gone, gone_too]).await;
    let stats = db.gc_semantic_projection(Duration::ZERO).await.unwrap();
    assert_eq!((stats.runs_deleted, stats.items_deleted), (3, 3));

    // Only the frames that still exist are reported, also when a deleted
    // frame was waiting on the same run.
    let outcome = buffer.flush(&db).await.unwrap();
    assert_eq!(outcome.frames_updated, 0);
    assert_eq!(
        outcome.skipped,
        vec![
            SemanticFrameAttachment {
                frame_id: kept,
                run_id: kept_run.run_id,
            },
            SemanticFrameAttachment {
                frame_id: kept_too,
                run_id: mixed_run.run_id,
            },
        ]
    );
    assert_eq!((outcome.runs_discarded, outcome.items_discarded), (0, 0));

    // The worker follows the contract: store each reported frame's parse
    // again and attach the new run.
    for (pair, parse) in outcome.skipped.iter().zip([&kept_parse, &mixed_parse]) {
        let again = db.store_semantic_run(parse).await.unwrap();
        assert!(again.created);
        buffer.push(pair.frame_id, again.run_id).unwrap();
    }
    let outcome = buffer.flush(&db).await.unwrap();
    assert_eq!(outcome.frames_updated, 2);
    assert!(outcome.skipped.is_empty());
    assert_eq!(fts_hits(&db, "keptbody").await, 1);
    assert_eq!(fts_hits(&db, "mixedbody").await, 1);
    assert_eq!(fts_hits(&db, "deletedsecret").await, 0);
    assert!(db
        .search_semantic_items(&query("deletedsecret"))
        .await
        .unwrap()
        .is_empty());
    for frame in [gone, gone_too] {
        assert!(db
            .get_semantic_run_for_frame(frame)
            .await
            .unwrap()
            .is_none());
    }
}

#[tokio::test]
async fn a_parse_stored_after_its_frame_was_deleted_is_discarded_by_the_flush() {
    let (_dir, db) = open_db().await;
    // A frame outside the deleted range holds a run with an item the late
    // parse shares.
    let holder = insert_frame(&db, 1).await;
    let holder_run = db
        .store_semantic_run(&new_run(
            "holder",
            vec![message("m", "shared", "sharedhello", None)],
        ))
        .await
        .unwrap();
    db.attach_semantic_runs_to_frames(&[SemanticFrameAttachment {
        frame_id: holder,
        run_id: holder_run.run_id,
    }])
    .await
    .unwrap();

    // The worker read `late` before the user deleted it, and stores its
    // parse only after the delete and its zero-grace pass have finished.
    let late = insert_frame(&db, 2).await;
    delete_frames(&db, &[late]).await;
    let stats = db.gc_semantic_projection(Duration::ZERO).await.unwrap();
    assert_eq!((stats.runs_deleted, stats.items_deleted), (0, 0));
    let late_run = db
        .store_semantic_run(&new_run(
            "late",
            vec![
                message("a", "late", "latepineapple", Some("Grace")),
                message("b", "shared", "sharedhello", None),
            ],
        ))
        .await
        .unwrap();
    assert!(late_run.created);
    assert_eq!(late_run.items_inserted, 1);
    assert_eq!(fts_hits(&db, "latepineapple").await, 1);

    // The flush finds no frame for the run and drops it with its own item.
    let mut buffer = SemanticFrameAttachmentBuffer::new(100, Duration::from_secs(3600));
    buffer.push(late, late_run.run_id).unwrap();
    let outcome = buffer.flush(&db).await.unwrap();
    assert_eq!(outcome.frames_updated, 0);
    assert!(outcome.skipped.is_empty());
    assert_eq!((outcome.runs_discarded, outcome.items_discarded), (1, 1));
    assert!(db
        .get_semantic_run_detail(late_run.run_id)
        .await
        .unwrap()
        .is_none());
    assert_eq!(fts_hits(&db, "latepineapple").await, 0);
    assert!(db
        .search_semantic_items(&query("latepineapple"))
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM semantic_item_actors").await,
        0
    );
    // The shared item still belongs to the holder's run, and actors stay, as
    // they do in GC.
    assert_eq!(fts_hits(&db, "sharedhello").await, 1);
    let detail = db
        .get_semantic_run_for_frame(holder)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.run.id, holder_run.run_id);
    assert_eq!(detail.items.len(), 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM semantic_actors").await, 1);

    // A run another frame points at is kept, even when every frame the batch
    // names for it is gone.
    let outcome = db
        .attach_semantic_runs_to_frames(&[SemanticFrameAttachment {
            frame_id: late,
            run_id: holder_run.run_id,
        }])
        .await
        .unwrap();
    assert_eq!(outcome.frames_updated, 0);
    assert!(outcome.skipped.is_empty());
    assert_eq!((outcome.runs_discarded, outcome.items_discarded), (0, 0));
    assert!(db
        .get_semantic_run_detail(holder_run.run_id)
        .await
        .unwrap()
        .is_some());

    // So is a fresh run the batch also attaches to a frame that exists.
    let fresh = insert_frame(&db, 3).await;
    let both = db
        .store_semantic_run(&new_run(
            "both",
            vec![message("m", "both", "bothbody", None)],
        ))
        .await
        .unwrap();
    buffer.push(late, both.run_id).unwrap();
    buffer.push(fresh, both.run_id).unwrap();
    let outcome = buffer.flush(&db).await.unwrap();
    assert_eq!(outcome.frames_updated, 1);
    assert!(outcome.skipped.is_empty());
    assert_eq!((outcome.runs_discarded, outcome.items_discarded), (0, 0));
    assert_eq!(
        db.get_semantic_run_for_frame(fresh)
            .await
            .unwrap()
            .unwrap()
            .run
            .id,
        both.run_id
    );
    assert_eq!(fts_hits(&db, "bothbody").await, 1);

    // A later frame with the late input finds no run and is reported, so the
    // worker stores the parse again for it.
    let next = insert_frame(&db, 4).await;
    let outcome = db
        .attach_semantic_runs_to_frames(&[SemanticFrameAttachment {
            frame_id: next,
            run_id: late_run.run_id,
        }])
        .await
        .unwrap();
    assert_eq!(
        outcome.skipped,
        vec![SemanticFrameAttachment {
            frame_id: next,
            run_id: late_run.run_id,
        }]
    );
}

#[tokio::test]
async fn dropping_a_flush_keeps_the_batch_queued() {
    let (_dir, db) = open_db().await;
    let stored = db
        .store_semantic_run(&new_run("a", vec![message("m", "a", "alpha", None)]))
        .await
        .unwrap();
    let f1 = insert_frame(&db, 1).await;
    let f2 = insert_frame(&db, 2).await;
    let mut buffer = SemanticFrameAttachmentBuffer::new(100, Duration::from_secs(3600));
    buffer.push(f1, stored.run_id).unwrap();
    buffer.push(f2, stored.run_id).unwrap();

    // Another writer holds the write lock, so the flush waits for it. Drop
    // the flush there, as a timeout or the losing branch of a select! would.
    let holder = db.begin_immediate_with_retry().await.unwrap();
    let res = tokio::time::timeout(Duration::from_millis(100), buffer.flush(&db)).await;
    assert!(
        res.is_err(),
        "the flush should still be waiting for the lock"
    );
    assert_eq!(buffer.len(), 2, "a dropped flush loses nothing");
    holder.rollback().await.unwrap();

    let outcome = buffer.flush(&db).await.unwrap();
    assert_eq!(outcome.frames_updated, 2);
    assert!(outcome.skipped.is_empty());
    assert!(buffer.is_empty());
    for frame in [f1, f2] {
        let on_frame = db.get_semantic_run_for_frame(frame).await.unwrap().unwrap();
        assert_eq!(on_frame.run.id, stored.run_id, "frame {frame}");
    }
}

#[tokio::test]
async fn gc_handles_more_than_one_batch() {
    let (dir, db) = open_db().await;
    // 2,100 runs (more than one GC batch), all unreferenced, with one item each.
    let mut tx_conn = SqliteConnectOptions::new()
        .filename(dir.path().join("db.sqlite"))
        .connect()
        .await
        .unwrap();
    let mut tx = tx_conn.begin().await.unwrap();
    for i in 0..2100i64 {
        let input: Vec<u8> = fp(&format!("bulk-input-{i}")).to_vec();
        let version: Vec<u8> = fp(&format!("bulk-version-{i}")).to_vec();
        let run_id = sqlx::query(
            "INSERT INTO semantic_runs (input_fingerprint, parser_id, parser_version, \
             schema_version, app_platform, input_content_hash, status, item_count) \
             VALUES (?1, 'bulk', '1', 1, 'linux', X'0000000000000000', 'handled', 1)",
        )
        .bind(input)
        .execute(&mut *tx)
        .await
        .unwrap()
        .last_insert_rowid();
        let item_id = sqlx::query(
            "INSERT INTO semantic_items (entity_fingerprint, version_fingerprint, kind, \
             item_key, identity_quality, body) VALUES (?1, ?1, 'page', 'k', 'ephemeral', 'bulk')",
        )
        .bind(version)
        .execute(&mut *tx)
        .await
        .unwrap()
        .last_insert_rowid();
        sqlx::query(
            "INSERT INTO semantic_run_items (run_id, local_id, item_id, sort_order) \
             VALUES (?1, 'p', ?2, 0)",
        )
        .bind(run_id)
        .bind(item_id)
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    tx_conn.close().await.unwrap();

    let stats = db.gc_semantic_projection(Duration::ZERO).await.unwrap();
    assert_eq!((stats.runs_deleted, stats.items_deleted), (2100, 2100));
    assert_eq!(count(&db, "SELECT COUNT(*) FROM semantic_items").await, 0);
}

#[tokio::test]
async fn alias_lookup_is_unique_per_source_and_normalized_label() {
    let (_dir, db) = open_db().await;
    let run = new_run(
        "frame-1",
        vec![
            message("m1", "1", "first", Some("Ada Lovelace")),
            message("m2", "2", "second", Some("  ada   LOVELACE ")),
            message("m3", "3", "third", Some("ADA LOVELACE\u{200B}")),
            message("m4", "4", "fourth", Some("   ")),
        ],
    );
    let stored = db.store_semantic_run(&run).await.unwrap();
    assert_eq!(stored.actors_created, 1);
    assert_eq!(
        stored.assignments_created, 3,
        "the blank label is not assigned"
    );
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM semantic_actor_aliases").await,
        1
    );

    let alias = db
        .find_semantic_actor_alias("chat.test", "ada lovelace")
        .await
        .unwrap()
        .expect("alias");
    assert_eq!(alias.source_key, "chat.test");
    assert_eq!(alias.source_label, "Ada Lovelace");
    assert_eq!(alias.observed_label, "Ada Lovelace");
    assert_eq!(alias.normalized_label, "ada lovelace");
    assert!(db
        .find_semantic_actor_alias("other.source", "Ada Lovelace")
        .await
        .unwrap()
        .is_none());

    // The same label from another source is a separate identity until merged.
    let mut elsewhere = new_run(
        "frame-2",
        vec![message("m", "5", "fifth", Some("Ada Lovelace"))],
    );
    elsewhere.actor_source_key = Some("mail:work".into());
    let stored_elsewhere = db.store_semantic_run(&elsewhere).await.unwrap();
    assert_eq!(stored_elsewhere.actors_created, 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM semantic_actors").await, 2);

    // A label with a leading NUL or only control characters must not reach
    // the alias table's length checks and roll the run back.
    let controls = new_run(
        "frame-3",
        vec![
            message("m1", "6", "sixth", Some("\0Ada Lovelace")),
            message("m2", "7", "seventh", Some("\u{1}\u{2}")),
            message("m3", "8", "eighth", Some("\0")),
            message("m4", "9", "ninth", Some("\u{202A}\u{202C}")),
            message("m5", "10", "tenth", Some("\u{7}Grace\u{0} Hopper")),
        ],
    );
    let stored_controls = db.store_semantic_run(&controls).await.unwrap();
    assert!(stored_controls.created);
    assert_eq!(stored_controls.items_inserted, 5);
    assert_eq!(
        stored_controls.actors_created, 1,
        "only Grace Hopper is new"
    );
    assert_eq!(stored_controls.assignments_created, 2);
    let ada = db
        .find_semantic_actor_alias("chat.test", "\0ada lovelace")
        .await
        .unwrap()
        .expect("NUL-prefixed label resolves to the existing alias");
    assert_eq!(ada.id, alias.id);
    let grace = db
        .find_semantic_actor_alias("chat.test", "Grace Hopper")
        .await
        .unwrap()
        .expect("alias");
    assert_eq!(grace.source_label, "Grace Hopper");
    assert_eq!(grace.observed_label, "Grace Hopper");
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM semantic_actor_aliases \
             WHERE instr(source_label, char(0)) > 0 \
                OR instr(observed_label, char(0)) > 0 \
                OR instr(normalized_label, char(0)) > 0"
        )
        .await,
        0
    );
    assert!(db
        .find_semantic_actor_alias("chat.test", "\u{1}\u{2}")
        .await
        .unwrap()
        .is_none());

    // The schema itself refuses a second alias for the same key.
    let dup = sqlx::query(
        "INSERT INTO semantic_actor_aliases \
         (actor_id, source_key, source_label, normalized_label, observed_label) \
         VALUES (?1, 'chat.test', 'ADA', 'ada lovelace', 'ADA')",
    )
    .bind(alias.actor_id)
    .execute(&db.pool)
    .await
    .unwrap_err();
    assert!(dup
        .as_database_error()
        .map(|e| e.is_unique_violation())
        .unwrap_or(false));
}

#[tokio::test]
async fn control_characters_in_actor_names_are_client_errors() {
    let (_dir, db) = open_db().await;
    // Stripped, or rejected as invalid input: never a constraint failure (a
    // 500) from the actor table's length check.
    let stripped = db.create_semantic_actor("\0Ada").await.unwrap();
    assert_eq!(stripped.name, "Ada");
    let renamed = db
        .rename_semantic_actor(stripped.id, "\u{1}Ada L.\0")
        .await
        .unwrap();
    assert_eq!(renamed.name, "Ada L.");
    for bad in ["\u{1}\u{2}", "\0", "\u{200B}", "\u{202A}\u{202C}"] {
        assert!(
            db.create_semantic_actor(bad)
                .await
                .unwrap_err()
                .is_invalid_input(),
            "{bad:?}"
        );
        assert!(
            db.rename_semantic_actor(stripped.id, bad)
                .await
                .unwrap_err()
                .is_invalid_input(),
            "{bad:?}"
        );
    }
    assert_eq!(count(&db, "SELECT COUNT(*) FROM semantic_actors").await, 1);
    assert_eq!(
        db.get_semantic_actor(stripped.id)
            .await
            .unwrap()
            .unwrap()
            .name,
        "Ada L."
    );
}

#[tokio::test]
async fn unusable_actor_source_key_costs_only_the_assignment() {
    let (_dir, db) = open_db().await;
    // The key may be built from screen text (a workspace or account name).
    // A control character or an over-long key never fails the run: the run
    // and its items are stored and searchable, only heuristic assignment is
    // skipped.
    for (tag, key) in [
        ("nul", "\0mail:work".to_string()),
        ("ctrl", "mail:\u{1}work".to_string()),
        ("long", "k".repeat(1025)),
    ] {
        let mut run = new_run(
            &format!("bad-key-{tag}"),
            vec![message(
                "m",
                &format!("bk-{tag}"),
                &format!("badkey{tag} body"),
                Some("Ada"),
            )],
        );
        run.actor_source_key = Some(key);
        let stored = db.store_semantic_run(&run).await.unwrap();
        assert!(stored.created, "{tag}");
        assert!(stored.actor_assignment_skipped, "{tag}");
        assert_eq!(
            (stored.actors_created, stored.assignments_created),
            (0, 0),
            "{tag}"
        );
        assert_eq!(stored.item_ids.len(), 1, "{tag}");
        assert_eq!(fts_hits(&db, &format!("badkey{tag}")).await, 1, "{tag}");
    }
    assert_eq!(count(&db, "SELECT COUNT(*) FROM semantic_runs").await, 3);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM semantic_actors").await, 0);
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM semantic_actor_aliases").await,
        0
    );
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM semantic_item_actors").await,
        0
    );

    // A later run that sees the same item with a valid key assigns it.
    let mut good = new_run(
        "good-key",
        vec![message("m", "bk-nul", "badkeynul body", Some("Ada"))],
    );
    good.actor_source_key = Some("mail:work".into());
    let stored = db.store_semantic_run(&good).await.unwrap();
    assert!(!stored.actor_assignment_skipped);
    assert_eq!((stored.actors_created, stored.assignments_created), (1, 1));

    // A bad parser id as the default key is still a caller error, before any
    // write.
    let mut bad_parser = new_run("bad-parser", vec![message("m", "bp", "body", Some("Ada"))]);
    bad_parser.parser_id = "chat\u{1}".into();
    assert!(db
        .store_semantic_run(&bad_parser)
        .await
        .unwrap_err()
        .is_invalid_input());
    assert_eq!(count(&db, "SELECT COUNT(*) FROM semantic_runs").await, 4);
}

#[tokio::test]
async fn create_rename_and_search_actors() {
    let (_dir, db) = open_db().await;
    let created = db
        .create_semantic_actor("  Margaret Hamilton ")
        .await
        .unwrap();
    assert_eq!(created.name, "Margaret Hamilton");
    assert!(db
        .create_semantic_actor("   ")
        .await
        .unwrap_err()
        .is_invalid_input());
    assert!(db
        .create_semantic_actor(&"n".repeat(257))
        .await
        .unwrap_err()
        .is_invalid_input());
    assert!(db.create_semantic_actor(&"n".repeat(256)).await.is_ok());

    tokio::time::sleep(Duration::from_millis(5)).await;
    let renamed = db
        .rename_semantic_actor(created.id, "Margaret H.")
        .await
        .unwrap();
    assert_eq!(renamed.name, "Margaret H.");
    assert!(renamed.updated_at > created.updated_at);
    assert_eq!(renamed.created_at, created.created_at);
    assert!(db
        .rename_semantic_actor(99_999, "x")
        .await
        .unwrap_err()
        .is_not_found());
    assert!(db
        .rename_semantic_actor(0, "x")
        .await
        .unwrap_err()
        .is_invalid_input());

    // Heuristic actors with assignments rank above the manual one.
    db.store_semantic_run(&new_run(
        "frame-1",
        vec![
            message("m1", "1", "one", Some("Bob 100%")),
            message("m2", "2", "two", Some("Bob 100%")),
            message("m3", "3", "three", Some("Alice")),
        ],
    ))
    .await
    .unwrap();
    let all = db.search_semantic_actors("", 100, 0).await.unwrap();
    let names: Vec<&str> = all.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names[..2], ["Bob 100%", "Alice"]);
    assert_eq!(all[0].item_count, 2);
    assert_eq!(all[0].aliases.len(), 1);
    assert_eq!(all[0].aliases[0].observed_label, "Bob 100%");
    assert_eq!(all.len(), 4);

    // Matching is a case-insensitive substring of the name or an alias label.
    let bob = &all[0];
    db.rename_semantic_actor(bob.id, "Robert").await.unwrap();
    let hits = db.search_semantic_actors("bob", 10, 0).await.unwrap();
    assert_eq!(hits.len(), 1, "found through the observed alias label");
    assert_eq!(hits[0].name, "Robert");
    assert_eq!(
        db.search_semantic_actors("MARGARET", 10, 0)
            .await
            .unwrap()
            .len(),
        1
    );
    // LIKE wildcards in the query are literal.
    assert_eq!(
        db.search_semantic_actors("100%", 10, 0)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(db
        .search_semantic_actors("_", 10, 0)
        .await
        .unwrap()
        .is_empty());
    // Paging.
    assert_eq!(db.search_semantic_actors("", 2, 0).await.unwrap().len(), 2);
    assert_eq!(db.search_semantic_actors("", 2, 3).await.unwrap().len(), 1);
    assert!(db
        .search_semantic_actors(&"q".repeat(257), 10, 0)
        .await
        .unwrap_err()
        .is_invalid_input());

    let one = db.get_semantic_actor(bob.id).await.unwrap().unwrap();
    assert_eq!(one.name, "Robert");
    assert_eq!(one.item_count, 2);
    assert!(db.get_semantic_actor(99_999).await.unwrap().is_none());
}

#[tokio::test]
async fn merge_moves_aliases_and_assignments() {
    let (_dir, db) = open_db().await;
    let chat = new_run("chat", vec![message("m1", "1", "from chat", Some("Ada"))]);
    let mut mail = new_run(
        "mail",
        vec![
            message("m1", "2", "from mail", Some("A. Lovelace")),
            message("m2", "3", "more mail", Some("A. Lovelace")),
        ],
    );
    mail.actor_source_key = Some("mail:work".into());
    db.store_semantic_run(&chat).await.unwrap();
    db.store_semantic_run(&mail).await.unwrap();
    let keep = db
        .find_semantic_actor_alias("chat.test", "Ada")
        .await
        .unwrap()
        .unwrap();
    let gone = db
        .find_semantic_actor_alias("mail:work", "A. Lovelace")
        .await
        .unwrap()
        .unwrap();
    assert_ne!(keep.actor_id, gone.actor_id);

    let merged = db
        .merge_semantic_actors(keep.actor_id, gone.actor_id)
        .await
        .unwrap();
    assert_eq!(merged.actor.id, keep.actor_id);
    assert_eq!(merged.merged_actor_id, gone.actor_id);
    assert_eq!(merged.aliases_moved, 1);
    assert_eq!(merged.items_moved, 2);
    assert!(db
        .get_semantic_actor(gone.actor_id)
        .await
        .unwrap()
        .is_none());
    let kept = db.get_semantic_actor(keep.actor_id).await.unwrap().unwrap();
    assert_eq!(kept.item_count, 3);
    assert_eq!(kept.aliases.len(), 2);
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM semantic_item_actors WHERE assignment_source = 'heuristic'"
        )
        .await,
        3
    );

    // Future observations of the merged alias land on the kept actor.
    let mut later = new_run(
        "mail-2",
        vec![message("m", "4", "later mail", Some("a. lovelace"))],
    );
    later.actor_source_key = Some("mail:work".into());
    let stored = db.store_semantic_run(&later).await.unwrap();
    assert_eq!(stored.actors_created, 0);
    let detail = db
        .get_semantic_run_detail(stored.run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        detail.items[0].assigned_actor.as_ref().unwrap().actor_id,
        keep.actor_id
    );

    assert!(db
        .merge_semantic_actors(keep.actor_id, keep.actor_id)
        .await
        .unwrap_err()
        .is_invalid_input());
    assert!(db
        .merge_semantic_actors(keep.actor_id, gone.actor_id)
        .await
        .unwrap_err()
        .is_not_found());
    assert!(db
        .merge_semantic_actors(-1, keep.actor_id)
        .await
        .unwrap_err()
        .is_invalid_input());
}

#[tokio::test]
async fn reassign_item_is_preserved_over_heuristics_and_alias_moves() {
    let (_dir, db) = open_db().await;
    let run = new_run(
        "frame-1",
        vec![
            message("m1", "1", "one", Some("Sam")),
            message("m2", "2", "two", Some("Sam")),
            message("m3", "3", "three", Some("Sam")),
            message("m4", "4", "no author", None),
        ],
    );
    let stored = db.store_semantic_run(&run).await.unwrap();
    let alias = db
        .find_semantic_actor_alias("chat.test", "Sam")
        .await
        .unwrap()
        .unwrap();
    let sam_two = db.create_semantic_actor("Samantha").await.unwrap();
    let sam_one = db.create_semantic_actor("Samuel").await.unwrap();

    // An explicit correction for one item.
    let item1 = stored.item_ids[0];
    let fixed = db
        .reassign_semantic_item_actor(item1, sam_two.id)
        .await
        .unwrap();
    assert_eq!(fixed.previous_actor_id, Some(alias.actor_id));
    assert_eq!(fixed.actor_id, sam_two.id);
    assert_eq!(
        fixed.assignment_source,
        SemanticAssignmentSource::Reconciled
    );
    assert_eq!(fixed.observed_alias_id, Some(alias.id), "alias link kept");

    // Moving the alias takes the heuristic items only.
    let moved = db
        .reassign_semantic_actor_alias(alias.id, sam_one.id)
        .await
        .unwrap();
    assert_eq!(moved.previous_actor_id, alias.actor_id);
    assert_eq!(moved.alias.actor_id, sam_one.id);
    assert_eq!(moved.items_moved, 2);
    let detail = db
        .get_semantic_run_detail(stored.run_id)
        .await
        .unwrap()
        .unwrap();
    let owners: Vec<Option<i64>> = detail
        .items
        .iter()
        .map(|i| i.assigned_actor.as_ref().map(|a| a.actor_id))
        .collect();
    assert_eq!(
        owners,
        [Some(sam_two.id), Some(sam_one.id), Some(sam_one.id), None]
    );

    // Seeing the same item again does not undo the correction, and new
    // observations follow the moved alias.
    let later = new_run(
        "frame-2",
        vec![
            message("a", "1", "one", Some("Sam")),
            message("b", "5", "five", Some("SAM")),
        ],
    );
    let stored_later = db.store_semantic_run(&later).await.unwrap();
    assert_eq!(stored_later.actors_created, 0);
    let detail = db
        .get_semantic_run_detail(stored_later.run_id)
        .await
        .unwrap()
        .unwrap();
    let first = detail.items[0].assigned_actor.as_ref().unwrap();
    assert_eq!(first.actor_id, sam_two.id);
    assert_eq!(
        first.assignment_source,
        SemanticAssignmentSource::Reconciled
    );
    let second = detail.items[1].assigned_actor.as_ref().unwrap();
    assert_eq!(second.actor_id, sam_one.id);
    assert_eq!(
        second.assignment_source,
        SemanticAssignmentSource::Heuristic
    );

    // A reconciled item stays put when its alias moves again.
    let moved_back = db
        .reassign_semantic_actor_alias(alias.id, alias.actor_id)
        .await
        .unwrap();
    assert_eq!(moved_back.items_moved, 3);
    assert_eq!(
        db.get_semantic_run_detail(stored.run_id)
            .await
            .unwrap()
            .unwrap()
            .items[0]
            .assigned_actor
            .as_ref()
            .unwrap()
            .actor_id,
        sam_two.id
    );

    // Errors.
    let no_author = stored.item_ids[3];
    assert!(db
        .reassign_semantic_item_actor(no_author, sam_one.id)
        .await
        .unwrap_err()
        .is_invalid_input());
    assert!(db
        .reassign_semantic_item_actor(99_999, sam_one.id)
        .await
        .unwrap_err()
        .is_not_found());
    assert!(db
        .reassign_semantic_item_actor(item1, 99_999)
        .await
        .unwrap_err()
        .is_not_found());
    assert!(db
        .reassign_semantic_item_actor(0, sam_one.id)
        .await
        .unwrap_err()
        .is_invalid_input());
    assert!(db
        .reassign_semantic_actor_alias(99_999, sam_one.id)
        .await
        .unwrap_err()
        .is_not_found());
    assert!(db
        .reassign_semantic_actor_alias(alias.id, 99_999)
        .await
        .unwrap_err()
        .is_not_found());

    // An actor still assigned to items cannot be deleted out from under them.
    assert!(sqlx::query("DELETE FROM semantic_actors WHERE id = ?1")
        .bind(sam_two.id)
        .execute(&db.pool)
        .await
        .is_err());
}

#[tokio::test]
async fn unknown_stored_values_are_errors_on_read() {
    let (dir, db) = open_db().await;
    // Write a row a newer screenpipe might produce, bypassing the CHECK lists.
    let mut side = SqliteConnectOptions::new()
        .filename(dir.path().join("db.sqlite"))
        .connect()
        .await
        .unwrap();
    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut side)
        .await
        .unwrap();
    let run_id = sqlx::query(
        "INSERT INTO semantic_runs (input_fingerprint, parser_id, parser_version, \
         schema_version, app_platform, input_content_hash, status) \
         VALUES (?1, 'future', '9', 1, 'android', X'0000000000000000', 'empty')",
    )
    .bind(fp("future").to_vec())
    .execute(&mut side)
    .await
    .unwrap()
    .last_insert_rowid();
    side.close().await.unwrap();

    let err = db.get_semantic_run_detail(run_id).await.unwrap_err();
    match err {
        SemanticStoreError::InvalidStoredValue { column, value } => {
            assert_eq!(column, "semantic_runs.app_platform");
            assert_eq!(value, "android");
        }
        other => panic!("unexpected error {other}"),
    }
}
