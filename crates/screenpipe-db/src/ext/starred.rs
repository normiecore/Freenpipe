// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Starred work sessions (`starred_sessions`).
//!
//! A starred session is a time range the user marked as worth keeping, with
//! an optional request for HD capture while it runs. The table layout comes
//! from schema v2.7.104 (migration `20261002190000`), so rows written by
//! screenpipe read back here and the other way round.
//!
//! - **Saving** ([`DatabaseManager::save_starred_session`]) uses optimistic
//!   concurrency: revision 0 creates a row (stored as revision 1), any other
//!   revision must equal the stored one and the write bumps it by one. The
//!   revision check, the "start inside another session" check and the write
//!   run in one `BEGIN IMMEDIATE` transaction on the serialised write path,
//!   so two clients saving the same revision get one success and one
//!   conflict.
//! - **Listing** ([`DatabaseManager::list_starred_sessions`]) returns the
//!   sessions overlapping a range, newest first, each with `has_audio`: true
//!   when an audio chunk with a file on disk was recorded inside the session.
//! - **Membership** ([`DatabaseManager::starred_membership`]) tells, for a list
//!   of timestamps, which fall inside a starred session.
//! - **Coverage** ([`DatabaseManager::starred_coverage_islands`]) merges the
//!   sessions overlapping a window into disjoint islands clipped to it.
//! - **HD bindings** ([`DatabaseManager::list_hd_starred_sessions`]) lists the
//!   sessions that asked for HD and have not ended yet, for the engine's
//!   high-FPS controller, and [`DatabaseManager::clear_starred_hd_request`]
//!   writes back an explicit HD stop for one revision.
//!
//! Sessions cover the half-open range `[start, end)`. Timestamps are written
//! as RFC 3339 UTC with millisecond precision and a `Z` suffix, so text order
//! equals time order for every row this crate writes. Reads compare through
//! `julianday()` and normalise through `strftime()`, so rows stored in another
//! RFC 3339 spelling (an offset instead of `Z`, other fractional digits, a
//! space instead of `T`) are still read correctly.

use std::collections::BTreeMap;

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use sqlx::{Error as SqlxError, FromRow};

use crate::DatabaseManager;

/// Largest page [`DatabaseManager::list_starred_sessions`] returns.
pub const MAX_STARRED_PAGE: u32 = 1000;

/// Largest number of timestamps one membership query accepts.
pub const MAX_MEMBERSHIP_TIMESTAMPS: usize = 10_000;

/// Canonical text form of a starred-session timestamp: RFC 3339 in UTC with
/// exactly three fractional digits and a `Z` suffix
/// (`2026-10-21T09:30:00.250Z`, the shape JavaScript's `toISOString` gives).
pub fn format_starred_timestamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Drop everything below the millisecond, the precision sessions are stored
/// at.
pub fn truncate_to_millis(t: DateTime<Utc>) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(t.timestamp_millis())
        .single()
        .unwrap_or(t)
}

/// Parse a timestamp the way this module stores or normalises it. Accepts any
/// RFC 3339 form and SQLite's `YYYY-MM-DD HH:MM:SS[.fff]` (taken as UTC).
pub fn parse_starred_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    let raw = raw.trim();
    if let Ok(t) = DateTime::parse_from_rfc3339(raw) {
        return Some(t.with_timezone(&Utc));
    }
    // RFC 3339 allows a space separator; chrono's strict parser does not.
    if raw.len() > 10 && raw.as_bytes()[10] == b' ' {
        let mut fixed = raw.to_string();
        fixed.replace_range(10..11, "T");
        if let Ok(t) = DateTime::parse_from_rfc3339(&fixed) {
            return Some(t.with_timezone(&Utc));
        }
    }
    for format in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f"] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(raw, format) {
            return Some(Utc.from_utc_datetime(&naive));
        }
    }
    None
}

/// One starred session as the API returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StarredSession {
    pub id: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub hd_requested: bool,
    pub revision: i64,
    /// True when an audio chunk with a non-empty file path has a timestamp in
    /// `[start, end)`.
    pub has_audio: bool,
}

/// A session that asked for HD capture and has not ended yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HdStarredSession {
    pub id: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub revision: i64,
}

