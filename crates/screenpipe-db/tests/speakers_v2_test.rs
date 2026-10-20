// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Speakers v2 (W2-16): five-table merge integrity, guarded speaker deletion,
//! reassign scope and undo, paginated search, playable samples, the speaker
//! table self-heal and the stricter automatic self-naming query.

use chrono::{DateTime, Duration, Utc};
use screenpipe_db::{DatabaseManager, SpeakerReassignScope};

async fn setup_db() -> DatabaseManager {
    let db = DatabaseManager::new("sqlite::memory:", Default::default())
        .await
        .expect("open in-memory db");
    screenpipe_db::schema_migrations::migrator()
        .run(&db.pool)
        .await
        .expect("run migrations");
    db
}

/// A 512-d voice embedding. Voices with different `phase` values are
/// orthogonal (cosine distance 1); `jitter` nudges one component so two
/// embeddings of the same voice are near-identical but not equal.
fn voice(phase: usize, jitter: f32) -> Vec<f32> {
    let mut v: Vec<f32> = (0..512)
        .map(|i| if i % 4 == phase % 4 { 1.0 } else { 0.0 })
        .collect();
    v[phase % 4] += jitter;
    v
}

async fn new_speaker(db: &DatabaseManager, phase: usize, name: Option<&str>) -> i64 {
    let id = db.insert_speaker(&voice(phase, 0.0)).await.unwrap().id;
    if let Some(name) = name {
        db.update_speaker_name(id, name).await.unwrap();
    }
    id
}

/// Insert an audio chunk with one transcription row (written directly so the
/// cross-device dedup in `insert_audio_transcription` cannot drop it).
#[allow(clippy::too_many_arguments)]
async fn add_chunk(
    db: &DatabaseManager,
    path: &str,
    speaker_id: Option<i64>,
    text: &str,
    ts: DateTime<Utc>,
    is_input: bool,
    engine: &str,
    span: (f64, f64),
) -> (i64, i64) {
    let chunk_id = db.insert_audio_chunk(path, Some(ts)).await.unwrap();
    let row_id = add_row(db, chunk_id, speaker_id, text, ts, is_input, engine, span).await;
    (chunk_id, row_id)
}

#[allow(clippy::too_many_arguments)]
async fn add_row(
    db: &DatabaseManager,
    chunk_id: i64,
    speaker_id: Option<i64>,
    text: &str,
    ts: DateTime<Utc>,
    is_input: bool,
    engine: &str,
    span: (f64, f64),
) -> i64 {
    sqlx::query(
        "INSERT INTO audio_transcriptions \
         (audio_chunk_id, transcription, offset_index, timestamp, transcription_engine, \
          device, is_input_device, speaker_id, start_time, end_time) \
         VALUES (?1, ?2, 0, ?3, ?4, 'test-device', ?5, ?6, ?7, ?8)",
    )
    .bind(chunk_id)
    .bind(text)
    .bind(ts)
    .bind(engine)
    .bind(is_input)
    .bind(speaker_id)
    .bind(span.0)
    .bind(span.1)
    .execute(&db.pool)
    .await
    .unwrap()
    .last_insert_rowid()
}

async fn add_meeting_segment(
    db: &DatabaseManager,
    meeting_id: i64,
    item: &str,
    text: &str,
    captured_at: DateTime<Utc>,
    speaker_id: Option<i64>,
) -> i64 {
    sqlx::query(
        "INSERT INTO meeting_transcript_segments \
         (meeting_id, provider, item_id, device_name, device_type, transcript, captured_at, speaker_id) \
         VALUES (?1, 'test-live', ?2, 'test-device', 'input', ?3, ?4, ?5)",
    )
    .bind(meeting_id)
    .bind(item)
    .bind(text)
    .bind(captured_at.to_rfc3339())
    .bind(speaker_id)
    .execute(&db.pool)
    .await
    .unwrap()
    .last_insert_rowid()
}

