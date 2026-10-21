// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Starred sessions in the database: optimistic revisions (also under
//! concurrent saves), the start rule, overlap listing with `has_audio`,
//! timestamp membership, coverage islands, the HD listing and rows stored in
//! other RFC 3339 spellings.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use screenpipe_db::ext::starred::{
    format_starred_timestamp, parse_starred_timestamp, CoverageIsland, StarredListFilter,
    StarredSaveOutcome, StarredSession, StarredSessionWrite,
};
use screenpipe_db::DatabaseManager;

async fn memory_db() -> DatabaseManager {
    DatabaseManager::new("sqlite::memory:", Default::default())
        .await
        .unwrap()
}

/// A database file in a temp dir, for the tests that race real connections.
async fn file_db() -> (Arc<DatabaseManager>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let db = DatabaseManager::new(path.to_str().unwrap(), Default::default())
        .await
        .unwrap();
    (Arc::new(db), dir)
}

fn at(s: &str) -> DateTime<Utc> {
    parse_starred_timestamp(s).unwrap()
}

fn base() -> DateTime<Utc> {
    at("2026-10-21T09:00:00Z")
}

fn mins(m: i64) -> DateTime<Utc> {
    base() + Duration::minutes(m)
}

fn write(id: &str, start: DateTime<Utc>, end: DateTime<Utc>, rev: i64) -> StarredSessionWrite<'_> {
    StarredSessionWrite {
        id,
        start,
        end,
        hd_requested: false,
        expected_revision: rev,
    }
}

async fn saved(db: &DatabaseManager, w: &StarredSessionWrite<'_>) -> StarredSession {
    match db.save_starred_session(w).await.unwrap() {
        StarredSaveOutcome::Saved(s) => s,
        other => panic!("expected a save, got {other:?}"),
    }
}

const A: &str = "6f1c1d4e-2b8a-4f0e-9a51-0d6c7f3b2a10";
const B: &str = "0b7e9a2c-5d43-4e8f-8c21-7a9d3e6f1b54";
const C: &str = "c3a8f5e1-7b2d-4c69-a0e4-58d1b2f97c3e";

// ── revisions ────────────────────────────────────────────────────────────

#[tokio::test]
async fn revision_zero_creates_and_each_save_bumps_by_one() {
    let db = memory_db().await;
    let first = saved(&db, &write(A, mins(0), mins(15), 0)).await;
    assert_eq!(first.revision, 1);
    assert_eq!(first.start, mins(0));
    assert_eq!(first.end, mins(15));
    assert!(!first.hd_requested);
    assert!(!first.has_audio);

    let mut edit = write(A, mins(0), mins(30), 1);
    edit.hd_requested = true;
    let second = saved(&db, &edit).await;
    assert_eq!(second.revision, 2);
    assert_eq!(second.end, mins(30));
    assert!(second.hd_requested);

    let stored: (String, String, i64, i64) = sqlx::query_as(
        "SELECT start, end, hd_requested, revision FROM starred_sessions WHERE id = ?1",
    )
    .bind(A)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        stored,
        (
            "2026-10-21T09:00:00.000Z".to_string(),
            "2026-10-21T09:30:00.000Z".to_string(),
            1,
            2
        )
    );
}

#[tokio::test]
async fn stale_revisions_and_missing_rows_are_refused_without_writing() {
    let db = memory_db().await;
    saved(&db, &write(A, mins(0), mins(15), 0)).await;
    saved(&db, &write(A, mins(0), mins(20), 1)).await;

    // Still holding revision 1 after someone else saved revision 2.
    assert_eq!(
        db.save_starred_session(&write(A, mins(0), mins(45), 1))
            .await
            .unwrap(),
        StarredSaveOutcome::RevisionConflict { current: 2 }
    );
    // A create for an id that already exists.
    assert_eq!(
        db.save_starred_session(&write(A, mins(0), mins(45), 0))
            .await
            .unwrap(),
        StarredSaveOutcome::RevisionConflict { current: 2 }
    );
    // A revision from the future.
    assert_eq!(
        db.save_starred_session(&write(A, mins(0), mins(45), 7))
            .await
            .unwrap(),
        StarredSaveOutcome::RevisionConflict { current: 2 }
    );
    // A change for a session nobody created.
    assert_eq!(
        db.save_starred_session(&write(B, mins(60), mins(70), 3))
            .await
            .unwrap(),
        StarredSaveOutcome::Missing
    );

    let row = db.get_starred_session(A).await.unwrap().unwrap();
    assert_eq!((row.revision, row.end), (2, mins(20)));
    assert!(db.get_starred_session(B).await.unwrap().is_none());
}