/// What a save asks for. `expected_revision` is 0 to create a session, or
/// the revision the client last read to change one.
#[derive(Debug, Clone)]
pub struct StarredSessionWrite<'a> {
    pub id: &'a str,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub hd_requested: bool,
    pub expected_revision: i64,
}

/// Outcome of [`DatabaseManager::save_starred_session`]. Only `Saved` wrote
/// anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StarredSaveOutcome {
    /// The session as stored after the write, with its new revision.
    Saved(StarredSession),
    /// The stored revision differs from the expected one (including a create
    /// for an id that already exists). `current` is the stored revision.
    RevisionConflict { current: i64 },
    /// A change (non-zero revision) for an id with no row.
    Missing,
    /// The start falls inside another session (`other.start <= start <
    /// other.end`).
    StartInsideOther { other_id: String },
    /// `end` is not after `start`, so the row would break `CHECK (end >
    /// start)`. Callers validate first; this guards the table.
    InvalidRange,
}

/// Filter for [`DatabaseManager::list_starred_sessions`]. A session is listed
/// when it overlaps the range: `end > start_time` and `start < end_time`.
/// A missing bound does not restrict that side.
#[derive(Debug, Clone, Default)]
pub struct StarredListFilter {
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub limit: u32,
    pub offset: u32,
}

/// One merged stretch of starred time inside a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoverageIsland {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