async fn add_diarization_refs(db: &DatabaseManager, chunk_id: i64, speaker_id: i64) {
    let run_id = sqlx::query(
        "INSERT INTO diarization_runs (audio_chunk_id, mode, provider) VALUES (?1, 'background', 'test')",
    )
    .bind(chunk_id)
    .execute(&db.pool)
    .await
    .unwrap()
    .last_insert_rowid();
    sqlx::query(
        "INSERT INTO diarization_segments \
         (diarization_run_id, audio_chunk_id, provider_speaker_label, speaker_id, start_time, end_time) \
         VALUES (?1, ?2, 'spk_0', ?3, 0.0, 2.0)",
    )
    .bind(run_id)
    .bind(chunk_id)
    .bind(speaker_id)
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO speaker_identity_evidence (speaker_id, audio_chunk_id, source) VALUES (?1, ?2, 'test')",
    )
    .bind(speaker_id)
    .bind(chunk_id)
    .execute(&db.pool)
    .await
    .unwrap();
}

async fn refs(db: &DatabaseManager, table: &str, speaker_id: i64) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT COUNT(*) FROM {table} WHERE speaker_id = ?1"
    )))
    .bind(speaker_id)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

async fn speaker_exists(db: &DatabaseManager, id: i64) -> bool {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM speakers WHERE id = ?1)")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

async fn speaker_name(db: &DatabaseManager, id: i64) -> Option<String> {
    sqlx::query_scalar("SELECT name FROM speakers WHERE id = ?1")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

async fn row_speaker(db: &DatabaseManager, table: &str, row_id: i64) -> Option<i64> {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT speaker_id FROM {table} WHERE id = ?1"
    )))
    .bind(row_id)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

fn hours_ago(h: i64) -> DateTime<Utc> {
    Utc::now() - Duration::hours(h)
}

// ── merge: all five tables ──────────────────────────────────────────────────

#[tokio::test]
async fn merge_repoints_all_five_tables_and_deletes_the_merged_row() {
    let db = setup_db().await;
    let keep = new_speaker(&db, 0, Some("Keep")).await;
    let merged = new_speaker(&db, 1, None).await;

    let ts = hours_ago(1);
    let (chunk, _) = add_chunk(
        &db,
        "/data/merge.mp4",
        Some(merged),
        "merge me please",
        ts,
        true,
        "whisper",
        (0.0, 3.0),
    )
    .await;
    let meeting = db.insert_meeting("test", "manual", None, None).await.unwrap();
    add_meeting_segment(&db, meeting, "m1", "merge me please", ts, Some(merged)).await;
    add_diarization_refs(&db, chunk, merged).await;

    db.merge_speakers(keep, merged).await.unwrap();

    for table in DatabaseManager::SPEAKER_REFERENCE_TABLES {
        assert_eq!(refs(&db, table, merged).await, 0, "{table} still points at merged");
    }
    assert_eq!(refs(&db, "audio_transcriptions", keep).await, 1);
    assert_eq!(refs(&db, "meeting_transcript_segments", keep).await, 1);
    assert_eq!(refs(&db, "speaker_embeddings", keep).await, 2);
    assert_eq!(refs(&db, "diarization_segments", keep).await, 1);
    assert_eq!(refs(&db, "speaker_identity_evidence", keep).await, 1);
    assert!(!speaker_exists(&db, merged).await);
    assert_eq!(speaker_name(&db, keep).await.as_deref(), Some("Keep"));
}

#[tokio::test]
async fn merge_into_itself_or_a_missing_speaker_changes_nothing() {
    let db = setup_db().await;
    let a = new_speaker(&db, 0, Some("Ada")).await;
    let b = new_speaker(&db, 1, None).await;

    let kept = db.merge_speakers(a, a).await.unwrap();
    assert_eq!(kept.id, a);
    assert!(speaker_exists(&db, a).await);

    assert!(db.merge_speakers(9_999, b).await.is_err());
    assert!(speaker_exists(&db, b).await);
    assert_eq!(refs(&db, "speaker_embeddings", b).await, 1);
}

// ── guarded deletion: a still-referenced speaker is kept ────────────────────