#[tokio::test]
async fn concurrent_saves_of_one_revision_give_one_success_and_one_conflict() {
    let (db, _dir) = file_db().await;
    saved(&db, &write(A, mins(0), mins(15), 0)).await;

    for round in 0..10i64 {
        let current = round + 1;
        let tasks: Vec<_> = (0..2i64)
            .map(|n| {
                let db = db.clone();
                tokio::spawn(async move {
                    db.save_starred_session(&write(A, mins(0), mins(20 + n + round * 2), current))
                        .await
                        .unwrap()
                })
            })
            .collect();
        let mut wins = 0;
        let mut conflicts = 0;
        for task in tasks {
            match task.await.unwrap() {
                StarredSaveOutcome::Saved(s) => {
                    assert_eq!(s.revision, current + 1);
                    wins += 1;
                }
                StarredSaveOutcome::RevisionConflict { current: c } => {
                    assert_eq!(c, current + 1);
                    conflicts += 1;
                }
                other => panic!("unexpected outcome {other:?}"),
            }
        }
        assert_eq!((wins, conflicts), (1, 1), "round {round}");
    }
    let row = db.get_starred_session(A).await.unwrap().unwrap();
    assert_eq!(row.revision, 11);
}

#[tokio::test]
async fn concurrent_creates_of_one_id_give_one_row() {
    let (db, _dir) = file_db().await;
    let tasks: Vec<_> = (0..4i64)
        .map(|n| {
            let db = db.clone();
            tokio::spawn(async move {
                db.save_starred_session(&write(C, mins(0), mins(10 + n), 0))
                    .await
                    .unwrap()
            })
        })
        .collect();
    let mut wins = 0;
    for task in tasks {
        match task.await.unwrap() {
            StarredSaveOutcome::Saved(s) => {
                assert_eq!(s.revision, 1);
                wins += 1;
            }
            StarredSaveOutcome::RevisionConflict { current } => assert_eq!(current, 1),
            other => panic!("unexpected outcome {other:?}"),
        }
    }
    assert_eq!(wins, 1);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM starred_sessions")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

// ── start rule and range guard ───────────────────────────────────────────

#[tokio::test]
async fn a_start_inside_another_session_is_refused() {
    let db = memory_db().await;
    saved(&db, &write(A, mins(0), mins(30), 0)).await;

    for start in [mins(0), mins(10), mins(29)] {
        assert_eq!(
            db.save_starred_session(&write(B, start, mins(40), 0))
                .await
                .unwrap(),
            StarredSaveOutcome::StartInsideOther {
                other_id: A.to_string()
            },
            "start {start}"
        );
    }
    // Starting where the other one ends is fine (sessions are half-open).
    let b = saved(&db, &write(B, mins(30), mins(40), 0)).await;
    assert_eq!(b.revision, 1);
    // A session may start before another and run over it.
    saved(&db, &write(C, mins(-10), mins(5), 0)).await;
    // Moving B's start into A is refused; a session never blocks itself.
    assert_eq!(
        db.save_starred_session(&write(B, mins(20), mins(40), 1))
            .await
            .unwrap(),
        StarredSaveOutcome::StartInsideOther {
            other_id: A.to_string()
        }
    );
    // A's own range does not block A; C (until minute 5) still does.
    saved(&db, &write(A, mins(5), mins(30), 1)).await;
    assert_eq!(
        db.save_starred_session(&write(A, mins(4), mins(30), 2))
            .await
            .unwrap(),
        StarredSaveOutcome::StartInsideOther {
            other_id: C.to_string()
        }
    );
}

#[tokio::test]
async fn an_empty_range_never_reaches_the_table() {
    let db = memory_db().await;
    assert_eq!(
        db.save_starred_session(&write(A, mins(5), mins(5), 0))
            .await
            .unwrap(),
        StarredSaveOutcome::InvalidRange
    );
    // Both ends collapse to the same millisecond.
    let start = mins(5) + Duration::microseconds(100);
    let end = mins(5) + Duration::microseconds(900);
    assert_eq!(
        db.save_starred_session(&write(A, start, end, 0))
            .await
            .unwrap(),
        StarredSaveOutcome::InvalidRange
    );
    assert!(db.get_starred_session(A).await.unwrap().is_none());
}

// ── listing and has_audio ────────────────────────────────────────────────

#[tokio::test]
async fn listing_returns_overlapping_sessions_newest_first_with_a_total() {
    let db = memory_db().await;
    saved(&db, &write(A, mins(0), mins(10), 0)).await;
    saved(&db, &write(B, mins(20), mins(30), 0)).await;
    saved(&db, &write(C, mins(40), mins(50), 0)).await;

    let all = |limit, offset| StarredListFilter {
        start_time: None,
        end_time: None,
        limit,
        offset,
    };
    let (rows, total) = db.list_starred_sessions(&all(100, 0)).await.unwrap();
    let ids: Vec<&str> = rows.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, vec![C, B, A]);
    assert_eq!(total, 3);

    let (page, total) = db.list_starred_sessions(&all(1, 1)).await.unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].id, B);
    assert_eq!(total, 3);

    // Overlap is strict on both ends: [10, 40) touches A's end and C's start.
    let (rows, total) = db
        .list_starred_sessions(&StarredListFilter {
            start_time: Some(mins(10)),
            end_time: Some(mins(40)),
            limit: 100,
            offset: 0,
        })
        .await
        .unwrap();
    assert_eq!(rows.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), [B]);
    assert_eq!(total, 1);

    // Open-ended on one side.
    let (rows, _) = db
        .list_starred_sessions(&StarredListFilter {
            start_time: Some(mins(25)),
            end_time: None,
            limit: 100,
            offset: 0,
        })
        .await
        .unwrap();
    assert_eq!(
        rows.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        [C, B]
    );
}