/// Merge `[start, end)` intervals into disjoint islands clipped to
/// `[window_start, window_end)`. Overlapping and touching intervals join one
/// island; empty intervals and intervals outside the window are dropped. The
/// result is sorted by start.
pub fn merge_coverage(
    intervals: impl IntoIterator<Item = (DateTime<Utc>, DateTime<Utc>)>,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Vec<CoverageIsland> {
    if window_end <= window_start {
        return Vec::new();
    }
    let mut clipped: Vec<(DateTime<Utc>, DateTime<Utc>)> = intervals
        .into_iter()
        .map(|(s, e)| (s.max(window_start), e.min(window_end)))
        .filter(|(s, e)| s < e)
        .collect();
    clipped.sort();
    let mut islands: Vec<CoverageIsland> = Vec::with_capacity(clipped.len());
    for (s, e) in clipped {
        match islands.last_mut() {
            Some(last) if s <= last.end => {
                if e > last.end {
                    last.end = e;
                }
            }
            _ => islands.push(CoverageIsland { start: s, end: e }),
        }
    }
    islands
}

/// Row shape shared by the session queries. `start_utc`/`end_utc` are the
/// stored values normalised by SQLite (`NULL` when SQLite cannot read them);
/// `start_raw`/`end_raw` are the stored text.
#[derive(Debug, FromRow)]
struct SessionRow {
    id: String,
    start_raw: String,
    end_raw: String,
    start_utc: Option<String>,
    end_utc: Option<String>,
    hd_requested: i64,
    revision: i64,
    has_audio: i64,
}

impl SessionRow {
    fn into_session(self) -> Option<StarredSession> {
        let start = read_stored(self.start_utc.as_deref(), &self.start_raw)?;
        let end = read_stored(self.end_utc.as_deref(), &self.end_raw)?;
        Some(StarredSession {
            id: self.id,
            start,
            end,
            hd_requested: self.hd_requested != 0,
            revision: self.revision,
            has_audio: self.has_audio != 0,
        })
    }
}

fn read_stored(normalised: Option<&str>, raw: &str) -> Option<DateTime<Utc>> {
    let parsed = normalised
        .and_then(parse_starred_timestamp)
        .or_else(|| parse_starred_timestamp(raw));
    if parsed.is_none() {
        tracing::warn!("starred_sessions: unreadable timestamp {raw:?}; row skipped");
    }
    parsed
}

/// Columns of [`SessionRow`] for a `starred_sessions` row aliased `s`.
///
/// `has_audio` looks for an audio chunk recorded inside `[s.start, s.end)`.
/// `audio_chunks.timestamp` is indexed text, written either as RFC 3339
/// (`2026-10-21T09:30:00.25+00:00`) or in SQLite's space-separated form, so
/// the lookup first narrows by a second-precision text range for each
/// spelling (both use `idx_audio_chunks_timestamp`) and then compares exactly
/// with `julianday()`. Chunks without a file on disk do not count.
macro_rules! session_columns {
    () => {
        "s.id AS id, s.start AS start_raw, s.end AS end_raw, \
         strftime('%Y-%m-%dT%H:%M:%fZ', s.start) AS start_utc, \
         strftime('%Y-%m-%dT%H:%M:%fZ', s.end) AS end_utc, \
         s.hd_requested AS hd_requested, s.revision AS revision, \
         EXISTS ( \
             SELECT 1 FROM audio_chunks a \
             WHERE ((a.timestamp >= strftime('%Y-%m-%dT%H:%M:%S', s.start) \
                     AND a.timestamp < strftime('%Y-%m-%dT%H:%M:%S', s.end, '+1 second')) \
                 OR (a.timestamp >= strftime('%Y-%m-%d %H:%M:%S', s.start) \
                     AND a.timestamp < strftime('%Y-%m-%d %H:%M:%S', s.end, '+1 second'))) \
               AND julianday(a.timestamp) >= julianday(s.start) \
               AND julianday(a.timestamp) < julianday(s.end) \
               AND a.file_path != '' \
         ) AS has_audio"
    };
}

/// Overlap filter: `?1` is the range start (or NULL), `?2` the range end (or
/// NULL).
macro_rules! overlap_filter {
    () => {
        "(?1 IS NULL OR julianday(s.end) > julianday(?1)) \
         AND (?2 IS NULL OR julianday(s.start) < julianday(?2))"
    };
}

const LIST_SQL: &str = concat!(
    "SELECT ",
    session_columns!(),
    " FROM starred_sessions s WHERE ",
    overlap_filter!(),
    " ORDER BY julianday(s.start) DESC, s.id ASC LIMIT ?3 OFFSET ?4"
);

const COUNT_SQL: &str = concat!(
    "SELECT COUNT(*) FROM starred_sessions s WHERE ",
    overlap_filter!()
);

const GET_SQL: &str = concat!(
    "SELECT ",
    session_columns!(),
    " FROM starred_sessions s WHERE s.id = ?1"
);

const START_INSIDE_OTHER_SQL: &str = "SELECT id FROM starred_sessions \
     WHERE id != ?1 AND julianday(start) <= julianday(?2) AND julianday(end) > julianday(?2) \
     ORDER BY julianday(start) LIMIT 1";

const MEMBERSHIP_SQL: &str = "SELECT j.key AS idx, EXISTS ( \
         SELECT 1 FROM starred_sessions s \
         WHERE julianday(s.start) <= julianday(j.value) \
           AND julianday(s.end) > julianday(j.value) \
           AND (?2 IS NULL OR s.id = ?2) \
     ) AS member \
     FROM json_each(?1) AS j ORDER BY j.key";

const OVERLAPPING_RANGES_SQL: &str = "SELECT s.start AS start_raw, s.end AS end_raw, \
         strftime('%Y-%m-%dT%H:%M:%fZ', s.start) AS start_utc, \
         strftime('%Y-%m-%dT%H:%M:%fZ', s.end) AS end_utc \
     FROM starred_sessions s \
     WHERE julianday(s.end) > julianday(?1) AND julianday(s.start) < julianday(?2) \
       AND (?3 IS NULL OR s.id = ?3) \
     ORDER BY julianday(s.start)";

const HD_SESSIONS_SQL: &str = "SELECT s.id AS id, s.start AS start_raw, s.end AS end_raw, \
         strftime('%Y-%m-%dT%H:%M:%fZ', s.start) AS start_utc, \
         strftime('%Y-%m-%dT%H:%M:%fZ', s.end) AS end_utc, s.revision AS revision \
     FROM starred_sessions s \
     WHERE s.hd_requested != 0 AND julianday(s.end) > julianday(?1) \
     ORDER BY julianday(s.start), s.id LIMIT ?2";

#[derive(Debug, FromRow)]
struct RangeRow {
    start_raw: String,
    end_raw: String,
    start_utc: Option<String>,
    end_utc: Option<String>,
}

#[derive(Debug, FromRow)]
struct HdRow {
    id: String,
    start_raw: String,
    end_raw: String,
    start_utc: Option<String>,
    end_utc: Option<String>,
    revision: i64,
}

impl DatabaseManager {
    /// Create or change a starred session. See [`StarredSaveOutcome`] for the
    /// results; only `Saved` writes. `start` and `end` are stored at
    /// millisecond precision.
    pub async fn save_starred_session(
        &self,
        write: &StarredSessionWrite<'_>,
    ) -> Result<StarredSaveOutcome, SqlxError> {
        let start = truncate_to_millis(write.start);
        let end = truncate_to_millis(write.end);
        if end <= start {
            return Ok(StarredSaveOutcome::InvalidRange);
        }
        let start_text = format_starred_timestamp(start);
        let end_text = format_starred_timestamp(end);
        let hd = i64::from(write.hd_requested);

        let mut tx = self.begin_immediate_with_retry().await?;
        let current: Option<i64> =
            sqlx::query_scalar("SELECT revision FROM starred_sessions WHERE id = ?1")
                .bind(write.id)
                .fetch_optional(&mut **tx.conn())
                .await?;
        match (write.expected_revision, current) {
            (0, Some(current)) => {
                tx.rollback().await?;
                return Ok(StarredSaveOutcome::RevisionConflict { current });
            }
            (expected, None) if expected != 0 => {
                tx.rollback().await?;
                return Ok(StarredSaveOutcome::Missing);
            }
            (expected, Some(current)) if expected != current => {
                tx.rollback().await?;
                return Ok(StarredSaveOutcome::RevisionConflict { current });
            }
            _ => {}
        }

        let other: Option<String> = sqlx::query_scalar(START_INSIDE_OTHER_SQL)
            .bind(write.id)
            .bind(&start_text)
            .fetch_optional(&mut **tx.conn())
            .await?;
        if let Some(other_id) = other {
            tx.rollback().await?;
            return Ok(StarredSaveOutcome::StartInsideOther { other_id });
        }

        if current.is_none() {
            sqlx::query(
                "INSERT INTO starred_sessions (id, start, end, hd_requested, revision) \
                 VALUES (?1, ?2, ?3, ?4, 1)",
            )
            .bind(write.id)
            .bind(&start_text)
            .bind(&end_text)
            .bind(hd)
            .execute(&mut **tx.conn())
            .await?;
        } else {
            // The revision condition repeats the check above inside the
            // statement, so the compare and the bump can never be split.
            let updated = sqlx::query(
                "UPDATE starred_sessions \
                 SET start = ?2, end = ?3, hd_requested = ?4, revision = revision + 1 \
                 WHERE id = ?1 AND revision = ?5",
            )
            .bind(write.id)
            .bind(&start_text)
            .bind(&end_text)
            .bind(hd)
            .bind(write.expected_revision)
            .execute(&mut **tx.conn())
            .await?
            .rows_affected();
            if updated != 1 {
                let current: Option<i64> =
                    sqlx::query_scalar("SELECT revision FROM starred_sessions WHERE id = ?1")
                        .bind(write.id)
                        .fetch_optional(&mut **tx.conn())
                        .await?;
                tx.rollback().await?;
                return Ok(match current {
                    Some(current) => StarredSaveOutcome::RevisionConflict { current },
                    None => StarredSaveOutcome::Missing,
                });
            }
        }

        let row: SessionRow = sqlx::query_as(GET_SQL)
            .bind(write.id)
            .fetch_one(&mut **tx.conn())
            .await?;
        tx.commit().await?;
        row.into_session()
            .map(StarredSaveOutcome::Saved)
            .ok_or_else(|| SqlxError::Protocol("starred session read back unreadable".into()))
    }

    /// One session by id, with `has_audio`.
    pub async fn get_starred_session(&self, id: &str) -> Result<Option<StarredSession>, SqlxError> {
        let row: Option<SessionRow> = sqlx::query_as(GET_SQL)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.and_then(SessionRow::into_session))
    }

    /// Sessions overlapping the filter's range, ordered by start (newest
    /// first) then id, plus the total number of matching sessions. `limit`
    /// is clamped to `1..=MAX_STARRED_PAGE`.
    pub async fn list_starred_sessions(
        &self,
        filter: &StarredListFilter,
    ) -> Result<(Vec<StarredSession>, i64), SqlxError> {
        let start = filter.start_time.map(format_starred_timestamp);
        let end = filter.end_time.map(format_starred_timestamp);
        let limit = filter.limit.clamp(1, MAX_STARRED_PAGE);
        let rows: Vec<SessionRow> = sqlx::query_as(LIST_SQL)
            .bind(start.as_deref())
            .bind(end.as_deref())
            .bind(i64::from(limit))
            .bind(i64::from(filter.offset))
            .fetch_all(&self.pool)
            .await?;
        let total: i64 = sqlx::query_scalar(COUNT_SQL)
            .bind(start.as_deref())
            .bind(end.as_deref())
            .fetch_one(&self.pool)
            .await?;
        Ok((
            rows.into_iter()
                .filter_map(SessionRow::into_session)
                .collect(),
            total,
        ))
    }

    /// For each timestamp, whether it lies inside a starred session
    /// (`start <= t < end`), in input order. With `session_id`, only that
    /// session counts.
    pub async fn starred_membership(
        &self,
        timestamps: &[DateTime<Utc>],
        session_id: Option<&str>,
    ) -> Result<Vec<bool>, SqlxError> {
        if timestamps.is_empty() {
            return Ok(Vec::new());
        }
        if timestamps.len() > MAX_MEMBERSHIP_TIMESTAMPS {
            return Err(SqlxError::Protocol(format!(
                "starred membership accepts at most {MAX_MEMBERSHIP_TIMESTAMPS} timestamps"
            )));
        }
        let values: Vec<String> = timestamps
            .iter()
            .map(|t| format_starred_timestamp(*t))
            .collect();
        let json = serde_json::to_string(&values)
            .map_err(|e| SqlxError::Protocol(format!("membership timestamps: {e}")))?;
        let rows: Vec<(i64, i64)> = sqlx::query_as(MEMBERSHIP_SQL)
            .bind(json)
            .bind(session_id)
            .fetch_all(&self.pool)
            .await?;
        // json_each returns every element once, keyed by its index.
        let by_index: BTreeMap<i64, bool> = rows.into_iter().map(|(i, m)| (i, m != 0)).collect();
        Ok((0..timestamps.len() as i64)
            .map(|i| by_index.get(&i).copied().unwrap_or(false))
            .collect())
    }

    /// Starred time inside `[window_start, window_end)`, merged into
    /// disjoint islands (see [`merge_coverage`]). With `session_id`, only
    /// that session counts.
    pub async fn starred_coverage_islands(
        &self,
        window_start: DateTime<Utc>,
        window_end: DateTime<Utc>,
        session_id: Option<&str>,
    ) -> Result<Vec<CoverageIsland>, SqlxError> {
        if window_end <= window_start {
            return Ok(Vec::new());
        }
        let rows: Vec<RangeRow> = sqlx::query_as(OVERLAPPING_RANGES_SQL)
            .bind(format_starred_timestamp(window_start))
            .bind(format_starred_timestamp(window_end))
            .bind(session_id)
            .fetch_all(&self.pool)
            .await?;
        let intervals = rows.into_iter().filter_map(|r| {
            Some((
                read_stored(r.start_utc.as_deref(), &r.start_raw)?,
                read_stored(r.end_utc.as_deref(), &r.end_raw)?,
            ))
        });
        Ok(merge_coverage(intervals, window_start, window_end))
    }

    /// Sessions with `hd_requested` that end after `now`, earliest start
    /// first, at most `limit` of them.
    pub async fn list_hd_starred_sessions(
        &self,
        now: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<HdStarredSession>, SqlxError> {
        let rows: Vec<HdRow> = sqlx::query_as(HD_SESSIONS_SQL)
            .bind(format_starred_timestamp(now))
            .bind(i64::from(limit.max(1)))
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|r| {
                Some(HdStarredSession {
                    start: read_stored(r.start_utc.as_deref(), &r.start_raw)?,
                    end: read_stored(r.end_utc.as_deref(), &r.end_raw)?,
                    id: r.id,
                    revision: r.revision,
                })
            })
            .collect())
    }

    /// Switch HD off for session `id`, bumping its revision, but only while
    /// it is still at `revision` and still asks for HD. The engine calls this
    /// when the user stops HD while the session runs, so the stop outlives a
    /// restart. Returns the new revision, or `None` when nothing changed (the
    /// row is gone, was edited since, or has HD off already). The compare and
    /// the write are one statement on the serialised write path.
    pub async fn clear_starred_hd_request(
        &self,
        id: &str,
        revision: i64,
    ) -> Result<Option<i64>, SqlxError> {
        let mut tx = self.begin_immediate_with_retry().await?;
        let new_revision: Option<i64> = sqlx::query_scalar(
            "UPDATE starred_sessions \
             SET hd_requested = 0, revision = revision + 1 \
             WHERE id = ?1 AND revision = ?2 AND hd_requested != 0 \
             RETURNING revision",
        )
        .bind(id)
        .bind(revision)
        .fetch_optional(&mut **tx.conn())
        .await?;
        tx.commit().await?;
        Ok(new_revision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn t(s: &str) -> DateTime<Utc> {
        parse_starred_timestamp(s).unwrap()
    }

    #[test]
    fn canonical_form_has_millis_and_z() {
        let at = t("2026-10-21T09:30:00.250123Z");
        assert_eq!(format_starred_timestamp(at), "2026-10-21T09:30:00.250Z");
        assert_eq!(
            format_starred_timestamp(truncate_to_millis(at)),
            "2026-10-21T09:30:00.250Z"
        );
        assert_eq!(truncate_to_millis(at).timestamp_subsec_nanos(), 250_000_000);
    }

    #[test]
    fn parses_the_spellings_sqlite_and_clients_use() {
        let want = t("2026-10-21T09:30:00Z");
        for raw in [
            "2026-10-21T09:30:00Z",
            "2026-10-21T09:30:00.000Z",
            "2026-10-21T09:30:00+00:00",
            "2026-10-21T11:30:00+02:00",
            "2026-10-21 09:30:00",
            "2026-10-21 09:30:00.000",
            "2026-10-21 09:30:00+00:00",
            "2026-10-21T09:30:00",
        ] {
            assert_eq!(parse_starred_timestamp(raw), Some(want), "{raw}");
        }
        assert_eq!(parse_starred_timestamp("yesterday"), None);
        assert_eq!(parse_starred_timestamp(""), None);
    }

    #[test]
    fn coverage_merges_overlapping_and_touching_ranges() {
        let base = t("2026-10-21T09:00:00Z");
        let at = |m: i64| base + Duration::minutes(m);
        let islands = merge_coverage(
            vec![
                (at(30), at(40)),
                (at(0), at(10)),
                (at(5), at(15)),
                (at(15), at(20)), // touches the previous island
                (at(50), at(55)),
                (at(52), at(53)), // inside the previous one
            ],
            at(-60),
            at(120),
        );
        assert_eq!(
            islands,
            vec![
                CoverageIsland {
                    start: at(0),
                    end: at(20)
                },
                CoverageIsland {
                    start: at(30),
                    end: at(40)
                },
                CoverageIsland {
                    start: at(50),
                    end: at(55)
                },
            ]
        );
    }

    #[test]
    fn coverage_clips_to_the_window_and_drops_outsiders() {
        let base = t("2026-10-21T09:00:00Z");
        let at = |m: i64| base + Duration::minutes(m);
        let islands = merge_coverage(
            vec![(at(-30), at(10)), (at(50), at(90)), (at(100), at(110))],
            at(0),
            at(60),
        );
        assert_eq!(
            islands,
            vec![
                CoverageIsland {
                    start: at(0),
                    end: at(10)
                },
                CoverageIsland {
                    start: at(50),
                    end: at(60)
                },
            ]
        );
        assert!(merge_coverage(vec![(at(0), at(10))], at(10), at(10)).is_empty());
        assert!(merge_coverage(vec![(at(5), at(5))], at(0), at(10)).is_empty());
    }
}