#[tokio::test]
async fn reassign_keeps_previous_speaker_that_other_chunks_reference() {
    let db = setup_db().await;
    let voice_x = new_speaker(&db, 0, None).await; // one embedding
    let (chunk1, _) = add_chunk(
        &db, "/data/x1.mp4", Some(voice_x), "first chunk words", hours_ago(2), true, "whisper", (0.0, 3.0),
    )
    .await;
    let (chunk2, _) = add_chunk(
        &db, "/data/x2.mp4", Some(voice_x), "second chunk words", hours_ago(1), true, "whisper", (0.0, 3.0),
    )
    .await;

    let outcome = db
        .reassign_speaker_scoped(chunk1, "Bob", false, SpeakerReassignScope::Chunk)
        .await
        .unwrap();

    // The only embedding moved to Bob, but chunk2 still belongs to voice_x:
    // the old code deleted voice_x here and left chunk2 pointing at nothing.
    assert_eq!(outcome.embeddings_moved, 1);
    assert_eq!(refs(&db, "speaker_embeddings", voice_x).await, 0);
    assert!(speaker_exists(&db, voice_x).await, "referenced speaker must be kept");
    assert_eq!(refs(&db, "audio_transcriptions", voice_x).await, 1);
    let chunk2_speaker: Option<i64> =
        sqlx::query_scalar("SELECT speaker_id FROM audio_transcriptions WHERE audio_chunk_id = ?1")
            .bind(chunk2)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(chunk2_speaker, Some(voice_x));
}

#[tokio::test]
async fn reassign_keeps_previous_speaker_referenced_only_by_a_meeting_segment() {
    let db = setup_db().await;
    let voice_x = new_speaker(&db, 0, None).await;
    let (chunk, _) = add_chunk(
        &db, "/data/only.mp4", Some(voice_x), "the only chunk", hours_ago(2), true, "whisper", (0.0, 3.0),
    )
    .await;
    // A live segment with different text, so it does not follow the chunk.
    let meeting = db.insert_meeting("test", "manual", None, None).await.unwrap();
    add_meeting_segment(&db, meeting, "m1", "unrelated words", hours_ago(2), Some(voice_x)).await;

    db.reassign_speaker_scoped(chunk, "Bob", false, SpeakerReassignScope::Chunk)
        .await
        .unwrap();

    assert!(speaker_exists(&db, voice_x).await);
    assert_eq!(refs(&db, "meeting_transcript_segments", voice_x).await, 1);
}

#[tokio::test]
async fn reassign_deletes_previous_speaker_once_unreferenced() {
    let db = setup_db().await;
    let voice_x = new_speaker(&db, 0, None).await;
    let (chunk, _) = add_chunk(
        &db, "/data/solo.mp4", Some(voice_x), "solo chunk words", hours_ago(1), true, "whisper", (0.0, 3.0),
    )
    .await;

    db.reassign_speaker_scoped(chunk, "Bob", false, SpeakerReassignScope::Chunk)
        .await
        .unwrap();
    assert!(!speaker_exists(&db, voice_x).await);
}

#[tokio::test]
async fn delete_speaker_leaves_no_orphans_in_the_other_tables() {
    let db = setup_db().await;
    let doomed = new_speaker(&db, 0, Some("Gone")).await;
    let survivor = new_speaker(&db, 1, None).await;
    let ts = hours_ago(1);
    // The chunk survives the delete because the survivor still has a row in it.
    let (chunk, _) = add_chunk(
        &db, "/data/shared.mp4", Some(doomed), "doomed words here", ts, true, "whisper", (0.0, 3.0),
    )
    .await;
    add_row(&db, chunk, Some(survivor), "survivor words here", ts, true, "whisper", (3.0, 6.0)).await;
    let meeting = db.insert_meeting("test", "manual", None, None).await.unwrap();
    let segment = add_meeting_segment(&db, meeting, "m1", "doomed words here", ts, Some(doomed)).await;
    add_diarization_refs(&db, chunk, doomed).await;

    db.delete_speaker(doomed).await.unwrap();

    assert!(!speaker_exists(&db, doomed).await);
    for table in DatabaseManager::SPEAKER_REFERENCE_TABLES {
        assert_eq!(refs(&db, table, doomed).await, 0, "{table} kept an orphan id");
    }
    // Meeting and diarization rows stay, unlinked.
    assert_eq!(row_speaker(&db, "meeting_transcript_segments", segment).await, None);
    let diarization_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM diarization_segments")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(diarization_rows, 1);
}

// ── reassign scope=auto ─────────────────────────────────────────────────────