#[tokio::test]
async fn has_audio_counts_chunks_with_a_file_inside_the_half_open_range() {
    let db = memory_db().await;
    saved(&db, &write(A, mins(0), mins(10), 0)).await;
    saved(&db, &write(B, mins(20), mins(30), 0)).await;
    saved(&db, &write(C, mins(40), mins(50), 0)).await;

    // A: a chunk exactly at the start counts.
    db.insert_audio_chunk("/audio/a.mp4", Some(mins(0)))
        .await
        .unwrap();
    // B: a chunk exactly at the end does not, nor one with no file.
    db.insert_audio_chunk("/audio/b.mp4", Some(mins(30)))
        .await
        .unwrap();
    db.insert_audio_chunk("", Some(mins(25))).await.unwrap();
    // C: a chunk stored in SQLite's space-separated form counts.
    sqlx::query("INSERT INTO audio_chunks (file_path, timestamp) VALUES (?1, ?2)")
        .bind("/audio/c.mp4")
        .bind("2026-10-21 09:49:59.500")
        .execute(&db.pool)
        .await
        .unwrap();

    let (rows, _) = db
        .list_starred_sessions(&StarredListFilter {
            limit: 100,
            ..Default::default()
        })
        .await
        .unwrap();
    let flags: Vec<(&str, bool)> = rows
        .iter()
        .map(|s| (s.id.as_str(), s.has_audio))
        .collect();
    assert_eq!(flags, vec![(C, true), (B, false), (A, true)]);

    // Sub-second precision decides at the edges.
    let edge = "1d2e3f40-5a6b-4c7d-8e9f-a0b1c2d3e4f5";
    saved(
        &db,
        &write(
            edge,
            mins(60) + Duration::milliseconds(700),
            mins(61) + Duration::milliseconds(200),
            0,
        ),
    )
    .await;
    db.insert_audio_chunk("/audio/early.mp4", Some(mins(60) + Duration::milliseconds(500)))
        .await
        .unwrap();
    assert!(!db.get_starred_session(edge).await.unwrap().unwrap().has_audio);
    db.insert_audio_chunk("/audio/late.mp4", Some(mins(61) + Duration::milliseconds(100)))
        .await
        .unwrap();
    assert!(db.get_starred_session(edge).await.unwrap().unwrap().has_audio);
}

