// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Stream-scoped live meeting speakers (db-014, audio-001), the guarded
//! per-chunk speaker backfill (audio-008), strict meeting-segment to global
//! speaker mapping (audio-M01) and live rows on the timeline (audio-M05).

use chrono::{DateTime, Duration, TimeZone, Utc};
use screenpipe_db::{DatabaseManager, NewMeetingTranscriptSegment, LEGACY_MEETING_STREAM_ID};

async fn setup_db() -> DatabaseManager {
    let db = DatabaseManager::new("sqlite::memory:", Default::default())
        .await
        .unwrap();
    screenpipe_db::schema_migrations::migrator()
        .run(db.write_pool())
        .await
        .expect("migrations");
    db
}

fn base() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 20, 10, 0, 0).unwrap() + Duration::milliseconds(250)
}

fn live_segment(
    stream_id: &str,
    item_id: &str,
    session_speaker_id: Option<&str>,
    speaker_name: Option<&str>,
    device_type: &str,
    transcript: &str,
    captured_at: DateTime<Utc>,
) -> NewMeetingTranscriptSegment {
    NewMeetingTranscriptSegment {
        provider: "deepgram-live".to_string(),
        model: Some("nova-3".to_string()),
        item_id: item_id.to_string(),
        device_name: device_name(device_type).to_string(),
        device_type: device_type.to_string(),
        stream_id: Some(stream_id.to_string()),
        session_speaker_id: session_speaker_id.map(str::to_string),
        speaker_name: speaker_name.map(str::to_string),
        transcript: transcript.to_string(),
        captured_at,
    }
}

fn device_name(device_type: &str) -> &'static str {
    if device_type == "input" {
        "Built-in Mic"
    } else {
        "System Audio"
    }
}

async fn meeting(db: &DatabaseManager) -> i64 {
    db.insert_meeting("zoom.us", "ui_scan", None, None)
        .await
        .unwrap()
}

async fn chunk(db: &DatabaseManager, device_type: &str, ts: DateTime<Utc>) -> i64 {
    let path = format!(
        "{} ({})_{}.mp4",
        device_name(device_type),
        device_type,
        ts.timestamp_millis()
    );
    db.insert_audio_chunk(&path, Some(ts)).await.unwrap()
}

/// Insert one `audio_transcriptions` row directly, so a test controls the
/// engine, speaker and timestamp exactly (the normal insert path dedups).
#[allow(clippy::too_many_arguments)]
async fn transcription_row(
    db: &DatabaseManager,
    chunk_id: i64,
    text: &str,
    engine: &str,
    device_type: &str,
    speaker_id: Option<i64>,
    ts: DateTime<Utc>,
) -> i64 {
    sqlx::query(
        "INSERT INTO audio_transcriptions \
         (audio_chunk_id, transcription, offset_index, timestamp, transcription_engine, \
          device, is_input_device, speaker_id, start_time, end_time, text_length) \
         VALUES (?1, ?2, 0, ?3, ?4, ?5, ?6, ?7, 0, 2, ?8)",
    )
    .bind(chunk_id)
    .bind(text)
    .bind(ts)
    .bind(engine)
    .bind(device_name(device_type))
    .bind(device_type == "input")
    .bind(speaker_id)
    .bind(text.len() as i64)
    .execute(db.write_pool())
    .await
    .unwrap()
    .last_insert_rowid()
}

async fn speakers_of_chunk(db: &DatabaseManager, chunk_id: i64) -> Vec<Option<i64>> {
    sqlx::query_scalar(
        "SELECT speaker_id FROM audio_transcriptions WHERE audio_chunk_id = ?1 ORDER BY id",
    )
    .bind(chunk_id)
    .fetch_all(&db.pool)
    .await
    .unwrap()
}