#[tokio::test]
async fn scope_auto_renames_an_unnamed_voice_in_place_and_undo_restores_it() {
    let db = setup_db().await;
    let voice_u = new_speaker(&db, 0, None).await;
    sqlx::query("UPDATE speakers SET hallucination = 1 WHERE id = ?1")
        .bind(voice_u)
        .execute(&db.pool)
        .await
        .unwrap();
    // A look-alike voice that propagation would absorb on the chunk path.
    let look_alike = db.insert_speaker(&voice(0, 0.01)).await.unwrap().id;

    let ts = hours_ago(1);
    let (chunk1, _) = add_chunk(
        &db, "/data/u1.mp4", Some(voice_u), "hello from the live call", ts, true, "live", (0.0, 3.0),
    )
    .await;
    add_chunk(
        &db, "/data/u2.mp4", Some(voice_u), "another unrelated sentence", hours_ago(2), true, "whisper", (0.0, 3.0),
    )
    .await;

    let meeting = db.insert_meeting("test", "manual", None, None).await.unwrap();
    let mirrored = add_meeting_segment(
        &db, meeting, "m1", "hello from the live call", ts + Duration::milliseconds(800), None,
    )
    .await;
    let too_late = add_meeting_segment(
        &db, meeting, "m2", "hello from the live call", ts + Duration::seconds(5), None,
    )
    .await;

    let outcome = db
        .reassign_speaker_scoped(chunk1, "  Grace  ", true, SpeakerReassignScope::Auto)
        .await
        .unwrap();

    assert!(outcome.renamed_whole_speaker);
    assert_eq!(outcome.new_speaker_id, voice_u);
    assert_eq!(outcome.new_speaker_name, "Grace");
    assert_eq!(outcome.previous_speaker_name.as_deref(), Some(""));
    assert_eq!(outcome.transcriptions_updated, 2);
    assert_eq!(outcome.embeddings_moved, 0);
    assert!(outcome.old_assignments.is_empty());
    assert_eq!(outcome.old_segment_assignments, vec![(mirrored, 0)]);

    assert_eq!(speaker_name(&db, voice_u).await.as_deref(), Some("Grace"));
    let hallucination: bool = sqlx::query_scalar("SELECT hallucination FROM speakers WHERE id = ?1")
        .bind(voice_u)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert!(!hallucination, "rename clears the hallucination flag");
    assert_eq!(row_speaker(&db, "meeting_transcript_segments", mirrored).await, Some(voice_u));
    assert_eq!(row_speaker(&db, "meeting_transcript_segments", too_late).await, None);
    // No propagation on the rename path.
    assert!(speaker_exists(&db, look_alike).await);
    assert_eq!(refs(&db, "speaker_embeddings", look_alike).await, 1);

    // Undo the way the UI does: rename back, then restore the segments.
    db.update_speaker_name(voice_u, outcome.previous_speaker_name.as_deref().unwrap())
        .await
        .unwrap();
    let restored = db
        .undo_speaker_reassign_with_segments(
            &outcome.old_assignments,
            &outcome.old_segment_assignments,
        )
        .await
        .unwrap();
    assert_eq!(restored, 1);
    assert_eq!(speaker_name(&db, voice_u).await.as_deref(), Some(""));
    assert_eq!(row_speaker(&db, "meeting_transcript_segments", mirrored).await, None);
}

#[tokio::test]
async fn scope_auto_uses_an_existing_named_speaker_case_insensitively() {
    let db = setup_db().await;
    let alice = new_speaker(&db, 0, Some("Alice")).await;
    let voice_u = new_speaker(&db, 1, None).await;
    let (chunk, _) = add_chunk(
        &db, "/data/a.mp4", Some(voice_u), "alice talking here", hours_ago(1), true, "whisper", (0.0, 3.0),
    )
    .await;

    let outcome = db
        .reassign_speaker_scoped(chunk, " alice ", false, SpeakerReassignScope::Auto)
        .await
        .unwrap();

    assert!(!outcome.renamed_whole_speaker);
    assert_eq!(outcome.previous_speaker_name, None);
    assert_eq!(outcome.new_speaker_id, alice);
    assert_eq!(outcome.new_speaker_name, "Alice");
    assert_eq!(outcome.transcriptions_updated, 1);
    assert_eq!(outcome.old_assignments.len(), 1);
    assert_eq!(outcome.old_assignments[0].1, voice_u);
    assert_eq!(refs(&db, "audio_transcriptions", alice).await, 1);
}