#[tokio::test]
async fn has_audio_uses_the_audio_timestamp_index() {
    let db = memory_db().await;
    let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(
        "EXPLAIN QUERY PLAN SELECT EXISTS ( \
             SELECT 1 FROM audio_chunks a \
             WHERE ((a.timestamp >= strftime('%Y-%m-%dT%H:%M:%S', s.start) \
                     AND a.timestamp < strftime('%Y-%m-%dT%H:%M:%S', s.end, '+1 second')) \
                 OR (a.timestamp >= strftime('%Y-%m-%d %H:%M:%S', s.start) \
                     AND a.timestamp < strftime('%Y-%m-%d %H:%M:%S', s.end, '+1 second'))) \
               AND julianday(a.timestamp) >= julianday(s.start) \
               AND julianday(a.timestamp) < julianday(s.end) \
               AND a.file_path != '') \
         FROM starred_sessions s",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    let details: Vec<&str> = plan.iter().map(|r| r.3.as_str()).collect();
    assert!(
        details
            .iter()
            .any(|d| d.contains("audio_chunks") && d.contains("idx_audio_chunks_timestamp")),
        "audio_chunks is scanned without its timestamp index: {details:?}"
    );
    assert!(
        !details.iter().any(|d| *d == "SCAN a"),
        "full scan of audio_chunks: {details:?}"
    );
}

// ── rows written in other spellings ──────────────────────────────────────

#[tokio::test]
async fn rows_stored_in_other_rfc3339_spellings_read_back() {
    let db = memory_db().await;
    // Each row keeps one spelling for both ends: `CHECK (end > start)`
    // compares text, so a writer cannot mix them within a row.
    sqlx::query(
        "INSERT INTO starred_sessions (id, start, end, hd_requested, revision) VALUES \
         (?1, '2026-10-21T09:00:00+00:00', '2026-10-21T09:10:00.123456+00:00', 1, 4), \
         (?2, '2026-10-21T11:20:00+02:00', '2026-10-21T11:30:00+02:00', 0, 1), \
         (?3, '2026-10-21 10:00:00', '2026-10-21 10:05:00.5', 0, 2)",
    )
    .bind(A)
    .bind(B)
    .bind(C)
    .execute(&db.pool)
    .await
    .unwrap();

    let a = db.get_starred_session(A).await.unwrap().unwrap();
    assert_eq!(a.start, mins(0));
    assert_eq!(a.end, mins(10) + Duration::milliseconds(123));
    assert!(a.hd_requested);
    assert_eq!(a.revision, 4);
    let b = db.get_starred_session(B).await.unwrap().unwrap();
    assert_eq!((b.start, b.end), (mins(20), mins(30)));
    let c = db.get_starred_session(C).await.unwrap().unwrap();
    assert_eq!(
        (c.start, c.end, c.revision),
        (mins(60), mins(65) + Duration::milliseconds(500), 2)
    );

    // Saving with the stored revision rewrites the row in the canonical form.
    let edited = saved(&db, &write(A, mins(0), mins(12), 4)).await;
    assert_eq!(edited.revision, 5);
    let stored: String = sqlx::query_scalar("SELECT end FROM starred_sessions WHERE id = ?1")
        .bind(A)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(stored, format_starred_timestamp(mins(12)));

    // The start rule and overlap listing compare by time, not by text.
    let d = "9a8b7c6d-5e4f-4a3b-9c2d-1e0f9a8b7c6d";
    assert_eq!(
        db.save_starred_session(&write(d, mins(25), mins(35), 0))
            .await
            .unwrap(),
        StarredSaveOutcome::StartInsideOther {
            other_id: B.to_string()
        }
    );
    assert_eq!(
        db.save_starred_session(&write(d, mins(62), mins(70), 0))
            .await
            .unwrap(),
        StarredSaveOutcome::StartInsideOther {
            other_id: C.to_string()
        }
    );
    let (rows, _) = db
        .list_starred_sessions(&StarredListFilter {
            start_time: Some(mins(29)),
            end_time: Some(mins(31)),
            limit: 10,
            offset: 0,
        })
        .await
        .unwrap();
    assert_eq!(rows.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), [B]);
}

