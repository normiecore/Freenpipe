// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Frame media lookups behind the thumbnail, preview-sample and
//! preview-media routes (`ext/frames_preview.rs`), against a real schema.

use chrono::{DateTime, Duration, TimeZone, Utc};
use screenpipe_db::ext::frames_preview::{FrameMediaLocation, FramePreviewFilter};
use screenpipe_db::DatabaseManager;

async fn setup_db() -> DatabaseManager {
    let db = DatabaseManager::new("sqlite::memory:", Default::default())
        .await
        .unwrap();
    screenpipe_db::schema_migrations::migrator()
        .run(&db.pool)
        .await
        .unwrap();
    db
}

fn at(secs: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap() + Duration::seconds(secs)
}

async fn snapshot(
    db: &DatabaseManager,
    device: &str,
    secs: i64,
    path: &str,
    app: &str,
    url: Option<&str>,
) -> i64 {
    db.insert_snapshot_frame(
        device,
        at(secs),
        path,
        Some(app),
        Some("window"),
        url,
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

#[tokio::test]
async fn exact_lookup_reports_each_media_location() {
    let db = setup_db().await;
    let snap = snapshot(&db, "monitor_1", 0, "/data/a.jpg", "Code", None).await;
    let row = db.get_frame_media(snap).await.unwrap().unwrap();
    assert_eq!(row.frame_id, snap);
    assert_eq!(row.timestamp, at(0));
    assert_eq!(row.device_name, "monitor_1");
    assert_eq!(
        row.location(),
        FrameMediaLocation::Snapshot {
            path: "/data/a.jpg".into()
        }
    );

    // Compaction folds the snapshot into an MP4 at position 4.
    let chunk = db
        .insert_video_chunk_with_fps("/data/chunk.mp4", "monitor_1", 2.0)
        .await
        .unwrap();
    db.compact_snapshots_queued(chunk, vec![(snap, 4)])
        .await
        .unwrap();
    let row = db.get_frame_media(snap).await.unwrap().unwrap();
    assert_eq!(
        row.location(),
        FrameMediaLocation::Video {
            chunk_id: chunk,
            path: "/data/chunk.mp4".into(),
            offset_index: 4,
            fps: Some(2.0),
        }
    );
    assert_eq!(row.video_offset_seconds(), Some(2.0));

    // Media retention empties the chunk's path: no location left.
    sqlx::query(
        "UPDATE video_chunks SET file_path = '', evicted_at = CURRENT_TIMESTAMP WHERE id = ?1",
    )
    .bind(chunk)
    .execute(&db.pool)
    .await
    .unwrap();
    let row = db.get_frame_media(snap).await.unwrap().unwrap();
    assert_eq!(row.location(), FrameMediaLocation::Unavailable);

    let evicted = db.get_preview_media_chunk(chunk).await.unwrap().unwrap();
    assert!(evicted.evicted);
    assert!(evicted.file_path.is_empty());

    assert!(db.get_frame_media(9_999).await.unwrap().is_none());
    assert!(db.get_preview_media_chunk(9_999).await.unwrap().is_none());
}

/// Compaction must re-point exactly the frames of its batch, whatever the
/// chunk id is (four older chunks make it differ from every frame id).
#[tokio::test]
async fn compaction_moves_exactly_the_batch_frames() {
    let db = setup_db().await;
    let untouched = snapshot(&db, "monitor_1", 0, "/d/0.jpg", "Code", None).await;
    let first = snapshot(&db, "monitor_1", 1, "/d/1.jpg", "Code", None).await;
    let second = snapshot(&db, "monitor_1", 2, "/d/2.jpg", "Code", None).await;
    let last = snapshot(&db, "monitor_1", 3, "/d/3.jpg", "Code", None).await;
    for old in 0..4 {
        db.insert_video_chunk_with_fps(&format!("/d/old-{old}.mp4"), "monitor_1", 1.0)
            .await
            .unwrap();
    }
    let chunk = db
        .insert_video_chunk_with_fps("/d/new.mp4", "monitor_1", 1.0)
        .await
        .unwrap();
    assert!(![untouched, first, second, last].contains(&chunk));

    db.compact_snapshots_queued(chunk, vec![(first, 0), (second, 1), (last, 2)])
        .await
        .unwrap();

    for (id, pos) in [(first, 0), (second, 1), (last, 2)] {
        let row = db.get_frame_media(id).await.unwrap().unwrap();
        assert_eq!(
            row.location(),
            FrameMediaLocation::Video {
                chunk_id: chunk,
                path: "/d/new.mp4".into(),
                offset_index: pos,
                fps: Some(1.0),
            },
            "frame {id}"
        );
    }
    let row = db.get_frame_media(untouched).await.unwrap().unwrap();
    assert_eq!(
        row.location(),
        FrameMediaLocation::Snapshot {
            path: "/d/0.jpg".into()
        }
    );
}

#[tokio::test]
async fn neighbours_stay_on_the_same_device_and_close_in_time() {
    let db = setup_db().await;
    let a1 = snapshot(&db, "monitor_1", 0, "/d/1.jpg", "Code", None).await;
    let _b = snapshot(&db, "monitor_2", 1, "/d/2.jpg", "Code", None).await;
    let anchor = snapshot(&db, "monitor_1", 2, "/d/3.jpg", "Code", None).await;
    let a2 = snapshot(&db, "monitor_1", 3, "/d/4.jpg", "Code", None).await;
    let far = snapshot(&db, "monitor_1", 3 * 3600, "/d/5.jpg", "Code", None).await;
    let remote = snapshot(&db, "monitor_1", 4, "cloud://frames/6", "Code", None).await;

    let anchor_row = db.get_frame_media(anchor).await.unwrap().unwrap();
    let back = db
        .get_frame_media_neighbours(&anchor_row, false, 3)
        .await
        .unwrap();
    assert_eq!(
        back.iter().map(|r| r.frame_id).collect::<Vec<_>>(),
        vec![a1]
    );

    let fwd = db
        .get_frame_media_neighbours(&anchor_row, true, 5)
        .await
        .unwrap();
    let ids: Vec<i64> = fwd.iter().map(|r| r.frame_id).collect();
    assert_eq!(
        ids,
        vec![a2],
        "the 3 h-later frame and the remote frame are skipped"
    );
    assert!(!ids.contains(&far) && !ids.contains(&remote));

    assert!(db
        .get_frame_media_neighbours(&anchor_row, true, 0)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn preview_candidates_spread_over_the_active_span() {
    let db = setup_db().await;
    // One Code frame per minute for an hour, plus frames of other apps.
    let mut code = Vec::new();
    for minute in 0..60 {
        code.push(
            snapshot(
                &db,
                "monitor_1",
                minute * 60,
                &format!("/d/c{minute}.jpg"),
                "Code",
                None,
            )
            .await,
        );
        snapshot(
            &db,
            "monitor_1",
            minute * 60 + 30,
            &format!("/d/s{minute}.jpg"),
            "Slack",
            None,
        )
        .await;
    }
    let filter = FramePreviewFilter {
        start: at(-3600),
        end: at(5 * 3600),
        app_name: "code", // case-insensitive
        browser_domain: None,
    };
    let slices = db
        .get_frame_preview_candidates(&filter, 6, 2)
        .await
        .unwrap();
    assert_eq!(slices.len(), 6, "the hour of activity is cut into 6 slices");
    let firsts: Vec<i64> = slices.iter().map(|s| s[0].frame_id).collect();
    // Slices start every 59 minutes / 6, at minutes 0, 9.8, 19.7, 29.5, 39.3, 49.2.
    assert_eq!(
        firsts,
        vec![code[0], code[10], code[20], code[30], code[40], code[50]]
    );
    for slice in &slices {
        assert!(slice.len() <= 2);
        assert!(slice.windows(2).all(|w| w[0].timestamp <= w[1].timestamp));
    }

    // The range end is exclusive and other apps never leak in.
    let filter = FramePreviewFilter {
        start: at(0),
        end: at(60),
        app_name: "Code",
        browser_domain: None,
    };
    let slices = db
        .get_frame_preview_candidates(&filter, 6, 4)
        .await
        .unwrap();
    let ids: Vec<i64> = slices.iter().flatten().map(|r| r.frame_id).collect();
    assert_eq!(ids, vec![code[0]]);

    let nothing = FramePreviewFilter {
        start: at(0),
        end: at(3600),
        app_name: "Figma",
        browser_domain: None,
    };
    assert!(db
        .get_frame_preview_candidates(&nothing, 6, 4)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn preview_candidates_filter_by_domain_and_media() {
    let db = setup_db().await;
    let gh = snapshot(
        &db,
        "m",
        0,
        "/d/1.jpg",
        "Chrome",
        Some("https://github.com/a"),
    )
    .await;
    let docs = snapshot(
        &db,
        "m",
        10,
        "/d/2.jpg",
        "Chrome",
        Some("https://docs.github.com/b"),
    )
    .await;
    let _lookalike = snapshot(
        &db,
        "m",
        20,
        "/d/3.jpg",
        "Chrome",
        Some("https://notgithub.com/"),
    )
    .await;
    let _query = snapshot(
        &db,
        "m",
        30,
        "/d/4.jpg",
        "Chrome",
        Some("https://example.com/?q=github.com"),
    )
    .await;
    let _remote = snapshot(
        &db,
        "m",
        40,
        "cloud://x",
        "Chrome",
        Some("https://github.com/c"),
    )
    .await;
    let no_media = snapshot(
        &db,
        "m",
        50,
        "/d/6.jpg",
        "Chrome",
        Some("https://github.com/d"),
    )
    .await;
    sqlx::query("UPDATE frames SET snapshot_path = NULL WHERE id = ?1")
        .bind(no_media)
        .execute(&db.pool)
        .await
        .unwrap();

    let filter = FramePreviewFilter {
        start: at(-10),
        end: at(100),
        app_name: "Chrome",
        browser_domain: Some("github.com"),
    };
    let slices = db
        .get_frame_preview_candidates(&filter, 12, 4)
        .await
        .unwrap();
    let mut ids: Vec<i64> = slices.iter().flatten().map(|r| r.frame_id).collect();
    ids.sort();
    assert_eq!(ids, vec![gh, docs]);

    // Degenerate requests return nothing instead of failing.
    let empty_range = FramePreviewFilter {
        start: at(100),
        end: at(100),
        ..filter.clone()
    };
    assert!(db
        .get_frame_preview_candidates(&empty_range, 6, 4)
        .await
        .unwrap()
        .is_empty());
    assert!(db
        .get_frame_preview_candidates(&filter, 0, 4)
        .await
        .unwrap()
        .is_empty());
}

/// URLs that name the domain only outside their host pass the SQL `LIKE`
/// pre-filter. The edge lookups and each slice must page past more of them
/// than one page holds to reach the real matches.
#[tokio::test]
async fn preview_candidates_page_past_domain_false_positives() {
    let db = setup_db().await;
    let decoy = Some("https://www.google.com/search?q=github.com");
    let mut decoy_secs = Vec::new();
    decoy_secs.extend(0..100); // before the first match
    decoy_secs.extend(101..400); // between the matches: 150 of them in the second slice
    decoy_secs.extend(401..500); // after the last match
    for secs in decoy_secs {
        snapshot(&db, "m", secs, &format!("/d/q{secs}.jpg"), "Chrome", decoy).await;
    }
    let first = snapshot(
        &db,
        "m",
        100,
        "/d/r1.jpg",
        "Chrome",
        Some("https://github.com/a"),
    )
    .await;
    let last = snapshot(
        &db,
        "m",
        400,
        "/d/r2.jpg",
        "Chrome",
        Some("https://github.com/b"),
    )
    .await;

    let filter = FramePreviewFilter {
        start: at(-10),
        end: at(1000),
        app_name: "Chrome",
        browser_domain: Some("github.com"),
    };
    let slices = db
        .get_frame_preview_candidates(&filter, 2, 1)
        .await
        .unwrap();
    let ids: Vec<i64> = slices.iter().flatten().map(|r| r.frame_id).collect();
    assert_eq!(ids, vec![first, last]);
    for row in slices.iter().flatten() {
        assert!(row
            .browser_url
            .as_deref()
            .is_some_and(|u| u.starts_with("https://github.com/")));
    }
}