#[tokio::test]
async fn scope_auto_on_a_named_speaker_moves_only_the_chunk() {
    let db = setup_db().await;
    let bob = new_speaker(&db, 0, Some("Bob")).await;
    let (chunk1, _) = add_chunk(
        &db, "/data/b1.mp4", Some(bob), "bob first words", hours_ago(2), true, "whisper", (0.0, 3.0),
    )
    .await;
    add_chunk(
        &db, "/data/b2.mp4", Some(bob), "bob second words", hours_ago(1), true, "whisper", (0.0, 3.0),
    )
    .await;

    let outcome = db
        .reassign_speaker_scoped(chunk1, "Carol", false, SpeakerReassignScope::Auto)
        .await
        .unwrap();

    assert!(!outcome.renamed_whole_speaker);
    assert_ne!(outcome.new_speaker_id, bob);
    assert_eq!(speaker_name(&db, bob).await.as_deref(), Some("Bob"));
    assert_eq!(refs(&db, "audio_transcriptions", bob).await, 1);
    assert_eq!(refs(&db, "audio_transcriptions", outcome.new_speaker_id).await, 1);
}

#[tokio::test]
async fn scope_chunk_undo_restores_rows_and_recreates_a_deleted_speaker() {
    let db = setup_db().await;
    let voice_x = new_speaker(&db, 0, None).await;
    let ts = hours_ago(1);
    let (chunk, row) = add_chunk(
        &db, "/data/c.mp4", Some(voice_x), "chunk scope words", ts, true, "live", (0.0, 3.0),
    )
    .await;
    let meeting = db.insert_meeting("test", "manual", None, None).await.unwrap();
    let segment = add_meeting_segment(&db, meeting, "m1", "chunk scope words", ts, Some(voice_x)).await;

    let outcome = db
        .reassign_speaker_scoped(chunk, "Dana", true, SpeakerReassignScope::Chunk)
        .await
        .unwrap();
    assert_eq!(outcome.old_assignments, vec![(row, voice_x)]);
    assert_eq!(outcome.old_segment_assignments, vec![(segment, voice_x)]);
    // Everything that pointed at voice_x moved, so the row is gone.
    assert!(!speaker_exists(&db, voice_x).await);

    let restored = db
        .undo_speaker_reassign_with_segments(
            &outcome.old_assignments,
            &outcome.old_segment_assignments,
        )
        .await
        .unwrap();
    assert_eq!(restored, 2);
    assert!(speaker_exists(&db, voice_x).await, "undo recreates the speaker id");
    assert_eq!(speaker_name(&db, voice_x).await.as_deref(), Some(""));
    assert_eq!(row_speaker(&db, "audio_transcriptions", row).await, Some(voice_x));
    assert_eq!(row_speaker(&db, "meeting_transcript_segments", segment).await, Some(voice_x));
}

#[tokio::test]
async fn propagation_absorbs_similar_voices_across_all_five_tables() {
    let db = setup_db().await;
    let source = new_speaker(&db, 0, None).await;
    let similar = db.insert_speaker(&voice(0, 0.02)).await.unwrap().id;
    let different = new_speaker(&db, 2, None).await;

    let ts = hours_ago(1);
    let (chunk, _) = add_chunk(
        &db, "/data/p1.mp4", Some(source), "propagation source words", ts, true, "whisper", (0.0, 3.0),
    )
    .await;
    let (similar_chunk, similar_row) = add_chunk(
        &db, "/data/p2.mp4", Some(similar), "similar voice words", hours_ago(2), true, "whisper", (0.0, 3.0),
    )
    .await;
    let meeting = db.insert_meeting("test", "manual", None, None).await.unwrap();
    let similar_segment =
        add_meeting_segment(&db, meeting, "m1", "said elsewhere", hours_ago(2), Some(similar)).await;
    add_diarization_refs(&db, similar_chunk, similar).await;

    let outcome = db
        .reassign_speaker_scoped(chunk, "Eve", true, SpeakerReassignScope::Chunk)
        .await
        .unwrap();
    let eve = outcome.new_speaker_id;

    assert!(!speaker_exists(&db, similar).await);
    for table in DatabaseManager::SPEAKER_REFERENCE_TABLES {
        assert_eq!(refs(&db, table, similar).await, 0, "{table} still points at absorbed");
    }
    assert_eq!(refs(&db, "diarization_segments", eve).await, 1);
    assert_eq!(refs(&db, "speaker_identity_evidence", eve).await, 1);
    assert_eq!(outcome.embeddings_moved, 2);
    assert!(outcome.old_assignments.contains(&(similar_row, similar)));
    assert!(outcome.old_segment_assignments.contains(&(similar_segment, similar)));
    // An unrelated voice is untouched.
    assert!(speaker_exists(&db, different).await);
    assert_eq!(refs(&db, "speaker_embeddings", different).await, 1);
}