async fn segment_speaker(db: &DatabaseManager, segment_id: i64) -> Option<i64> {
    sqlx::query_scalar("SELECT speaker_id FROM meeting_transcript_segments WHERE id = ?1")
        .bind(segment_id)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

async fn diarization(db: &DatabaseManager, chunk_id: i64, segments: &[(&str, Option<i64>, bool)]) {
    let run_id = sqlx::query(
        "INSERT INTO diarization_runs (audio_chunk_id, mode, provider) VALUES (?1, 'background', 'test')",
    )
    .bind(chunk_id)
    .execute(db.write_pool())
    .await
    .unwrap()
    .last_insert_rowid();
    for (i, (label, speaker_id, overlap)) in segments.iter().enumerate() {
        sqlx::query(
            "INSERT INTO diarization_segments \
             (diarization_run_id, audio_chunk_id, provider_speaker_label, speaker_id, start_time, end_time, overlap) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(run_id)
        .bind(chunk_id)
        .bind(*label)
        .bind(*speaker_id)
        .bind(i as f64)
        .bind(i as f64 + 1.0)
        .bind(*overlap)
        .execute(db.write_pool())
        .await
        .unwrap();
    }
}

// ---------------------------------------------------------------------------
// Segment insert (db-014 / audio-001)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_insert_stores_stream_and_session_speaker() {
    let db = setup_db().await;
    let meeting_id = meeting(&db).await;
    let id = db
        .insert_live_meeting_transcript_segment(
            meeting_id,
            &live_segment(
                "stream-a",
                "deepgram:stream-a:turn:0:1500",
                Some("deepgram:stream-a:speaker:1"),
                Some("speaker 2"),
                "output",
                "  we ship friday  ",
                base(),
            ),
        )
        .await
        .unwrap();
    assert!(id > 0);

    let (stream_id, session, name, transcript): (String, Option<String>, Option<String>, String) =
        sqlx::query_as(
            "SELECT stream_id, session_speaker_id, speaker_name, transcript \
             FROM meeting_transcript_segments WHERE id = ?1",
        )
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(stream_id, "stream-a");
    assert_eq!(session.as_deref(), Some("deepgram:stream-a:speaker:1"));
    assert_eq!(name.as_deref(), Some("speaker 2"));
    assert_eq!(transcript, "we ship friday");

    // The unique key still drops a duplicate final of the same turn.
    let again = db
        .insert_live_meeting_transcript_segment(
            meeting_id,
            &live_segment(
                "stream-a",
                "deepgram:stream-a:turn:0:1500",
                Some("deepgram:stream-a:speaker:1"),
                Some("speaker 2"),
                "output",
                "we ship friday",
                base(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(again, 0);
}

#[tokio::test]
async fn unscoped_inserts_read_as_legacy() {
    let db = setup_db().await;
    let meeting_id = meeting(&db).await;

    // The unscoped convenience insert.
    let plain = db
        .insert_meeting_transcript_segment(
            meeting_id,
            "deepgram-live",
            None,
            "deepgram:0:0",
            "System Audio",
            "output",
            Some("speaker 1"),
            "plain insert",
            base(),
        )
        .await
        .unwrap();
    // A blank stream id (and session speaker) is treated like a missing one.
    let blank = live_segment(
        "  ",
        "blank",
        Some(" "),
        None,
        "output",
        "blank stream",
        base(),
    );
    let blank = db
        .insert_live_meeting_transcript_segment(meeting_id, &blank)
        .await
        .unwrap();
    // A row written by code that predates the columns (column default).
    let raw = sqlx::query(
        "INSERT INTO meeting_transcript_segments \
         (meeting_id, provider, item_id, device_name, device_type, transcript, captured_at) \
         VALUES (?1, 'deepgram-live', 'old', 'System Audio', 'output', 'old row', ?2)",
    )
    .bind(meeting_id)
    .bind(base().to_rfc3339())
    .execute(db.write_pool())
    .await
    .unwrap()
    .last_insert_rowid();

    for id in [plain, blank, raw] {
        let (stream_id, session): (String, Option<String>) = sqlx::query_as(
            "SELECT stream_id, session_speaker_id FROM meeting_transcript_segments WHERE id = ?1",
        )
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(stream_id, LEGACY_MEETING_STREAM_ID, "row {id}");
        assert_eq!(session, None, "row {id}");
    }
}

#[tokio::test]
async fn reconnected_streams_keep_both_turns_and_identities() {
    let db = setup_db().await;
    let meeting_id = meeting(&db).await;
    // Turn offsets restart at 0 after a reconnect; the stream id in the
    // item id keeps the second connection's first turn from being dropped.
    for stream in ["first", "second"] {
        let id = db
            .insert_live_meeting_transcript_segment(
                meeting_id,
                &live_segment(
                    stream,
                    &format!("deepgram:{stream}:turn:0:0"),
                    Some(&format!("deepgram:{stream}:speaker:0")),
                    Some("speaker 1"),
                    "output",
                    &format!("hello from {stream}"),
                    base(),
                ),
            )
            .await
            .unwrap();
        assert!(id > 0, "turn of stream {stream} must be stored");
    }

    let segments = db
        .list_meeting_transcript_segments(meeting_id)
        .await
        .unwrap();
    let mut sessions: Vec<Option<String>> = segments
        .iter()
        .filter(|s| s.source == "live")
        .map(|s| s.session_speaker_id.clone())
        .collect();
    sessions.sort();
    assert_eq!(
        sessions,
        vec![
            Some("deepgram:first:speaker:0".to_string()),
            Some("deepgram:second:speaker:0".to_string())
        ],
        "both rows read 'speaker 1' but are two stream-scoped identities"
    );

    let json = serde_json::to_value(&segments[0]).unwrap();
    assert!(
        json.get("sessionSpeakerId").is_some(),
        "transcript API exposes sessionSpeakerId: {json}"
    );
}

#[tokio::test]
async fn replacement_segments_default_to_legacy() {
    let db = setup_db().await;
    let meeting_id = meeting(&db).await;
    let segment = NewMeetingTranscriptSegment {
        provider: "retranscribe".to_string(),
        model: Some("whisper".to_string()),
        item_id: "meeting-retranscribe:output:1:1".to_string(),
        device_name: "System Audio".to_string(),
        device_type: "output".to_string(),
        stream_id: None,
        session_speaker_id: None,
        speaker_name: None,
        transcript: "replaced text".to_string(),
        captured_at: base(),
    };
    let (_, inserted) = db
        .replace_meeting_transcript_segments(meeting_id, &[segment])
        .await
        .unwrap();
    assert_eq!(inserted, 1);
    let stream_id: String = sqlx::query_scalar(
        "SELECT stream_id FROM meeting_transcript_segments WHERE meeting_id = ?1",
    )
    .bind(meeting_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(stream_id, LEGACY_MEETING_STREAM_ID);
}

// ---------------------------------------------------------------------------
// Guarded per-chunk backfill (audio-008)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn guarded_backfill_assigns_a_clean_chunk() {
    let db = setup_db().await;
    let alice = db.create_speaker_with_name("Alice").await.unwrap().id;
    let c = chunk(&db, "input", base()).await;
    transcription_row(&db, c, "first line", "whisper", "input", None, base()).await;
    transcription_row(&db, c, "second line", "whisper", "input", None, base()).await;
    // A diarization run that found one voice does not block it.
    diarization(
        &db,
        c,
        &[("spk0", None, false), ("spk0", Some(alice), false)],
    )
    .await;

    let written = db
        .assign_chunk_speaker_if_unambiguous(c, alice)
        .await
        .unwrap();
    assert_eq!(written, 2);
    assert_eq!(
        speakers_of_chunk(&db, c).await,
        vec![Some(alice), Some(alice)]
    );
}

#[tokio::test]
async fn guarded_backfill_skips_ambiguous_chunks() {
    let db = setup_db().await;
    let alice = db.create_speaker_with_name("Alice").await.unwrap().id;
    let bob = db.create_speaker_with_name("Bob").await.unwrap().id;

    // (a) another row of the chunk already names a different speaker.
    let labelled = chunk(&db, "output", base()).await;
    transcription_row(
        &db,
        labelled,
        "known",
        "whisper",
        "output",
        Some(bob),
        base(),
    )
    .await;
    transcription_row(&db, labelled, "unknown", "whisper", "output", None, base()).await;

    // (b) a diarization segment names a different speaker.
    let named = chunk(&db, "output", base() + Duration::seconds(30)).await;
    transcription_row(&db, named, "x", "whisper", "output", None, base()).await;
    diarization(&db, named, &[("spk0", Some(bob), false)]).await;

    // (b) a diarization segment is flagged as overlapping speech.
    let overlapping = chunk(&db, "output", base() + Duration::seconds(60)).await;
    transcription_row(&db, overlapping, "y", "whisper", "output", None, base()).await;
    diarization(&db, overlapping, &[("spk0", None, true)]).await;

    // (c) one diarization run heard two different labels.
    let two_voices = chunk(&db, "output", base() + Duration::seconds(90)).await;
    transcription_row(&db, two_voices, "z", "whisper", "output", None, base()).await;
    diarization(
        &db,
        two_voices,
        &[("spk0", None, false), ("spk1", None, false)],
    )
    .await;

    for chunk_id in [labelled, named, overlapping, two_voices] {
        let written = db
            .assign_chunk_speaker_if_unambiguous(chunk_id, alice)
            .await
            .unwrap();
        assert_eq!(written, 0, "chunk {chunk_id} is ambiguous");
    }
    assert_eq!(
        speakers_of_chunk(&db, labelled).await,
        vec![Some(bob), None]
    );
    for chunk_id in [named, overlapping, two_voices] {
        assert_eq!(speakers_of_chunk(&db, chunk_id).await, vec![None]);
    }

    // Two runs that each heard one (different) label are not ambiguous on
    // their own: labels of different runs are not comparable.
    let two_runs = chunk(&db, "output", base() + Duration::seconds(120)).await;
    transcription_row(&db, two_runs, "w", "whisper", "output", None, base()).await;
    diarization(&db, two_runs, &[("A", None, false)]).await;
    diarization(&db, two_runs, &[("B", None, false)]).await;
    assert_eq!(
        db.assign_chunk_speaker_if_unambiguous(two_runs, alice)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn guarded_backfill_checks_live_evidence() {
    let db = setup_db().await;
    let alice = db.create_speaker_with_name("Alice").await.unwrap().id;
    let bob = db.create_speaker_with_name("Bob").await.unwrap().id;
    let meeting_id = meeting(&db).await;
    let t1 = base();
    let t2 = base() + Duration::seconds(4);

    async fn live_pair(
        db: &DatabaseManager,
        meeting_id: i64,
        chunk_id: i64,
        stream: &str,
        label: Option<&str>,
        text: &str,
        ts: DateTime<Utc>,
    ) {
        db.insert_live_meeting_transcript_segment(
            meeting_id,
            &live_segment(
                stream,
                &format!("deepgram:{stream}:turn:{text}"),
                label,
                Some("speaker 1"),
                "output",
                text,
                ts,
            ),
        )
        .await
        .unwrap();
        transcription_row(db, chunk_id, text, "live", "output", None, ts).await;
    }

    // One stream-scoped identity across the chunk: assigned.
    let single = chunk(&db, "output", t1).await;
    live_pair(
        &db,
        meeting_id,
        single,
        "s1",
        Some("deepgram:s1:speaker:0"),
        "one a",
        t1,
    )
    .await;
    live_pair(
        &db,
        meeting_id,
        single,
        "s1",
        Some("deepgram:s1:speaker:0"),
        "one b",
        t2,
    )
    .await;
    assert_eq!(
        db.assign_chunk_speaker_if_unambiguous(single, alice)
            .await
            .unwrap(),
        2
    );

    // Two identities of one stream: skipped.
    let mixed = chunk(&db, "output", t1 + Duration::minutes(1)).await;
    let m1 = t1 + Duration::minutes(1);
    live_pair(
        &db,
        meeting_id,
        mixed,
        "s2",
        Some("deepgram:s2:speaker:0"),
        "mix a",
        m1,
    )
    .await;
    live_pair(
        &db,
        meeting_id,
        mixed,
        "s2",
        Some("deepgram:s2:speaker:1"),
        "mix b",
        m1 + Duration::seconds(3),
    )
    .await;

    // The same label in two streams is two people as far as we know: skipped.
    let reconnect = chunk(&db, "output", t1 + Duration::minutes(2)).await;
    let r1 = t1 + Duration::minutes(2);
    live_pair(&db, meeting_id, reconnect, "s3", None, "re a", r1).await;
    live_pair(
        &db,
        meeting_id,
        reconnect,
        "s4",
        None,
        "re b",
        r1 + Duration::seconds(3),
    )
    .await;

    // A live row that matches no segment: skipped.
    let orphan = chunk(&db, "output", t1 + Duration::minutes(3)).await;
    let o1 = t1 + Duration::minutes(3);
    live_pair(
        &db,
        meeting_id,
        orphan,
        "s5",
        Some("deepgram:s5:speaker:0"),
        "orph a",
        o1,
    )
    .await;
    transcription_row(&db, orphan, "no segment for me", "live", "output", None, o1).await;

    // Matched evidence already mapped to another global speaker: skipped.
    let conflicting = chunk(&db, "output", t1 + Duration::minutes(4)).await;
    let c1 = t1 + Duration::minutes(4);
    live_pair(
        &db,
        meeting_id,
        conflicting,
        "s6",
        Some("deepgram:s6:speaker:0"),
        "conf a",
        c1,
    )
    .await;
    sqlx::query(
        "UPDATE meeting_transcript_segments SET speaker_id = ?1 WHERE transcript = 'conf a'",
    )
    .bind(bob)
    .execute(db.write_pool())
    .await
    .unwrap();

    // Evidence with no label at all cannot vouch for one speaker: skipped.
    let unlabelled = chunk(&db, "output", t1 + Duration::minutes(5)).await;
    let u1 = t1 + Duration::minutes(5);
    db.insert_live_meeting_transcript_segment(
        meeting_id,
        &live_segment(
            "s7",
            "deepgram:s7:turn:u",
            None,
            None,
            "output",
            "unl a",
            u1,
        ),
    )
    .await
    .unwrap();
    transcription_row(&db, unlabelled, "unl a", "live", "output", None, u1).await;

    for chunk_id in [mixed, reconnect, orphan, conflicting, unlabelled] {
        assert_eq!(
            db.assign_chunk_speaker_if_unambiguous(chunk_id, alice)
                .await
                .unwrap(),
            0,
            "chunk {chunk_id} must stay unassigned"
        );
        assert!(speakers_of_chunk(&db, chunk_id)
            .await
            .iter()
            .all(Option::is_none));
    }
}

// ---------------------------------------------------------------------------
// Strict meeting-segment mapping (audio-M01)
// ---------------------------------------------------------------------------

async fn insert_live(
    db: &DatabaseManager,
    meeting_id: i64,
    item: &str,
    device_type: &str,
    text: &str,
    ts: DateTime<Utc>,
) -> i64 {
    db.insert_live_meeting_transcript_segment(
        meeting_id,
        &live_segment(
            "s1",
            item,
            Some("deepgram:s1:speaker:0"),
            Some("speaker 1"),
            device_type,
            text,
            ts,
        ),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn strict_mapping_uses_only_exact_live_mirrors() {
    let db = setup_db().await;
    let alice = db.create_speaker_with_name("Alice").await.unwrap().id;
    let meeting_id = meeting(&db).await;
    let c = chunk(&db, "output", base()).await;

    // Exact mirror with a resolved speaker: mapped.
    let exact = insert_live(&db, meeting_id, "a", "output", "exact text", base()).await;
    transcription_row(&db, c, "exact text", "live", "output", Some(alice), base()).await;

    // A background (non-live) row one second away no longer counts.
    let near = base() + Duration::seconds(10);
    let nearby = insert_live(&db, meeting_id, "b", "output", "nearby text", near).await;
    transcription_row(
        &db,
        c,
        "background line",
        "whisper",
        "output",
        Some(alice),
        near + Duration::seconds(1),
    )
    .await;

    // Same instant and text but the other direction: not a mirror.
    let flipped_at = base() + Duration::seconds(20);
    let flipped = insert_live(&db, meeting_id, "c", "input", "flipped text", flipped_at).await;
    transcription_row(
        &db,
        c,
        "flipped text",
        "live",
        "output",
        Some(alice),
        flipped_at,
    )
    .await;

    // Same instant and direction but different text: not a mirror.
    let edited_at = base() + Duration::seconds(30);
    let edited = insert_live(&db, meeting_id, "d", "output", "original", edited_at).await;
    transcription_row(
        &db,
        c,
        "original!",
        "live",
        "output",
        Some(alice),
        edited_at,
    )
    .await;

    let mapped = db
        .backfill_meeting_segment_speakers(base() - Duration::hours(1), 15.0)
        .await
        .unwrap();
    assert_eq!(mapped, 1);
    assert_eq!(segment_speaker(&db, exact).await, Some(alice));
    for id in [nearby, flipped, edited] {
        assert_eq!(segment_speaker(&db, id).await, None, "segment {id}");
    }

    // Idempotent: nothing left to map.
    assert_eq!(
        db.backfill_meeting_segment_speakers(base() - Duration::hours(1), 15.0)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn strict_mapping_skips_identical_text_twins() {
    let db = setup_db().await;
    let alice = db.create_speaker_with_name("Alice").await.unwrap().id;
    let meeting_id = meeting(&db).await;
    let c = chunk(&db, "output", base()).await;

    // Two segments with the same text at the same instant on the same device
    // (two turns that both said "yes"): the single mirror row cannot say which
    // one it belongs to, so neither is mapped.
    let twin_a = insert_live(&db, meeting_id, "twin-a", "output", "yes", base()).await;
    let twin_b = insert_live(&db, meeting_id, "twin-b", "output", "yes", base()).await;
    transcription_row(&db, c, "yes", "live", "output", Some(alice), base()).await;

    assert_eq!(
        db.backfill_meeting_segment_speakers(base() - Duration::hours(1), 15.0)
            .await
            .unwrap(),
        0
    );
    assert_eq!(segment_speaker(&db, twin_a).await, None);
    assert_eq!(segment_speaker(&db, twin_b).await, None);
}

#[tokio::test]
async fn strict_mapping_requires_a_single_distinct_speaker() {
    let db = setup_db().await;
    let alice = db.create_speaker_with_name("Alice").await.unwrap().id;
    let bob = db.create_speaker_with_name("Bob").await.unwrap().id;
    let meeting_id = meeting(&db).await;

    // The same segment mirrored onto two chunks that resolved to different
    // speakers: ambiguous, so not mapped.
    let split = insert_live(&db, meeting_id, "split", "output", "who said this", base()).await;
    let c1 = chunk(&db, "output", base()).await;
    let c2 = chunk(&db, "output", base() + Duration::seconds(1)).await;
    transcription_row(
        &db,
        c1,
        "who said this",
        "live",
        "output",
        Some(alice),
        base(),
    )
    .await;
    transcription_row(
        &db,
        c2,
        "who said this",
        "live",
        "output",
        Some(bob),
        base(),
    )
    .await;

    // Two mirrors that agree: mapped.
    let agreed_at = base() + Duration::seconds(5);
    let agreed = insert_live(&db, meeting_id, "agree", "output", "we agree", agreed_at).await;
    transcription_row(&db, c1, "we agree", "live", "output", Some(bob), agreed_at).await;
    transcription_row(&db, c2, "we agree", "live", "output", Some(bob), agreed_at).await;

    assert_eq!(
        db.backfill_meeting_segment_speakers(base() - Duration::hours(1), 15.0)
            .await
            .unwrap(),
        1
    );
    assert_eq!(segment_speaker(&db, split).await, None);
    assert_eq!(segment_speaker(&db, agreed).await, Some(bob));
}

// ---------------------------------------------------------------------------
// Timeline audio rows (audio-M05)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn timeline_live_rows_are_flagged_with_distinct_negative_ids() {
    let db = setup_db().await;
    let t = base();
    db.insert_video_chunk("v.mp4", "screen").await.unwrap();
    db.insert_frame(
        "screen",
        Some(t),
        None,
        Some("zoom.us"),
        Some("Zoom"),
        false,
        None,
    )
    .await
    .unwrap();

    let meeting_id = meeting(&db).await;
    let mut segment_ids = Vec::new();
    for (i, text) in ["first live line", "second live line"].iter().enumerate() {
        segment_ids.push(
            insert_live(
                &db,
                meeting_id,
                &format!("tl-{i}"),
                "output",
                text,
                t + Duration::seconds(i as i64 + 1),
            )
            .await,
        );
    }

    let chunks = db
        .find_video_chunks(t - Duration::minutes(1), t + Duration::minutes(1))
        .await
        .unwrap();
    let live: Vec<_> = chunks
        .frames
        .iter()
        .flat_map(|f| f.audio_entries.iter())
        .filter(|a| a.transcription.contains("live line"))
        .collect();
    assert_eq!(live.len(), 2);
    assert!(live.iter().all(|a| a.is_live));
    let mut ids: Vec<i64> = live.iter().map(|a| a.audio_chunk_id).collect();
    ids.sort();
    let mut expected: Vec<i64> = segment_ids.iter().map(|id| -id).collect();
    expected.sort();
    assert_eq!(ids, expected, "audio_chunk_id is the negated segment id");
    assert!(ids.iter().all(|id| *id < 0));

    // A segment mapped to a global speaker shows that speaker's name.
    let alice = db.create_speaker_with_name("Alice").await.unwrap().id;
    sqlx::query("UPDATE meeting_transcript_segments SET speaker_id = ?1 WHERE id = ?2")
        .bind(alice)
        .bind(segment_ids[0])
        .execute(db.write_pool())
        .await
        .unwrap();
    let chunks = db
        .find_video_chunks(t - Duration::minutes(1), t + Duration::minutes(1))
        .await
        .unwrap();
    let mapped = chunks
        .frames
        .iter()
        .flat_map(|f| f.audio_entries.iter())
        .find(|a| a.audio_chunk_id == -segment_ids[0])
        .expect("mapped live row");
    assert_eq!(mapped.speaker_name.as_deref(), Some("Alice"));
    let unmapped = chunks
        .frames
        .iter()
        .flat_map(|f| f.audio_entries.iter())
        .find(|a| a.audio_chunk_id == -segment_ids[1])
        .expect("unmapped live row");
    assert_eq!(unmapped.speaker_name.as_deref(), Some("speaker 1"));
}

#[tokio::test]
async fn timeline_marks_live_mirrors_but_not_background_rows() {
    let db = setup_db().await;
    let t = base();
    db.insert_video_chunk("v.mp4", "screen").await.unwrap();
    db.insert_frame(
        "screen",
        Some(t),
        None,
        Some("zoom.us"),
        Some("Zoom"),
        false,
        None,
    )
    .await
    .unwrap();
    let c = chunk(&db, "output", t).await;
    transcription_row(&db, c, "mirrored meeting line", "live", "output", None, t).await;
    transcription_row(&db, c, "background line", "whisper", "output", None, t).await;

    let chunks = db
        .find_video_chunks(t - Duration::minutes(1), t + Duration::minutes(1))
        .await
        .unwrap();
    let entries: Vec<_> = chunks
        .frames
        .iter()
        .flat_map(|f| f.audio_entries.iter())
        .collect();
    let mirror = entries
        .iter()
        .find(|a| a.transcription == "mirrored meeting line")
        .expect("mirror row on the timeline");
    let background = entries
        .iter()
        .find(|a| a.transcription == "background line")
        .expect("background row on the timeline");
    assert!(mirror.is_live);
    assert_eq!(mirror.audio_chunk_id, c);
    assert!(!background.is_live);
}
