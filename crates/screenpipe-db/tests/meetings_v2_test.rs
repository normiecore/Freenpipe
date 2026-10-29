// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Meetings v2 database behaviour: conditional calendar binding, the room
//! change switch, the rejoin lookup, the summary section writer, the summary
//! run snapshot, the `q` filter over `meeting_app` and the transcript offset.

use chrono::{Duration, Utc};
use screenpipe_db::ext::meetings_v2::{
    is_calendar_event_conflict, CalendarBinding, MeetingSummaryState, RoomSwitch,
};
use screenpipe_db::{
    AudioDevice, DatabaseManager, DeviceType, MEETING_DETECTION_SOURCE_ROOM_CHANGE,
    MEETING_END_REASON_EXPLICIT_STOP, MEETING_END_REASON_ROOM_CHANGED,
};

async fn setup_test_db() -> DatabaseManager {
    let db = DatabaseManager::new("sqlite::memory:", Default::default())
        .await
        .unwrap();
    screenpipe_db::schema_migrations::migrator()
        .run(db.write_pool())
        .await
        .expect("migrations");
    db
}

fn ts(offset_secs: i64) -> String {
    (Utc::now() + Duration::seconds(offset_secs))
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

async fn end_reason(db: &DatabaseManager, id: i64) -> Option<String> {
    let row: (Option<String>,) = sqlx::query_as("SELECT end_reason FROM meetings WHERE id = ?1")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    row.0
}

/// Insert and immediately end a meeting so the next one can be opened
/// (`idx_meetings_single_open` allows one open row).
async fn ended_meeting(db: &DatabaseManager, app: &str, reason: Option<&str>) -> i64 {
    let id = db.insert_meeting(app, "ui_scan", None, None).await.unwrap();
    db.end_meeting(id, &ts(0), reason).await.unwrap();
    id
}

// ── calendar binding ─────────────────────────────────────────────────────

#[tokio::test]
async fn binding_claims_an_event_for_one_meeting_only() {
    let db = setup_test_db().await;
    let first = ended_meeting(&db, "zoom.us", None).await;
    let second = ended_meeting(&db, "zoom.us", None).await;

    assert_eq!(
        db.bind_meeting_calendar_event(first, "evt1").await.unwrap(),
        CalendarBinding::Bound
    );
    assert_eq!(
        db.get_meeting_calendar_event_id(first).await.unwrap(),
        Some("evt1".to_string())
    );
    // Idempotent for the owner.
    assert_eq!(
        db.bind_meeting_calendar_event(first, " evt1 ")
            .await
            .unwrap(),
        CalendarBinding::AlreadyBound
    );
    // Another meeting cannot take it, and nothing errors.
    assert_eq!(
        db.bind_meeting_calendar_event(second, "evt1")
            .await
            .unwrap(),
        CalendarBinding::OwnedElsewhere {
            other_meeting_id: Some(first)
        }
    );
    assert_eq!(
        db.get_meeting_calendar_event_id(second).await.unwrap(),
        None
    );
    // An existing binding is never overwritten.
    assert_eq!(
        db.bind_meeting_calendar_event(first, "evt2").await.unwrap(),
        CalendarBinding::MeetingHasOtherEvent {
            existing: "evt1".to_string()
        }
    );
    assert_eq!(
        db.bind_meeting_calendar_event(9999, "evt3").await.unwrap(),
        CalendarBinding::MeetingMissing
    );
    assert!(db.bind_meeting_calendar_event(second, "  ").await.is_err());
}

#[tokio::test]
async fn unique_index_violation_is_recognised_as_a_binding_conflict() {
    let db = setup_test_db().await;
    let first = ended_meeting(&db, "Teams", None).await;
    let second = ended_meeting(&db, "Teams", None).await;
    sqlx::query("UPDATE meetings SET calendar_event_id = 'evt' WHERE id = ?1")
        .bind(first)
        .execute(db.write_pool())
        .await
        .unwrap();
    let err = sqlx::query("UPDATE meetings SET calendar_event_id = 'evt' WHERE id = ?1")
        .bind(second)
        .execute(db.write_pool())
        .await
        .unwrap_err();
    assert!(is_calendar_event_conflict(&err), "{err}");

    // Empty strings are outside the partial index: any number may hold one.
    for id in [first, second] {
        sqlx::query("UPDATE meetings SET calendar_event_id = '' WHERE id = ?1")
            .bind(id)
            .execute(db.write_pool())
            .await
            .unwrap();
    }
    assert_eq!(
        db.bind_meeting_calendar_event(second, "evt").await.unwrap(),
        CalendarBinding::Bound
    );
}

// ── rejoin lookup ────────────────────────────────────────────────────────

#[tokio::test]
async fn rejoin_lookup_ignores_room_changed_meetings() {
    let db = setup_test_db().await;
    ended_meeting(&db, "Google Chrome", Some(MEETING_END_REASON_ROOM_CHANGED)).await;
    assert!(
        db.find_recent_meeting_for_app("Google Chrome", 120)
            .await
            .unwrap()
            .is_none(),
        "a room_changed meeting must never be reopened"
    );

    // explicit_stop stays excluded; natural ends stay eligible.
    ended_meeting(&db, "Zoom", Some(MEETING_END_REASON_EXPLICIT_STOP)).await;
    assert!(db
        .find_recent_meeting_for_app("Zoom", 120)
        .await
        .unwrap()
        .is_none());
    let natural = ended_meeting(&db, "Zoom", None).await;
    assert_eq!(
        db.find_recent_meeting_for_app("Zoom", 120)
            .await
            .unwrap()
            .map(|m| m.id),
        Some(natural)
    );
}

#[tokio::test]
async fn rejoin_lookup_skips_a_newer_room_changed_row_for_an_older_eligible_one() {
    let db = setup_test_db().await;
    let eligible = ended_meeting(&db, "Slack", Some("auto_end")).await;
    ended_meeting(&db, "Slack", Some(MEETING_END_REASON_ROOM_CHANGED)).await;
    assert_eq!(
        db.find_recent_meeting_for_app("Slack", 120)
            .await
            .unwrap()
            .map(|m| m.id),
        Some(eligible)
    );
}

// ── room change ──────────────────────────────────────────────────────────

#[tokio::test]
async fn room_switch_ends_the_meeting_and_opens_a_successor() {
    let db = setup_test_db().await;
    let old = db
        .insert_meeting("Google Chrome", "ui_scan", Some("Design review"), None)
        .await
        .unwrap();
    db.bind_meeting_calendar_event(old, "evt-design")
        .await
        .unwrap();

    let RoomSwitch::Switched {
        ended_id,
        new_id,
        meeting_app,
        ..
    } = db.switch_meeting_room(old).await.unwrap()
    else {
        panic!("expected a switch");
    };
    assert_eq!(ended_id, old);
    assert_eq!(meeting_app, "Google Chrome");

    let ended = db.get_meeting_by_id(old).await.unwrap();
    assert!(ended.meeting_end.is_some());
    assert_eq!(ended.title.as_deref(), Some("Design review"));
    assert_eq!(
        end_reason(&db, old).await.as_deref(),
        Some(MEETING_END_REASON_ROOM_CHANGED)
    );

    let fresh = db.get_meeting_by_id(new_id).await.unwrap();
    assert!(fresh.meeting_end.is_none());
    assert_eq!(fresh.meeting_app, "Google Chrome");
    assert_eq!(fresh.detection_source, MEETING_DETECTION_SOURCE_ROOM_CHANGE);
    assert_eq!(fresh.title, None);
    assert_eq!(
        db.get_meeting_calendar_event_id(new_id).await.unwrap(),
        None
    );
    assert_eq!(
        db.get_most_recent_active_meeting()
            .await
            .unwrap()
            .map(|m| m.id),
        Some(new_id)
    );

    // The old meeting is no longer active, so a second answer cannot switch.
    assert_eq!(
        db.switch_meeting_room(old).await.unwrap(),
        RoomSwitch::NotActive
    );
    assert_eq!(
        db.switch_meeting_room(424242).await.unwrap(),
        RoomSwitch::Missing
    );
    // And the rejoin lookup will not merge the next call back into it.
    db.end_meeting(new_id, &ts(0), Some(MEETING_END_REASON_EXPLICIT_STOP))
        .await
        .unwrap();
    assert!(db
        .find_recent_meeting_for_app("Google Chrome", 120)
        .await
        .unwrap()
        .is_none());
}

// ── summaries ────────────────────────────────────────────────────────────

#[tokio::test]
async fn summary_save_appends_then_replaces_the_section() {
    let db = setup_test_db().await;
    let id = ended_meeting(&db, "manual", None).await;
    db.update_meeting(id, None, None, None, None, Some("my own notes"), None)
        .await
        .unwrap();

    let saved = db
        .save_meeting_summary(id, "first pass", None)
        .await
        .unwrap()
        .expect("meeting exists");
    assert_eq!(
        saved.note.as_deref(),
        Some("my own notes\n\n## Summary\nfirst pass")
    );
    assert_eq!(saved.title, None);

    let saved = db
        .save_meeting_summary(id, "second pass", Some("  Budget sync  "))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        saved.note.as_deref(),
        Some("my own notes\n\n## Summary\nsecond pass")
    );
    assert_eq!(saved.title.as_deref(), Some("Budget sync"));

    // A blank title leaves the stored one alone.
    let saved = db
        .save_meeting_summary(id, "third pass", Some(" "))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.title.as_deref(), Some("Budget sync"));

    assert!(db
        .save_meeting_summary(777, "x", None)
        .await
        .unwrap()
        .is_none());
}