#[tokio::test]
async fn reassign_rejects_an_empty_name_and_an_unknown_chunk() {
    let db = setup_db().await;
    let voice_x = new_speaker(&db, 0, None).await;
    let (chunk, _) = add_chunk(
        &db, "/data/e.mp4", Some(voice_x), "empty name words", hours_ago(1), true, "whisper", (0.0, 3.0),
    )
    .await;

    let empty = db
        .reassign_speaker_scoped(chunk, "   ", true, SpeakerReassignScope::Auto)
        .await
        .unwrap_err();
    assert!(matches!(empty, sqlx::Error::InvalidArgument(_)));
    assert!(matches!(
        db.reassign_speaker_scoped(9_999, "Name", true, SpeakerReassignScope::Auto)
            .await
            .unwrap_err(),
        sqlx::Error::RowNotFound
    ));
}

#[tokio::test]
async fn reassign_handles_a_chunk_without_a_speaker() {
    let db = setup_db().await;
    let (chunk, row) = add_chunk(
        &db, "/data/none.mp4", None, "nobody assigned yet", hours_ago(1), true, "whisper", (0.0, 3.0),
    )
    .await;

    let outcome = db
        .reassign_speaker_scoped(chunk, "Finn", true, SpeakerReassignScope::Auto)
        .await
        .unwrap();
    assert!(!outcome.renamed_whole_speaker);
    assert_eq!(outcome.old_assignments, vec![(row, 0)]);
    assert_eq!(row_speaker(&db, "audio_transcriptions", row).await, Some(outcome.new_speaker_id));

    db.undo_speaker_reassign(&outcome.old_assignments).await.unwrap();
    assert_eq!(row_speaker(&db, "audio_transcriptions", row).await, None);
    assert!(!speaker_exists(&db, 0).await, "undo must not invent speaker 0");
}

// ── search paging and samples ───────────────────────────────────────────────

#[tokio::test]
async fn search_pages_names_case_insensitively_and_matches_prefixes_literally() {
    let db = setup_db().await;
    for (phase, name) in ["Andrew", "albert", "anna", "Bob", "a_b"].iter().enumerate() {
        new_speaker(&db, phase, Some(name)).await;
    }
    let duplicate = new_speaker(&db, 1, Some("anna")).await;
    let ghost = new_speaker(&db, 2, Some("amber")).await;
    db.mark_speaker_as_hallucination(ghost).await.unwrap();

    let names = |speakers: Vec<screenpipe_db::Speaker>| {
        speakers.into_iter().map(|s| s.name).collect::<Vec<_>>()
    };

    let first = db.search_speakers_paged("a", Some(2), 0, false).await.unwrap();
    assert!(first.iter().all(|s| s.metadata == "{}"));
    assert_eq!(names(first), vec!["a_b", "albert"]);
    let second = db.search_speakers_paged("A", Some(2), 2, false).await.unwrap();
    let anna_id = second.iter().find(|s| s.name == "anna").map(|s| s.id);
    assert_eq!(names(second), vec!["Andrew", "anna"]);
    assert!(anna_id.unwrap() < duplicate, "duplicate names collapse to the lowest id");

    assert_eq!(
        names(db.search_speakers_paged("a_", None, 0, false).await.unwrap()),
        vec!["a_b"]
    );
    assert_eq!(
        names(db.search_speakers_paged("a%", None, 0, true).await.unwrap()),
        Vec::<String>::new()
    );
    assert_eq!(db.search_speakers_paged("", Some(5), 0, false).await.unwrap().len(), 5);
    assert_eq!(db.search_speakers("").await.unwrap().len(), 5);
    assert_eq!(
        names(db.search_speakers_paged("b", Some(5), 0, true).await.unwrap()),
        vec!["Bob"]
    );
}

