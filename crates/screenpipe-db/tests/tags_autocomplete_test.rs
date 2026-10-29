// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Tag autocomplete (`ext/tags.rs`) against a real schema: counts merged
//! across frame tags, audio tags and memory JSON tags, matching, ranking,
//! escaping and limits.

use chrono::{Duration, TimeZone, Utc};
use screenpipe_db::ext::tags::{TagUsage, MAX_TAG_AUTOCOMPLETE_LIMIT};
use screenpipe_db::{DatabaseManager, TagContentType};

async fn setup_db() -> DatabaseManager {
    let db = DatabaseManager::new("sqlite::memory:", Default::default())
        .await
        .unwrap();
    screenpipe_db::schema_migrations::migrator()
        .run(db.write_pool())
        .await
        .unwrap();
    db
}

async fn frame(db: &DatabaseManager, n: i64) -> i64 {
    let at = Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap() + Duration::seconds(n);
    db.insert_snapshot_frame(
        "monitor_1",
        at,
        &format!("/data/f{n}.jpg"),
        Some("Code"),
        Some("window"),
        None,
        true,
        Some("visual_change"),
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap()
}

async fn tag(db: &DatabaseManager, id: i64, kind: TagContentType, tags: &[&str]) {
    db.add_tags(id, kind, tags.iter().map(|t| t.to_string()).collect())
        .await
        .unwrap();
}

async fn memory(db: &DatabaseManager, tags: &str) {
    db.insert_memory("fact", "user", None, Some(tags), 0.5, None)
        .await
        .unwrap();
}

fn usage(name: &str, frames: i64, audio: i64, memories: i64) -> TagUsage {
    TagUsage {
        name: name.to_string(),
        frame_count: frames,
        audio_count: audio,
        memory_count: memories,
    }
}

/// Frames, audio and memories tagged with an overlapping set of names.
async fn fixture() -> DatabaseManager {
    let db = setup_db().await;
    let f1 = frame(&db, 0).await;
    let f2 = frame(&db, 10).await;
    let f3 = frame(&db, 20).await;
    tag(&db, f1, TagContentType::Vision, &["work", "project:atlas"]).await;
    tag(&db, f2, TagContentType::Vision, &["work", "homework"]).await;
    tag(&db, f3, TagContentType::Vision, &["homework"]).await;

    let a1 = db
        .insert_audio_chunk("/data/a1.mp4", Some(Utc::now()))
        .await
        .unwrap();
    let a2 = db
        .insert_audio_chunk("/data/a2.mp4", Some(Utc::now()))
        .await
        .unwrap();
    tag(
        &db,
        a1,
        TagContentType::Audio,
        &["work", "homework", "meeting"],
    )
    .await;
    tag(&db, a2, TagContentType::Audio, &["homework"]).await;

    memory(&db, r#"["work","person:ada"]"#).await;
    memory(&db, r#"["work","work"]"#).await; // one memory, counted once
    memory(&db, r#"["Workshop",7,null,""]"#).await; // non-strings ignored
    memory(&db, "not json").await; // malformed: ignored, not an error
    memory(&db, r#"{"work":1}"#).await; // not an array: ignored
    memory(&db, r#"["homework"]"#).await;

    // A tag whose only use was removed is no longer listed.
    tag(&db, f1, TagContentType::Vision, &["stale"]).await;
    db.remove_tags(f1, TagContentType::Vision, vec!["stale".to_string()])
        .await
        .unwrap();
    db
}

#[tokio::test]
async fn counts_merge_frames_audio_and_memories() {
    let db = fixture().await;
    let all = db.autocomplete_tags("", 50).await.unwrap();
    assert_eq!(
        all,
        vec![
            usage("homework", 2, 2, 1),
            usage("work", 2, 1, 2),
            usage("meeting", 0, 1, 0),
            usage("person:ada", 0, 0, 1),
            usage("project:atlas", 1, 0, 0),
            usage("Workshop", 0, 0, 1),
        ]
    );
    assert!(all.iter().all(|t| t.name != "stale"));
    assert_eq!(all[0].total(), 5);
}

#[tokio::test]
async fn prefix_matches_rank_ahead_of_more_used_inner_matches() {
    let db = fixture().await;
    let names: Vec<String> = db
        .autocomplete_tags("wo", 50)
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect();
    // `work` and `Workshop` start with "wo" (any case); `homework` only
    // contains it, so it comes last despite having the most uses.
    assert_eq!(names, vec!["work", "Workshop", "homework"]);
}

#[tokio::test]
async fn the_part_after_a_namespace_counts_as_a_prefix() {
    let db = fixture().await;
    let found = db.autocomplete_tags("ada", 50).await.unwrap();
    assert_eq!(found, vec![usage("person:ada", 0, 0, 1)]);
    let found = db.autocomplete_tags("project:", 50).await.unwrap();
    assert_eq!(found, vec![usage("project:atlas", 1, 0, 0)]);
    let found = db.autocomplete_tags("#ATL", 50).await.unwrap();
    assert_eq!(found, vec![usage("project:atlas", 1, 0, 0)]);
}

#[tokio::test]
async fn wildcards_in_the_query_match_literally() {
    let db = setup_db().await;
    let f = frame(&db, 0).await;
    tag(
        &db,
        f,
        TagContentType::Vision,
        &["100%_done", "100x", "a\\b"],
    )
    .await;
    memory(&db, r#"["50%"]"#).await;

    let names = |v: Vec<TagUsage>| v.into_iter().map(|t| t.name).collect::<Vec<_>>();
    assert_eq!(
        names(db.autocomplete_tags("%", 50).await.unwrap()),
        vec!["100%_done", "50%"]
    );
    assert_eq!(
        names(db.autocomplete_tags("0_", 50).await.unwrap()),
        Vec::<String>::new()
    );
    assert_eq!(
        names(db.autocomplete_tags("%_", 50).await.unwrap()),
        vec!["100%_done"]
    );
    assert_eq!(
        names(db.autocomplete_tags("\\", 50).await.unwrap()),
        vec!["a\\b"]
    );
}

#[tokio::test]
async fn limit_is_clamped() {
    let db = fixture().await;
    assert_eq!(db.autocomplete_tags("", 2).await.unwrap().len(), 2);
    // Zero still returns the single best match.
    assert_eq!(
        db.autocomplete_tags("", 0).await.unwrap(),
        vec![usage("homework", 2, 2, 1)]
    );
    assert_eq!(db.autocomplete_tags("", u32::MAX).await.unwrap().len(), 6);
    assert_eq!(MAX_TAG_AUTOCOMPLETE_LIMIT, 500);
}

#[tokio::test]
async fn an_empty_database_has_no_tags() {
    let db = setup_db().await;
    assert!(db.autocomplete_tags("", 50).await.unwrap().is_empty());
    assert!(db.autocomplete_tags("wo", 50).await.unwrap().is_empty());
}