async fn insert_execution(
    db: &DatabaseManager,
    pipe: &str,
    status: &str,
    trigger_key: &str,
    started_at: &str,
    error_type: Option<&str>,
) -> i64 {
    sqlx::query(
        "INSERT INTO pipe_executions \
         (pipe_name, status, trigger_type, started_at, error_type, error_message, \
          trigger_event, trigger_key) \
         VALUES (?1, ?2, 'event', ?3, ?4, ?5, 'meeting_ended', ?6)",
    )
    .bind(pipe)
    .bind(status)
    .bind(started_at)
    .bind(error_type)
    .bind(error_type.map(|_| "raw error"))
    .bind(trigger_key)
    .execute(db.write_pool())
    .await
    .unwrap()
    .last_insert_rowid()
}

async fn insert_claim(db: &DatabaseManager, pipe: &str, key: &str, claimed_at: &str) {
    sqlx::query(
        "INSERT INTO pipe_event_runs (pipe_name, event_name, event_key, claimed_at) \
         VALUES (?1, 'meeting_ended', ?2, ?3)",
    )
    .bind(pipe)
    .bind(key)
    .bind(claimed_at)
    .execute(db.write_pool())
    .await
    .unwrap();
}

#[tokio::test]
async fn summary_snapshot_is_idle_without_runs() {
    let db = setup_test_db().await;
    let id = ended_meeting(&db, "Zoom", None).await;
    let snap = db
        .meeting_summary_run_snapshot(id, "meeting-summary", None)
        .await
        .unwrap();
    assert_eq!(snap.state(), MeetingSummaryState::Idle);
    assert!(snap.latest_execution.is_none());
}