#[tokio::test]
async fn samples_carry_the_chunk_id_and_skip_unplayable_paths() {
    let db = setup_db().await;
    let sam = new_speaker(&db, 0, Some("Sam")).await;
    let unnamed = new_speaker(&db, 1, None).await;

    let (playable, _) = add_chunk(
        &db, "/data/sam-ok.mp4", Some(sam), "sam playable words", hours_ago(5), true, "whisper", (1.0, 4.0),
    )
    .await;
    for (i, path) in ["", ".", "cloud://bucket/sam.mp4"].iter().enumerate() {
        add_chunk(
            &db, path, Some(sam), &format!("sam hidden words {i}"), hours_ago(1), true, "whisper", (0.0, 2.0),
        )
        .await;
    }
    let (unnamed_playable, _) = add_chunk(
        &db, "/data/u-ok.mp4", Some(unnamed), "unnamed playable words", hours_ago(5), true, "whisper", (0.0, 2.0),
    )
    .await;
    add_chunk(
        &db, " . ", Some(unnamed), "unnamed hidden words", hours_ago(1), true, "whisper", (0.0, 2.0),
    )
    .await;

    let samples_of = |metadata: &str| -> Vec<serde_json::Value> {
        let parsed: serde_json::Value = serde_json::from_str(metadata).unwrap();
        parsed["audio_samples"].as_array().cloned().unwrap_or_default()
    };

    let found = db.search_speakers_paged("Sam", None, 0, true).await.unwrap();
    assert_eq!(found.len(), 1);
    let samples = samples_of(&found[0].metadata);
    assert_eq!(samples.len(), 1, "only the playable chunk is a sample: {samples:?}");
    assert_eq!(samples[0]["audio_chunk_id"], serde_json::json!(playable));
    assert_eq!(samples[0]["path"], "/data/sam-ok.mp4");
    assert_eq!(samples[0]["start_time"], serde_json::json!(1.0));

    let unnamed_list = db.get_unnamed_speakers(10, 0, None).await.unwrap();
    let entry = unnamed_list.iter().find(|s| s.id == unnamed).expect("unnamed listed");
    let samples = samples_of(&entry.metadata);
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0]["audio_chunk_id"], serde_json::json!(unnamed_playable));

    assert_eq!(
        db.get_audio_chunk_file_path(playable).await.unwrap().as_deref(),
        Some("/data/sam-ok.mp4")
    );
    assert_eq!(db.get_audio_chunk_file_path(9_999).await.unwrap(), None);
}

#[test]
fn like_prefix_pattern_escapes_wildcards() {
    assert_eq!(DatabaseManager::like_prefix_pattern("ab"), "ab%");
    assert_eq!(DatabaseManager::like_prefix_pattern("a_b%"), "a\\_b\\%%");
    assert_eq!(DatabaseManager::like_prefix_pattern("c\\d"), "c\\\\d%");
    assert_eq!(DatabaseManager::like_prefix_pattern(""), "%");
}

// ── startup self-heal ───────────────────────────────────────────────────────

#[tokio::test]
async fn self_heal_recreates_missing_speaker_tables() {
    let db = setup_db().await;
    sqlx::query("DROP TABLE speaker_embeddings")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE speakers").execute(&db.pool).await.unwrap();

    DatabaseManager::ensure_speaker_tables(&db.pool).await.unwrap();

    let columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('speakers') ORDER BY cid")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(
        columns,
        vec!["id", "name", "metadata", "hallucination", "centroid", "embedding_count"]
    );
    for index in ["idx_speakers_hallucination_name", "idx_speaker_embeddings_speaker_id"] {
        let present: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1)",
        )
        .bind(index)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert!(present, "{index} recreated");
    }

    // The healed tables work end to end, and a second pass is a no-op.
    let id = new_speaker(&db, 0, Some("Healed")).await;
    assert_eq!(refs(&db, "speaker_embeddings", id).await, 1);
    DatabaseManager::ensure_speaker_tables(&db.pool).await.unwrap();
    assert_eq!(speaker_name(&db, id).await.as_deref(), Some("Healed"));
}

// ── automatic self-naming (audio-009) ───────────────────────────────────────

