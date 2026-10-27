// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! The `meeting_ended` event, shared by everything that ends a meeting (the
//! detectors and the meeting routes).
//!
//! Payload: `meeting_id`, `meeting_end`, `duration_secs` and
//! `transcript_segments` (live segments plus background transcriptions inside
//! the meeting window, as `GET /meetings/:id/transcript` returns them), and
//! `end_reason` when the end has one. Subscribers that gate on short or silent
//! meetings (the summary pipe trigger) read the last two fields. A field that
//! could not be read is `null`, never a guess: the event itself always goes
//! out, because the audio capture gate and live notes wait for it.
//!
//! The emitters await the payload before they send the event (so a room
//! change still announces the old meeting's end before the new meeting's
//! start), which puts the segment count on the stop route's and the
//! detector's path. [`count_meeting_transcript_segments`] therefore counts in
//! SQL and narrows the background transcriptions by an indexed timestamp
//! range before the exact window test, instead of loading the transcript.

use chrono::{DateTime, NaiveDateTime, Utc};
use screenpipe_db::DatabaseManager;
use serde_json::{json, Value};
use tracing::{debug, warn};

/// Event name on the engine bus.
pub const MEETING_ENDED_EVENT: &str = "meeting_ended";

/// Build the payload from what is known about the meeting.
///
/// `duration_secs` is whole seconds between `meeting_start` and
/// `meeting_end` (never negative), or `null` when either is missing or does
/// not parse.
pub fn meeting_ended_payload(
    meeting_id: i64,
    meeting_start: Option<&str>,
    meeting_end: Option<&str>,
    transcript_segments: Option<usize>,
    end_reason: Option<&str>,
) -> Value {
    let duration_secs = match (
        meeting_start.and_then(parse_ts),
        meeting_end.and_then(parse_ts),
    ) {
        (Some(start), Some(end)) => Some((end - start).num_seconds().max(0)),
        _ => None,
    };
    let mut payload = json!({
        "meeting_id": meeting_id,
        "meeting_end": meeting_end,
        "duration_secs": duration_secs,
        "transcript_segments": transcript_segments,
    });
    if let Some(reason) = end_reason.map(str::trim).filter(|r| !r.is_empty()) {
        payload["end_reason"] = json!(reason);
    }
    payload
}