// ── membership and coverage ──────────────────────────────────────────────

#[tokio::test]
async fn membership_reports_each_timestamp_in_input_order() {
    let db = memory_db().await;
    saved(&db, &write(A, mins(0), mins(10), 0)).await;
    saved(&db, &write(B, mins(20), mins(30), 0)).await;

    let probes = [
        mins(25),
        mins(-1),
        mins(0),
        mins(10),
        mins(9) + Duration::seconds(59),
        mins(15),
        mins(20),
    ];
    assert_eq!(
        db.starred_membership(&probes, None).await.unwrap(),
        vec![true, false, true, false, true, false, true]
    );
    assert_eq!(
        db.starred_membership(&probes, Some(B)).await.unwrap(),
        vec![true, false, false, false, false, false, true]
    );
    assert!(db.starred_membership(&[], None).await.unwrap().is_empty());
    // Duplicates keep their own slots.
    assert_eq!(
        db.starred_membership(&[mins(5), mins(5), mins(50)], None)
            .await
            .unwrap(),
        vec![true, true, false]
    );
}

#[tokio::test]
async fn coverage_islands_merge_overlaps_and_clip_to_the_window() {
    let db = memory_db().await;
    saved(&db, &write(A, mins(0), mins(10), 0)).await;
    // Starts before A and runs into it.
    saved(&db, &write(B, mins(-5), mins(3), 0)).await;
    saved(&db, &write(C, mins(10), mins(20), 0)).await; // touches A
    let d = "9a8b7c6d-5e4f-4a3b-9c2d-1e0f9a8b7c6d";
    saved(&db, &write(d, mins(40), mins(70), 0)).await;

    let islands = db
        .starred_coverage_islands(mins(-2), mins(60), None)
        .await
        .unwrap();
    assert_eq!(
        islands,
        vec![
            CoverageIsland {
                start: mins(-2),
                end: mins(20)
            },
            CoverageIsland {
                start: mins(40),
                end: mins(60)
            },
        ]
    );
    assert_eq!(
        db.starred_coverage_islands(mins(-60), mins(60), Some(A))
            .await
            .unwrap(),
        vec![CoverageIsland {
            start: mins(0),
            end: mins(10)
        }]
    );
    assert!(db
        .starred_coverage_islands(mins(25), mins(35), None)
        .await
        .unwrap()
        .is_empty());
}

// ── HD listing ───────────────────────────────────────────────────────────

#[tokio::test]
async fn hd_listing_holds_only_unfinished_sessions_that_asked_for_hd() {
    let db = memory_db().await;
    let mut a = write(A, mins(0), mins(10), 0);
    a.hd_requested = true;
    saved(&db, &a).await;
    let mut b = write(B, mins(20), mins(30), 0);
    b.hd_requested = true;
    saved(&db, &b).await;
    saved(&db, &write(C, mins(40), mins(50), 0)).await; // no HD

    let ids = |rows: Vec<screenpipe_db::ext::starred::HdStarredSession>| {
        rows.into_iter().map(|r| r.id).collect::<Vec<_>>()
    };
    assert_eq!(
        ids(db.list_hd_starred_sessions(mins(5), 10).await.unwrap()),
        vec![A.to_string(), B.to_string()]
    );
    // A session that ended at `now` is over.
    assert_eq!(
        ids(db.list_hd_starred_sessions(mins(10), 10).await.unwrap()),
        vec![B.to_string()]
    );
    assert!(db
        .list_hd_starred_sessions(mins(30), 10)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(db.list_hd_starred_sessions(mins(5), 1).await.unwrap().len(), 1);

    // Turning HD off removes it.
    let mut off = write(B, mins(20), mins(30), 1);
    off.hd_requested = false;
    saved(&db, &off).await;
    assert_eq!(
        ids(db.list_hd_starred_sessions(mins(15), 10).await.unwrap()),
        Vec::<String>::new()
    );
    let rows = db.list_hd_starred_sessions(mins(5), 10).await.unwrap();
    assert_eq!(rows[0].revision, 1);
    assert_eq!((rows[0].start, rows[0].end), (mins(0), mins(10)));
}