#[tokio::test]
async fn summary_snapshot_follows_the_newest_matching_execution() {
    let db = setup_test_db().await;
    let id = ended_meeting(&db, "Zoom", None).await;
    let other = ended_meeting(&db, "Zoom", None).await;
    let pipe = "meeting-summary";

    // Rows that must not match: another meeting, a key that only shares a
    // prefix, another pipe, another event.
    insert_execution(&db, pipe, "running", &other.to_string(), &ts(-30), None).await;
    insert_execution(&db, pipe, "running", &format!("{id}0"), &ts(-30), None).await;
    insert_execution(&db, "day-recap", "running", &id.to_string(), &ts(-30), None).await;
    sqlx::query(
        "INSERT INTO pipe_executions (pipe_name, status, trigger_type, started_at, \
         trigger_event, trigger_key) VALUES (?1, 'running', 'event', ?2, 'meeting_started', ?3)",
    )
    .bind(pipe)
    .bind(ts(-30))
    .bind(id.to_string())
    .execute(db.write_pool())
    .await
    .unwrap();
    assert_eq!(
        db.meeting_summary_run_snapshot(id, pipe, None)
            .await
            .unwrap()
            .state(),
        MeetingSummaryState::Idle
    );

    let failed = insert_execution(
        &db,
        pipe,
        "failed",
        &id.to_string(),
        &ts(-20),
        Some("summary_not_saved"),
    )
    .await;
    let snap = db
        .meeting_summary_run_snapshot(id, pipe, None)
        .await
        .unwrap();
    assert_eq!(snap.state(), MeetingSummaryState::Failed);
    let exec = snap.latest_execution.unwrap();
    assert_eq!(exec.id, failed);
    assert_eq!(exec.error_type.as_deref(), Some("summary_not_saved"));

    // A recovery run (key `<id>:...`) is newer and wins.
    let recovery = insert_execution(
        &db,
        pipe,
        "running",
        &format!("{id}:summary-recovery-1"),
        &ts(-10),
        None,
    )
    .await;
    let snap = db
        .meeting_summary_run_snapshot(id, pipe, None)
        .await
        .unwrap();
    assert_eq!(snap.state(), MeetingSummaryState::Running);
    assert_eq!(snap.latest_execution.unwrap().id, recovery);

    // not_before hides runs that started earlier.
    let bound = screenpipe_db::ext::meetings_v2::parse_not_before(&ts(-5)).unwrap();
    assert_eq!(
        db.meeting_summary_run_snapshot(id, pipe, Some(&bound))
            .await
            .unwrap()
            .state(),
        MeetingSummaryState::Idle
    );

    sqlx::query("UPDATE pipe_executions SET status = 'completed' WHERE id = ?1")
        .bind(recovery)
        .execute(db.write_pool())
        .await
        .unwrap();
    assert_eq!(
        db.meeting_summary_run_snapshot(id, pipe, None)
            .await
            .unwrap()
            .state(),
        MeetingSummaryState::Ready
    );
}