fn parse_ts(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Read the ended meeting and count its transcript, then build the payload.
/// Call after the end was written.
pub async fn load_meeting_ended_payload(
    db: &DatabaseManager,
    meeting_id: i64,
    end_reason: Option<&str>,
) -> Value {
    let meeting = match db.get_meeting_by_id(meeting_id).await {
        Ok(meeting) => Some(meeting),
        Err(e) => {
            warn!(
                "meeting_ended: could not read meeting {} for the event payload: {}",
                meeting_id, e
            );
            None
        }
    };
    let segments = match &meeting {
        Some(m) => match count_meeting_transcript_segments(
            db,
            meeting_id,
            &m.meeting_start,
            m.meeting_end.as_deref(),
        )
        .await
        {
            Ok(count) => Some(count),
            Err(e) => {
                warn!(
                    "meeting_ended: could not count transcript segments of meeting {}: {}",
                    meeting_id, e
                );
                None
            }
        },
        None => None,
    };
    meeting_ended_payload(
        meeting_id,
        meeting.as_ref().map(|m| m.meeting_start.as_str()),
        meeting.as_ref().and_then(|m| m.meeting_end.as_deref()),
        segments,
        end_reason,
    )
}

/// Count of the segments `GET /meetings/:id/transcript` lists for the
/// meeting (`list_meeting_transcript_segments`), without loading them.
///
/// The predicates are the transcript query's: every live segment of the
/// meeting, plus each non-empty background transcription inside the meeting
/// window that is not on a cloud chunk, not by a speaker marked as a
/// hallucination, and not within 15 s of a live segment of the same
/// direction. What differs is the first test on the background rows: a plain
/// comparison of `audio_transcriptions.timestamp` against calendar days
/// around the window, which the timestamp index answers, so only those days'
/// rows reach the exact `julianday` window test. The day margin covers any
/// UTC offset a stored timestamp may carry; every stored spelling starts
/// with `YYYY-MM-DD`. When the meeting start cannot be read as a time, the
/// transcript itself is counted.
pub async fn count_meeting_transcript_segments(
    db: &DatabaseManager,
    meeting_id: i64,
    meeting_start: &str,
    meeting_end: Option<&str>,
) -> Result<usize, sqlx::Error> {
    let Some(start) = parse_db_ts(meeting_start) else {
        return Ok(db.list_meeting_transcript_segments(meeting_id).await?.len());
    };
    let end = meeting_end
        .and_then(parse_db_ts)
        .unwrap_or_else(Utc::now)
        .max(start);
    let first_day = (start - chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    // Exclusive: the day after the last day that may hold a window row.
    let past_last_day = (end + chrono::Duration::days(2))
        .format("%Y-%m-%d")
        .to_string();
    let count: i64 = sqlx::query_scalar(TRANSCRIPT_COUNT_SQL)
        .bind(meeting_id)
        .bind(first_day)
        .bind(past_last_day)
        .fetch_one(&db.pool)
        .await?;
    Ok(usize::try_from(count).unwrap_or(0))
}

/// Bound as `?1` meeting id, `?2` first day, `?3` the day after the last.
const TRANSCRIPT_COUNT_SQL: &str = r#"
    WITH meeting_window AS (
        SELECT
            id AS meeting_id,
            meeting_start,
            COALESCE(meeting_end, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) AS meeting_end
        FROM meetings
        WHERE id = ?1
    )
    SELECT
        (SELECT COUNT(*) FROM meeting_transcript_segments WHERE meeting_id = ?1)
      + (SELECT COUNT(*)
           FROM audio_transcriptions at
           JOIN audio_chunks ac ON ac.id = at.audio_chunk_id
           JOIN meeting_window mw ON 1 = 1
           LEFT JOIN speakers s ON s.id = at.speaker_id
          WHERE at.timestamp >= ?2
            AND at.timestamp < ?3
            AND julianday(at.timestamp) >= julianday(mw.meeting_start)
            AND julianday(at.timestamp) <= julianday(mw.meeting_end)
            AND TRIM(at.transcription) != ''
            AND ac.file_path NOT LIKE 'cloud://%'
            AND (s.id IS NULL OR s.hallucination = 0)
            AND NOT EXISTS (
                SELECT 1 FROM meeting_transcript_segments mts
                WHERE mts.meeting_id = mw.meeting_id
                  AND mts.device_type = CASE
                        WHEN COALESCE(at.is_input_device, 1) THEN 'input'
                        ELSE 'output'
                      END
                  AND ABS(julianday(mts.captured_at) - julianday(at.timestamp))
                      <= (15.0 / 86400.0)
            ))
    "#;

/// A meeting time as the database stores it: RFC 3339, or SQLite's
/// `YYYY-MM-DD HH:MM:SS[.fff]` (read as UTC).
fn parse_db_ts(raw: &str) -> Option<DateTime<Utc>> {
    let raw = raw.trim();
    parse_ts(raw).or_else(|| {
        NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f")
            .ok()
            .map(|naive| naive.and_utc())
    })
}

/// Publish `meeting_ended` for a meeting whose end was just written.
pub async fn emit_meeting_ended(db: &DatabaseManager, meeting_id: i64, end_reason: Option<&str>) {
    let payload = load_meeting_ended_payload(db, meeting_id, end_reason).await;
    debug!("meeting_ended: {}", payload);
    if let Err(e) = screenpipe_events::send_event(MEETING_ENDED_EVENT, payload) {
        warn!("failed to emit meeting_ended event: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_carries_duration_and_segment_count() {
        let p = meeting_ended_payload(
            7,
            Some("2026-10-26T10:00:00.000Z"),
            Some("2026-10-26T10:31:15.500Z"),
            Some(42),
            None,
        );
        assert_eq!(p["meeting_id"], 7);
        assert_eq!(p["meeting_end"], "2026-10-26T10:31:15.500Z");
        assert_eq!(p["duration_secs"], 1875);
        assert_eq!(p["transcript_segments"], 42);
        assert!(p.get("end_reason").is_none());
    }

    #[test]
    fn unknown_parts_are_null_and_reason_is_kept() {
        let p = meeting_ended_payload(3, Some("garbage"), None, None, Some("auto_end"));
        assert_eq!(p["meeting_id"], 3);
        assert!(p["meeting_end"].is_null());
        assert!(p["duration_secs"].is_null());
        assert!(p["transcript_segments"].is_null());
        assert_eq!(p["end_reason"], "auto_end");

        // A clock that ran backwards never yields a negative duration.
        let p = meeting_ended_payload(
            4,
            Some("2026-10-26T10:00:05Z"),
            Some("2026-10-26T10:00:00Z"),
            Some(0),
            Some("  "),
        );
        assert_eq!(p["duration_secs"], 0);
        assert!(p.get("end_reason").is_none());
    }

    #[tokio::test]
    async fn loaded_payload_counts_the_meeting_transcript() {
        let db = DatabaseManager::new("sqlite::memory:", Default::default())
            .await
            .unwrap();
        screenpipe_db::schema_migrations::migrator()
            .run(db.write_pool())
            .await
            .unwrap();
        let id = db
            .insert_meeting("zoom.us", "ui_scan", None, None)
            .await
            .unwrap();
        for (i, text) in ["hello there", "next item"].iter().enumerate() {
            db.insert_meeting_transcript_segment(
                id,
                "deepgram",
                None,
                &format!("item-{i}"),
                "mic",
                "input",
                None,
                text,
                Utc::now(),
            )
            .await
            .unwrap();
        }
        let end = (Utc::now() + chrono::Duration::seconds(90))
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string();
        db.end_meeting(id, &end, Some("explicit_stop"))
            .await
            .unwrap();

        let p = load_meeting_ended_payload(&db, id, Some("explicit_stop")).await;
        assert_eq!(p["meeting_id"], id);
        // The database may store the end in its own RFC 3339 spelling.
        assert_eq!(p["meeting_end"].as_str().and_then(parse_ts), parse_ts(&end));
        assert_eq!(p["transcript_segments"], 2);
        let secs = p["duration_secs"].as_i64().unwrap();
        assert!((89..=91).contains(&secs), "duration {secs}");
        assert_eq!(p["end_reason"], "explicit_stop");

        // An unknown meeting still yields an event to send.
        let p = load_meeting_ended_payload(&db, id + 100, None).await;
        assert_eq!(p["meeting_id"], id + 100);
        assert!(p["meeting_end"].is_null());
        assert!(p["duration_secs"].is_null());
        assert!(p["transcript_segments"].is_null());
    }

    #[tokio::test]
    async fn segment_count_matches_the_transcript_route() {
        use screenpipe_db::{AudioDevice, DeviceType};

        let db = DatabaseManager::new("sqlite::memory:", Default::default())
            .await
            .unwrap();
        screenpipe_db::schema_migrations::migrator()
            .run(db.write_pool())
            .await
            .unwrap();
        let id = db
            .insert_meeting("zoom.us", "ui_scan", None, None)
            .await
            .unwrap();
        let meeting = db.get_meeting_by_id(id).await.unwrap();
        let base = parse_db_ts(&meeting.meeting_start).unwrap() + chrono::Duration::seconds(1);
        let end = base + chrono::Duration::minutes(30);
        db.end_meeting(id, &end.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(), None)
            .await
            .unwrap();

        let mic = AudioDevice {
            name: "Mic".to_string(),
            device_type: DeviceType::Input,
        };
        let speakers = AudioDevice {
            name: "Speakers".to_string(),
            device_type: DeviceType::Output,
        };
        sqlx::query("INSERT INTO speakers (name) VALUES ('echo')")
            .execute(db.write_pool())
            .await
            .unwrap();
        let echo: i64 = sqlx::query_scalar("SELECT id FROM speakers WHERE name = 'echo'")
            .fetch_one(db.write_pool())
            .await
            .unwrap();
        db.mark_speaker_as_hallucination(echo).await.unwrap();

        // Two live segments from the microphone.
        for i in 0..2 {
            db.insert_meeting_transcript_segment(
                id,
                "deepgram",
                None,
                &format!("live-{i}"),
                "Mic",
                "input",
                None,
                "live words",
                base + chrono::Duration::seconds(i * 2),
            )
            .await
            .unwrap();
        }
        // Background rows: (chunk path, device, speaker, text, offset secs).
        let rows: [(&str, &AudioDevice, Option<i64>, &str, i64); 8] = [
            // Kept: other direction than the nearby live segment.
            ("out-a.mp4", &speakers, None, "a question", 3),
            // Dropped: same direction within 15 s of a live segment.
            ("in-a.mp4", &mic, None, "copy of live words", 4),
            // Kept: far from any live segment.
            ("in-b.mp4", &mic, None, "later remark", 600),
            // Dropped: blank, cloud chunk, hallucinating speaker.
            ("in-c.mp4", &mic, None, "   ", 700),
            ("cloud://bucket/in-d.mp4", &mic, None, "uploaded", 800),
            ("out-b.mp4", &speakers, Some(echo), "phantom", 900),
            // Dropped: before the start, after the end.
            ("in-e.mp4", &mic, None, "before", -3600),
            ("in-f.mp4", &mic, None, "after", 30 * 60 + 600),
        ];
        for (path, device, speaker, text, offset) in rows {
            let at = base + chrono::Duration::seconds(offset);
            let chunk = db.insert_audio_chunk(path, Some(at)).await.unwrap();
            db.insert_audio_transcription(
                chunk,
                text,
                0,
                "whisper",
                device,
                speaker,
                None,
                None,
                Some(at),
            )
            .await
            .unwrap();
        }

        let listed = db.list_meeting_transcript_segments(id).await.unwrap().len();
        let counted = count_meeting_transcript_segments(
            &db,
            id,
            &meeting.meeting_start,
            Some(&end.to_rfc3339()),
        )
        .await
        .unwrap();
        assert_eq!(listed, 4);
        assert_eq!(counted, listed);

        // A start the parser cannot read falls back to the transcript itself.
        let fallback = count_meeting_transcript_segments(&db, id, "not a time", None)
            .await
            .unwrap();
        assert_eq!(fallback, listed);

        // The background rows are narrowed through the timestamp index.
        let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "EXPLAIN QUERY PLAN {TRANSCRIPT_COUNT_SQL}"
        )))
        .bind(id)
        .bind("2026-10-26")
        .bind("2026-10-29")
        .fetch_all(&db.pool)
        .await
        .unwrap();
        assert!(
            plan.iter()
                .any(|(_, _, _, detail)| detail.starts_with("SEARCH at USING")
                    && detail.contains("timestamp>? AND timestamp<?")),
            "background rows are not read through a timestamp range: {plan:#?}"
        );
    }
}