async fn add_mic_chunks(db: &DatabaseManager, speaker: i64, count: usize, tag: &str) {
    for i in 0..count {
        add_chunk(
            db,
            &format!("/data/mic-{tag}-{i}.mp4"),
            Some(speaker),
            &format!("microphone words {tag} {i}"),
            hours_ago(1 + i as i64),
            true,
            "whisper",
            (0.0, 3.0),
        )
        .await;
    }
}

async fn dominant(db: &DatabaseManager) -> Option<i64> {
    db.get_dominant_unnamed_input_speaker(3, hours_ago(24))
        .await
        .unwrap()
}

#[tokio::test]
async fn self_naming_finds_the_lone_unnamed_microphone_voice() {
    let db = setup_db().await;
    let me = new_speaker(&db, 0, None).await;
    add_mic_chunks(&db, me, 3, "me").await;
    assert_eq!(dominant(&db).await, Some(me));
}

#[tokio::test]
async fn self_naming_needs_enough_long_non_live_chunks() {
    let db = setup_db().await;
    let me = new_speaker(&db, 0, None).await;
    add_mic_chunks(&db, me, 2, "me").await;
    assert_eq!(dominant(&db).await, None, "two chunks are not enough");

    // Short and live-engine rows do not count as evidence.
    add_chunk(&db, "/data/short.mp4", Some(me), "short words", hours_ago(3), true, "whisper", (0.0, 1.5)).await;
    add_chunk(&db, "/data/live.mp4", Some(me), "live words", hours_ago(3), true, "live", (0.0, 5.0)).await;
    assert_eq!(dominant(&db).await, None);

    // Neither do rows older than the window.
    add_chunk(&db, "/data/old.mp4", Some(me), "old words", hours_ago(30), true, "whisper", (0.0, 5.0)).await;
    assert_eq!(dominant(&db).await, None);

    add_chunk(&db, "/data/third.mp4", Some(me), "third words", hours_ago(4), true, "whisper", (0.0, 5.0)).await;
    assert_eq!(dominant(&db).await, Some(me));
}

#[tokio::test]
async fn self_naming_refuses_a_second_microphone_voice_even_if_named() {
    let db = setup_db().await;
    let me = new_speaker(&db, 0, None).await;
    add_mic_chunks(&db, me, 3, "me").await;
    let colleague = new_speaker(&db, 1, Some("Colleague")).await;
    add_chunk(&db, "/data/col.mp4", Some(colleague), "colleague words", hours_ago(2), true, "whisper", (0.0, 3.0)).await;
    assert_eq!(dominant(&db).await, None);
}

#[tokio::test]
async fn self_naming_refuses_a_voice_heard_on_output_or_already_named() {
    let db = setup_db().await;
    let me = new_speaker(&db, 0, None).await;
    add_mic_chunks(&db, me, 3, "me").await;
    add_chunk(&db, "/data/out.mp4", Some(me), "played back words", hours_ago(40), false, "whisper", (0.0, 3.0)).await;
    assert_eq!(dominant(&db).await, None, "a voice from the speakers is not the user");

    let db = setup_db().await;
    let me = new_speaker(&db, 0, Some("Already Named")).await;
    add_mic_chunks(&db, me, 3, "me").await;
    assert_eq!(dominant(&db).await, None);
}

#[tokio::test]
async fn self_naming_ignores_rows_inside_meetings() {
    let db = setup_db().await;
    let me = new_speaker(&db, 0, None).await;
    add_mic_chunks(&db, me, 3, "me").await;
    // A meeting covering the most recent chunk (one hour ago).
    let start = (hours_ago(1) - Duration::minutes(5)).format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let end = (hours_ago(1) + Duration::minutes(5)).format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    sqlx::query(
        "INSERT INTO meetings (meeting_start, meeting_end, meeting_app, detection_source) \
         VALUES (?1, ?2, 'test', 'manual')",
    )
    .bind(start)
    .bind(end)
    .execute(&db.pool)
    .await
    .unwrap();
    assert_eq!(dominant(&db).await, None);

    add_chunk(&db, "/data/after.mp4", Some(me), "outside meeting words", hours_ago(6), true, "whisper", (0.0, 3.0)).await;
    assert_eq!(dominant(&db).await, Some(me));
}