#[tokio::test]
async fn summary_snapshot_reports_claimed_runs_as_pending() {
    let db = setup_test_db().await;
    let id = ended_meeting(&db, "Zoom", None).await;
    let pipe = "meeting-summary";

    // A claim whose execution exists does not count on its own.
    insert_claim(&db, pipe, &id.to_string(), &ts(-60)).await;
    insert_execution(&db, pipe, "completed", &id.to_string(), &ts(-59), None).await;
    assert_eq!(
        db.meeting_summary_run_snapshot(id, pipe, None)
            .await
            .unwrap()
            .state(),
        MeetingSummaryState::Ready
    );

    // A recovery claim without an execution yet: pending.
    insert_claim(&db, pipe, &format!("{id}:summary-recovery-1"), &ts(-1)).await;
    assert_eq!(
        db.meeting_summary_run_snapshot(id, pipe, None)
            .await
            .unwrap()
            .state(),
        MeetingSummaryState::Pending
    );

    // A stale claim that never produced a run is ignored.
    let other = ended_meeting(&db, "Zoom", None).await;
    insert_claim(&db, pipe, &other.to_string(), &ts(-3 * 3600)).await;
    assert_eq!(
        db.meeting_summary_run_snapshot(other, pipe, None)
            .await
            .unwrap()
            .state(),
        MeetingSummaryState::Idle
    );
}

// ── list q and transcript ────────────────────────────────────────────────

#[tokio::test]
async fn list_q_matches_meeting_app_and_treats_wildcards_literally() {
    let db = setup_test_db().await;
    let zoom = ended_meeting(&db, "zoom.us", None).await;
    let teams = ended_meeting(&db, "Microsoft Teams", None).await;
    db.update_meeting(teams, None, None, Some("100% sync"), None, None, None)
        .await
        .unwrap();

    let ids =
        |v: Vec<screenpipe_db::MeetingRecord>| v.into_iter().map(|m| m.id).collect::<Vec<_>>();
    assert_eq!(
        ids(db
            .list_meetings(None, None, Some("ZOOM"), 20, 0)
            .await
            .unwrap()),
        vec![zoom]
    );
    assert_eq!(
        ids(db
            .list_meetings(None, None, Some("teams"), 20, 0)
            .await
            .unwrap()),
        vec![teams]
    );
    assert_eq!(
        ids(db
            .list_meetings(None, None, Some("100%"), 20, 0)
            .await
            .unwrap()),
        vec![teams]
    );
    // `%` alone is a literal percent sign, not "match everything".
    assert_eq!(
        ids(db
            .list_meetings(None, None, Some("%"), 20, 0)
            .await
            .unwrap()),
        vec![teams]
    );
    assert!(db
        .list_meetings(None, None, Some("_"), 20, 0)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn transcript_rows_carry_their_offset_in_the_chunk() {
    let db = setup_test_db().await;
    let id = db
        .insert_meeting("manual", "manual", None, None)
        .await
        .unwrap();
    db.end_meeting(id, &ts(3600), None).await.unwrap();
    let at = Utc::now() + Duration::seconds(5);

    let chunk = db
        .insert_audio_chunk("System Audio (output)_chunk.mp4", Some(at))
        .await
        .unwrap();
    db.insert_audio_transcription(
        chunk,
        "the background row",
        0,
        "whisper",
        &AudioDevice {
            name: "System Audio".to_string(),
            device_type: DeviceType::Output,
        },
        None,
        Some(12.5),
        Some(15.0),
        Some(at),
    )
    .await
    .unwrap();
    db.insert_meeting_transcript_segment(
        id,
        "deepgram",
        Some("nova-3"),
        "deepgram:s:turn:0:0",
        "Mic",
        "input",
        None,
        "the live row",
        at + Duration::seconds(60),
    )
    .await
    .unwrap();

    let rows = db.list_meeting_transcript_segments(id).await.unwrap();
    let background = rows.iter().find(|r| r.source == "background").unwrap();
    assert_eq!(background.audio_start_time_secs, Some(12.5));
    let live = rows.iter().find(|r| r.source == "live").unwrap();
    assert_eq!(live.audio_start_time_secs, None);

    let json = serde_json::to_value(background).unwrap();
    assert_eq!(json["audioStartTimeSecs"], serde_json::json!(12.5));
    assert!(json.get("sessionSpeakerId").is_some());
}
