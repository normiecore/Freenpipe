// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Starred work session routes (`/starred-sessions`) and their HD binding.
//!
//! - `GET /starred-sessions?start_time=&end_time=&limit=&offset=` lists the
//!   sessions overlapping the range (`end > start_time`, `start < end_time`),
//!   newest start first, each with a computed `has_audio`, as
//!   `{ data: [...], pagination: { limit, offset, total } }`. Pipes and
//!   agents may read it.
//! - `POST /starred-sessions` saves one session from
//!   `{ id, start, end, hd_requested, revision }`. Revision 0 creates it and
//!   any other revision must be the stored one (a mismatch is a 409); the
//!   response is the stored session with its new revision. A session must
//!   start no later than now, last less than 24 hours and end at most two
//!   hours from now, and its start may not fall inside another session.
//!   Marking is a user action, so a pipe token gets a 403.
//!
//! A session with `hd_requested` raises the engine's HD capture while it
//! runs ([`crate::high_fps_controller`]). Every successful save re-reads the
//! unfinished HD sessions and installs them in the controller, and
//! [`spawn_hd_sync`] repeats that on a short tick so changes made outside a
//! save (a deleted row, a second engine instance) apply too. When the user
//! stops HD (`POST /capture/hd/stop`) while a starred session raises it, the
//! sync also switches `hd_requested` off in that session's row (bumping its
//! revision), so the stop still holds after a restart.

use std::sync::{Arc, Weak};
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::{header::AUTHORIZATION, HeaderMap, StatusCode};
use axum::response::Json as JsonResponse;
use chrono::{DateTime, Utc};
use oasgen::{oasgen, OaSchema};
use screenpipe_db::ext::starred::{
    format_starred_timestamp, truncate_to_millis, StarredListFilter, StarredSaveOutcome,
    StarredSession, StarredSessionWrite, MAX_STARRED_PAGE,
};
use screenpipe_db::DatabaseManager;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{debug, info, warn};

use super::ext::{documented_routes, DocumentedRoutes, ExtRouter};
use crate::high_fps_controller::{HighFpsController, StarredHdWindow};
use crate::server::AppState;

/// Sessions must be shorter than this.
pub const MAX_SESSION_LENGTH_MS: i64 = 24 * 60 * 60 * 1000;
/// A session may end at most this far after the current time.
pub const MAX_END_AHEAD_MS: i64 = 2 * 60 * 60 * 1000;
/// A start this little after the current time still counts as "now", so a
/// client stamping `start = now` is never refused for clock granularity.
pub const START_CLOCK_SLACK_MS: i64 = 2_000;
/// Page size when the request names none.
pub const DEFAULT_LIST_LIMIT: u32 = 100;
/// How often [`spawn_hd_sync`] re-reads the HD sessions.
pub const HD_SYNC_INTERVAL: Duration = Duration::from_secs(15);
/// Most HD sessions one sync installs.
const HD_SYNC_LIMIT: u32 = crate::high_fps_controller::MAX_STARRED_WINDOWS as u32;

// Client-visible messages. The first five are interoperability strings that
// clients may match on; the others are Freenpipe's own wording.
pub const ERR_IDENTITY: &str = "valid UUID and nonnegative revision required";
pub const ERR_ORDER: &str = "start_time must precede end_time";
pub const ERR_MISSING: &str = "session no longer exists";
pub const ERR_HD_UNAVAILABLE: &str = "HD capture is unavailable";
pub const ERR_VISION_DISABLED: &str = "Screen recording is disabled in settings";
pub const ERR_TIMESTAMPS: &str = "start and end must be RFC 3339 timestamps";
pub const ERR_QUERY_TIMESTAMPS: &str = "start_time and end_time must be RFC 3339 timestamps";
pub const ERR_RANGE: &str = "invalid range: a starred session needs a start that is not in the \
     future, a length under 24 hours and an end at most 2 hours ahead";
pub const ERR_START_INSIDE: &str = "start falls inside another starred session";
pub const ERR_CONFLICT: &str =
    "this starred session was changed somewhere else; reload the sessions and try again";
pub const ERR_AGENT: &str =
    "pipes and agents can read starred sessions but only the user can star or edit them";

/// A starred session on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, OaSchema)]
pub struct StarredSessionJson {
    /// Client-generated UUID.
    pub id: String,
    /// Inclusive start, RFC 3339 UTC.
    pub start: String,
    /// Exclusive end, RFC 3339 UTC.
    pub end: String,
    /// The session asked for HD capture while it runs.
    pub hd_requested: bool,
    /// Bumped on every save; send it back to change the session.
    pub revision: i64,
    /// An audio chunk was recorded inside the session.
    pub has_audio: bool,
}

impl From<StarredSession> for StarredSessionJson {
    fn from(s: StarredSession) -> Self {
        Self {
            id: s.id,
            start: format_starred_timestamp(s.start),
            end: format_starred_timestamp(s.end),
            hd_requested: s.hd_requested,
            revision: s.revision,
            has_audio: s.has_audio,
        }
    }
}

/// Body of `POST /starred-sessions`.
#[derive(Debug, Clone, Deserialize, OaSchema)]
pub struct SaveSession {
    /// Client-generated UUID (hyphenated form).
    pub id: String,
    /// RFC 3339 start.
    pub start: String,
    /// RFC 3339 end.
    pub end: String,
    /// Raise HD capture while the session runs.
    #[serde(default)]
    pub hd_requested: bool,
    /// 0 creates the session; otherwise the revision last read.
    pub revision: i64,
}

/// Query of `GET /starred-sessions`.
#[derive(Debug, Clone, Default, Deserialize, OaSchema)]
pub struct StarredListQuery {
    /// Only sessions ending after this time.
    pub start_time: Option<String>,
    /// Only sessions starting before this time.
    pub end_time: Option<String>,
    /// Page size (default 100, at most 1000).
    pub limit: Option<u32>,
    /// Sessions to skip.
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Serialize, OaSchema)]
pub struct StarredPagination {
    pub limit: u32,
    pub offset: u32,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize, OaSchema)]
pub struct StarredListResponse {
    pub data: Vec<StarredSessionJson>,
    pub pagination: StarredPagination,
}

/// A save that passed the request checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidSave {
    pub id: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub hd_requested: bool,
    pub revision: i64,
}

/// Why a save body was refused before touching the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveRejection {
    /// Not a hyphenated UUID, or a negative revision.
    Identity,
    /// `start` or `end` is not RFC 3339.
    Timestamps,
    /// `end` is not after `start`.
    Order,
    /// Starts in the future, lasts 24 hours or more, or ends more than two
    /// hours from now.
    Range,
}

impl SaveRejection {
    pub fn message(self) -> &'static str {
        match self {
            SaveRejection::Identity => ERR_IDENTITY,
            SaveRejection::Timestamps => ERR_TIMESTAMPS,
            SaveRejection::Order => ERR_ORDER,
            SaveRejection::Range => ERR_RANGE,
        }
    }
}

fn is_hyphenated_uuid(id: &str) -> bool {
    id.len() == 36 && uuid::Uuid::try_parse(id).is_ok()
}

fn parse_rfc3339(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Check a save body against the rules that need no database, at `now`.
/// Times are reduced to whole milliseconds, the precision they are stored at.
pub fn validate_save(body: &SaveSession, now: DateTime<Utc>) -> Result<ValidSave, SaveRejection> {
    let id = body.id.trim();
    if !is_hyphenated_uuid(id) || body.revision < 0 {
        return Err(SaveRejection::Identity);
    }
    let (Some(start), Some(end)) = (parse_rfc3339(&body.start), parse_rfc3339(&body.end)) else {
        return Err(SaveRejection::Timestamps);
    };
    let (start, end) = (truncate_to_millis(start), truncate_to_millis(end));
    if end <= start {
        return Err(SaveRejection::Order);
    }
    let length_ms = (end - start).num_milliseconds();
    let start_ahead_ms = (start - now).num_milliseconds();
    let end_ahead_ms = (end - now).num_milliseconds();
    if start_ahead_ms > START_CLOCK_SLACK_MS
        || length_ms >= MAX_SESSION_LENGTH_MS
        || end_ahead_ms > MAX_END_AHEAD_MS
    {
        return Err(SaveRejection::Range);
    }
    Ok(ValidSave {
        id: id.to_string(),
        start,
        end,
        hd_requested: body.hd_requested,
        revision: body.revision,
    })
}

/// True when the request carries a pipe token (`Bearer sp_pipe_…`), whether
/// or not the token is registered. Such requests may read but not mark.
pub fn is_agent_request(headers: &HeaderMap) -> bool {
    headers
        .get_all(AUTHORIZATION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|value| {
            let value = value.trim();
            value.len() > 7
                && value.is_char_boundary(7)
                && value[..7].eq_ignore_ascii_case("bearer ")
                && value[7..].trim_start().starts_with("sp_pipe_")
        })
}

type ApiError = (StatusCode, JsonResponse<Value>);

fn api_error(status: StatusCode, message: &str) -> ApiError {
    (status, JsonResponse(json!({ "error": message })))
}

fn db_error(context: &str, e: &sqlx::Error) -> ApiError {
    warn!("starred sessions: {context}: {e}");
    api_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        &format!("failed to {context}"),
    )
}

/// `raw` with a space just before a trailing `HH:MM` offset read as `+`. A
/// `+` sent unencoded in a query string decodes to a space, so
/// `…T10:00:00+02:00` arrives as `…T10:00:00 02:00`.
fn restore_offset_plus(raw: &str) -> Option<String> {
    let b = raw.as_bytes();
    let n = b.len();
    // Shortest RFC 3339 date-time with an offset is 25 bytes.
    if n < 25 {
        return None;
    }
    let tail = &b[n - 6..];
    let offset_shaped = tail[0] == b' '
        && tail[1].is_ascii_digit()
        && tail[2].is_ascii_digit()
        && tail[3] == b':'
        && tail[4].is_ascii_digit()
        && tail[5].is_ascii_digit();
    // The last six bytes are ASCII, so both slice points are char boundaries.
    offset_shaped.then(|| format!("{}+{}", &raw[..n - 6], &raw[n - 5..]))
}

/// An optional RFC 3339 query bound; blank means absent. A positive offset
/// whose `+` arrived as a space is accepted too.
fn query_bound(raw: Option<&str>) -> Result<Option<DateTime<Utc>>, ApiError> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) => parse_rfc3339(s)
            .or_else(|| restore_offset_plus(s).as_deref().and_then(parse_rfc3339))
            .map(Some)
            .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, ERR_QUERY_TIMESTAMPS)),
    }
}

/// List the starred sessions overlapping a time range, newest first.
#[oasgen]
pub async fn list_starred_sessions(
    State(state): State<Arc<AppState>>,
    Query(query): Query<StarredListQuery>,
) -> Result<JsonResponse<StarredListResponse>, ApiError> {
    let start_time = query_bound(query.start_time.as_deref())?;
    let end_time = query_bound(query.end_time.as_deref())?;
    if let (Some(start), Some(end)) = (start_time, end_time) {
        if end <= start {
            return Err(api_error(StatusCode::BAD_REQUEST, ERR_ORDER));
        }
    }
    let limit = query
        .limit
        .unwrap_or(DEFAULT_LIST_LIMIT)
        .clamp(1, MAX_STARRED_PAGE);
    let offset = query.offset.unwrap_or(0);
    let (sessions, total) = state
        .db
        .list_starred_sessions(&StarredListFilter {
            start_time,
            end_time,
            limit,
            offset,
        })
        .await
        .map_err(|e| db_error("list starred sessions", &e))?;
    Ok(JsonResponse(StarredListResponse {
        data: sessions.into_iter().map(Into::into).collect(),
        pagination: StarredPagination {
            limit,
            offset,
            total,
        },
    }))
}

/// Create or change a starred session (optimistic revision).
#[oasgen]
pub async fn save_starred_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    JsonResponse(body): JsonResponse<SaveSession>,
) -> Result<JsonResponse<StarredSessionJson>, ApiError> {
    if is_agent_request(&headers) {
        return Err(api_error(StatusCode::FORBIDDEN, ERR_AGENT));
    }
    let now = Utc::now();
    let valid =
        validate_save(&body, now).map_err(|r| api_error(StatusCode::BAD_REQUEST, r.message()))?;
    // HD only matters for a session that has not ended yet; a past session
    // keeps its flag whatever the capture settings are now.
    if valid.hd_requested && valid.end > now {
        if state.vision_disabled {
            return Err(api_error(StatusCode::CONFLICT, ERR_VISION_DISABLED));
        }
        if state.high_fps_controller.is_none() {
            return Err(api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                ERR_HD_UNAVAILABLE,
            ));
        }
    }

    let outcome = state
        .db
        .save_starred_session(&StarredSessionWrite {
            id: &valid.id,
            start: valid.start,
            end: valid.end,
            hd_requested: valid.hd_requested,
            expected_revision: valid.revision,
        })
        .await
        .map_err(|e| db_error("save starred session", &e))?;

    match outcome {
        StarredSaveOutcome::Saved(session) => {
            if let Some(controller) = state.high_fps_controller.as_ref() {
                if let Err(e) = sync_hd_bindings(&state.db, controller).await {
                    // The refresh tick retries; the save itself stands.
                    warn!("starred sessions: HD binding refresh after save failed: {e}");
                }
            }
            info!(
                session_id = %session.id,
                revision = session.revision,
                hd_requested = session.hd_requested,
                "starred session saved"
            );
            Ok(JsonResponse(session.into()))
        }
        StarredSaveOutcome::RevisionConflict { current } => Err((
            StatusCode::CONFLICT,
            JsonResponse(json!({ "error": ERR_CONFLICT, "current_revision": current })),
        )),
        StarredSaveOutcome::Missing => Err(api_error(StatusCode::NOT_FOUND, ERR_MISSING)),
        StarredSaveOutcome::StartInsideOther { other_id } => Err((
            StatusCode::BAD_REQUEST,
            JsonResponse(json!({ "error": ERR_START_INSIDE, "conflicting_session_id": other_id })),
        )),
        StarredSaveOutcome::InvalidRange => Err(api_error(StatusCode::BAD_REQUEST, ERR_ORDER)),
    }
}

/// Write the starred HD stops the user made (tray or settings, through
/// `POST /capture/hd/stop`) back to the store: each stopped session that is
/// still at the stopped revision gets `hd_requested` off and a new revision,
/// so the stop holds after a restart. Stops the store cannot write yet are
/// handed back to the controller for the next sync.
async fn persist_starred_stops(
    db: &DatabaseManager,
    controller: &HighFpsController,
) -> Result<(), sqlx::Error> {
    let stops = controller.take_stopped_starred_windows();
    for (i, stop) in stops.iter().enumerate() {
        match db.clear_starred_hd_request(&stop.id, stop.revision).await {
            Ok(Some(revision)) => info!(
                session_id = %stop.id,
                stopped_revision = stop.revision,
                revision,
                "starred session: HD request switched off after an explicit HD stop"
            ),
            // Edited, deleted or switched off since: the newer state stands.
            Ok(None) => debug!(
                session_id = %stop.id,
                stopped_revision = stop.revision,
                "starred session changed since the HD stop; nothing to write"
            ),
            Err(e) => {
                controller.requeue_stopped_starred_windows(stops[i..].to_vec());
                return Err(e);
            }
        }
    }
    Ok(())
}

/// Write back pending HD stops, then install the unfinished HD sessions
/// from the store into `controller`. Returns how many were read. The ticket
/// is taken before the read, so an older read finishing late never replaces
/// a newer one. A failed stop write is reported after the install, which
/// happens either way (stopped windows stay off in the controller).
pub async fn sync_hd_bindings(
    db: &DatabaseManager,
    controller: &HighFpsController,
) -> Result<usize, sqlx::Error> {
    let stops_written = persist_starred_stops(db, controller).await;
    let ticket = controller.starred_sync_ticket();
    let now = controller.now_utc();
    let rows = db.list_hd_starred_sessions(now, HD_SYNC_LIMIT).await?;
    let windows: Vec<StarredHdWindow> = rows
        .into_iter()
        .map(|r| StarredHdWindow {
            id: r.id,
            start: r.start,
            end: r.end,
            revision: r.revision,
        })
        .collect();
    let count = windows.len();
    controller.install_starred_windows(ticket, windows);
    stops_written?;
    Ok(count)
}

/// Keep `controller`'s starred HD windows in step with the store: sync now,
/// then every [`HD_SYNC_INTERVAL`] and right after an explicit HD stop (so
/// the stop reaches the store at once). The task holds only weak references
/// to the database and the controller and ends once either is dropped.
pub fn spawn_hd_sync(
    db: &Arc<DatabaseManager>,
    controller: &Arc<HighFpsController>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run_hd_sync(
        Arc::downgrade(db),
        Arc::downgrade(controller),
        HD_SYNC_INTERVAL,
    ))
}

async fn run_hd_sync(
    db: Weak<DatabaseManager>,
    controller: Weak<HighFpsController>,
    every: Duration,
) {
    let Some(stopped) = controller.upgrade().map(|c| c.starred_stop_signal()) else {
        return;
    };
    let mut ticker = tokio::time::interval(every);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut failing = false;
    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            _ = stopped.notified() => {}
        }
        let (Some(db), Some(controller)) = (db.upgrade(), controller.upgrade()) else {
            debug!("starred sessions: HD sync stopped (engine gone)");
            return;
        };
        match sync_hd_bindings(&db, &controller).await {
            Ok(_) if failing => {
                info!("starred sessions: HD sync recovered");
                failing = false;
            }
            Ok(_) => {}
            Err(e) if !failing => {
                warn!("starred sessions: HD sync failed, will retry: {e}");
                failing = true;
            }
            Err(e) => debug!("starred sessions: HD sync still failing: {e}"),
        }
    }
}

/// `GET` and `POST /starred-sessions`.
pub fn documented() -> DocumentedRoutes {
    documented_routes!(oasgen::Server::axum()
        .get("/starred-sessions", list_starred_sessions)
        .post("/starred-sessions", save_starred_session))
}

/// No undocumented routes.
pub fn router() -> ExtRouter {
    ExtRouter::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::high_fps_controller::{test_clock::ManualClock, DefaultMode, SessionKind};
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use axum::Router;
    use chrono::Duration as ChronoDuration;
    use screenpipe_audio::audio_manager::AudioManagerBuilder;
    use tower::ServiceExt;

    const ID: &str = "6f1c1d4e-2b8a-4f0e-9a51-0d6c7f3b2a10";
    const OTHER: &str = "0b7e9a2c-5d43-4e8f-8c21-7a9d3e6f1b54";

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-21T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn body(start: DateTime<Utc>, end: DateTime<Utc>, revision: i64) -> SaveSession {
        SaveSession {
            id: ID.to_string(),
            start: format_starred_timestamp(start),
            end: format_starred_timestamp(end),
            hd_requested: false,
            revision,
        }
    }

    fn mins(m: i64) -> ChronoDuration {
        ChronoDuration::minutes(m)
    }

    // ── request checks ──────────────────────────────────────────────────

    #[test]
    fn a_plain_session_passes() {
        let v = validate_save(&body(now() - mins(5), now() + mins(15), 0), now()).unwrap();
        assert_eq!(v.id, ID);
        assert_eq!(v.start, now() - mins(5));
        assert_eq!(v.end, now() + mins(15));
        assert_eq!(v.revision, 0);
    }

    #[test]
    fn identity_rules() {
        let mut b = body(now() - mins(5), now(), 0);
        for bad in [
            "",
            "not-a-uuid",
            "6f1c1d4e2b8a4f0e9a510d6c7f3b2a10",
            "{6f1c1d4e-2b8a-4f0e-9a51-0d6c7f3b2a10}",
            "urn:uuid:6f1c1d4e-2b8a-4f0e-9a51-0d6c7f3b2a10",
        ] {
            b.id = bad.to_string();
            assert_eq!(
                validate_save(&b, now()),
                Err(SaveRejection::Identity),
                "{bad}"
            );
        }
        b.id = ID.to_uppercase();
        assert!(validate_save(&b, now()).is_ok());
        b.id = ID.to_string();
        b.revision = -1;
        assert_eq!(validate_save(&b, now()), Err(SaveRejection::Identity));
        assert_eq!(SaveRejection::Identity.message(), ERR_IDENTITY);
    }

    #[test]
    fn timestamps_must_be_rfc3339() {
        let mut b = body(now() - mins(5), now(), 0);
        b.start = "yesterday".into();
        assert_eq!(validate_save(&b, now()), Err(SaveRejection::Timestamps));
        b.start = "2026-10-21 08:55:00".into(); // no offset
        assert_eq!(validate_save(&b, now()), Err(SaveRejection::Timestamps));
        b.start = "2026-10-21T10:55:00+02:00".into();
        assert_eq!(validate_save(&b, now()).unwrap().start, now() - mins(5));
    }

    #[test]
    fn order_rule() {
        assert_eq!(
            validate_save(&body(now(), now(), 0), now()),
            Err(SaveRejection::Order)
        );
        assert_eq!(
            validate_save(&body(now(), now() - mins(1), 0), now()),
            Err(SaveRejection::Order)
        );
        // Less than a millisecond apart collapses to nothing.
        let start = now() - mins(1);
        let mut b = body(start, start, 0);
        b.end = (start + ChronoDuration::microseconds(400)).to_rfc3339();
        assert_eq!(validate_save(&b, now()), Err(SaveRejection::Order));
        assert_eq!(SaveRejection::Order.message(), ERR_ORDER);
    }

    #[test]
    fn range_rules() {
        // 25 hours.
        assert_eq!(
            validate_save(&body(now() - mins(25 * 60), now(), 0), now()),
            Err(SaveRejection::Range)
        );
        // Exactly 24 hours is not "less than 24 h".
        assert_eq!(
            validate_save(&body(now() - mins(24 * 60), now(), 0), now()),
            Err(SaveRejection::Range)
        );
        // Just under 24 hours.
        let start = now() - mins(24 * 60) + ChronoDuration::milliseconds(1);
        assert!(validate_save(&body(start, now(), 0), now()).is_ok());
        // Starting in the future.
        assert_eq!(
            validate_save(&body(now() + mins(1), now() + mins(10), 0), now()),
            Err(SaveRejection::Range)
        );
        // ... within the clock slack is fine.
        let start = now() + ChronoDuration::milliseconds(START_CLOCK_SLACK_MS);
        assert!(validate_save(&body(start, now() + mins(10), 0), now()).is_ok());
        // Ending more than two hours ahead.
        assert_eq!(
            validate_save(&body(now() - mins(1), now() + mins(121), 0), now()),
            Err(SaveRejection::Range)
        );
        assert!(validate_save(&body(now() - mins(1), now() + mins(120), 0), now()).is_ok());
        // Long-past sessions are fine.
        assert!(validate_save(
            &body(
                now() - mins(30 * 24 * 60),
                now() - mins(30 * 24 * 60 - 30),
                3
            ),
            now()
        )
        .is_ok());
        assert_eq!(SaveRejection::Range.message(), ERR_RANGE);
    }

    #[test]
    fn pipe_tokens_are_agents() {
        let mut h = HeaderMap::new();
        assert!(!is_agent_request(&h));
        h.insert(AUTHORIZATION, "Bearer fp-dev-key".parse().unwrap());
        assert!(!is_agent_request(&h));
        h.insert(AUTHORIZATION, "Bearer sp_pipe_abc123".parse().unwrap());
        assert!(is_agent_request(&h));
        h.insert(AUTHORIZATION, "bearer   sp_pipe_abc123".parse().unwrap());
        assert!(is_agent_request(&h));
        h.insert(AUTHORIZATION, "Basic sp_pipe_abc123".parse().unwrap());
        assert!(!is_agent_request(&h));
    }

    // ── over HTTP ───────────────────────────────────────────────────────

    struct Harness {
        app: Router,
        db: Arc<DatabaseManager>,
        controller: Option<Arc<HighFpsController>>,
        _dir: tempfile::TempDir,
    }

    async fn harness(vision_disabled: bool, with_controller: bool) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(
            DatabaseManager::new(
                dir.path().join("db.sqlite").to_str().unwrap(),
                Default::default(),
            )
            .await
            .unwrap(),
        );
        let audio_manager = Arc::new(
            AudioManagerBuilder::new()
                .output_path(dir.path().to_path_buf())
                .is_disabled(true)
                .build(db.clone())
                .await
                .unwrap(),
        );
        let mut server = crate::SCServer::new(
            db.clone(),
            std::net::SocketAddr::from(([127, 0, 0, 1], 3035)),
            dir.path().to_path_buf(),
            vision_disabled,
            true,
            audio_manager,
            false,
            "balanced".to_string(),
        );
        let controller = with_controller.then(|| {
            Arc::new(HighFpsController::new(
                None,
                DefaultMode::Ask,
                crate::high_fps_controller::DEFAULT_INTERVAL_MS,
            ))
        });
        if let Some(c) = controller.clone() {
            server = server.with_high_fps_controller(c);
        }
        Harness {
            app: server.create_router().await,
            db,
            controller,
            _dir: dir,
        }
    }

    async fn call(
        app: &Router,
        method: &str,
        uri: &str,
        body: Option<Value>,
        token: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(token) = token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        let body = match body {
            Some(v) => {
                req = req.header("Content-Type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        (status, value)
    }

    fn save_json(
        id: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        hd: bool,
        revision: i64,
    ) -> Value {
        json!({
            "id": id,
            "start": format_starred_timestamp(start),
            "end": format_starred_timestamp(end),
            "hd_requested": hd,
            "revision": revision,
        })
    }

    #[tokio::test]
    async fn create_edit_conflict_and_missing_over_http() {
        let h = harness(false, true).await;
        let t = Utc::now();
        let (status, created) = call(
            &h.app,
            "POST",
            "/starred-sessions",
            Some(save_json(ID, t - mins(5), t + mins(10), false, 0)),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{created}");
        assert_eq!(created["revision"], 1);
        assert_eq!(created["id"], ID);
        assert_eq!(created["hd_requested"], false);
        assert_eq!(created["has_audio"], false);
        // The engine's response layer may localise timestamps; the instant
        // is what counts.
        assert_eq!(
            parse_rfc3339(created["start"].as_str().unwrap()),
            Some(truncate_to_millis(t - mins(5)))
        );
        let (_, utc) = call(&h.app, "GET", "/starred-sessions?timezone=utc", None, None).await;
        assert_eq!(
            utc["data"][0]["start"],
            format_starred_timestamp(truncate_to_millis(t - mins(5)))
        );

        let (status, edited) = call(
            &h.app,
            "POST",
            "/starred-sessions",
            Some(save_json(ID, t - mins(5), t + mins(25), false, 1)),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{edited}");
        assert_eq!(edited["revision"], 2);

        // A second window still holding revision 1.
        let (status, stale) = call(
            &h.app,
            "POST",
            "/starred-sessions",
            Some(save_json(ID, t - mins(5), t + mins(40), false, 1)),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(stale["error"], ERR_CONFLICT);
        assert_eq!(stale["current_revision"], 2);

        let (status, missing) = call(
            &h.app,
            "POST",
            "/starred-sessions",
            Some(save_json(OTHER, t - mins(50), t - mins(40), false, 3)),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(missing["error"], ERR_MISSING);

        let (status, inside) = call(
            &h.app,
            "POST",
            "/starred-sessions",
            Some(save_json(OTHER, t - mins(1), t + mins(5), false, 0)),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(inside["error"], ERR_START_INSIDE);
        assert_eq!(inside["conflicting_session_id"], ID);
    }

    #[tokio::test]
    async fn concurrent_posts_with_one_revision_give_one_200_and_one_409() {
        let h = harness(false, true).await;
        let t = Utc::now();
        let (status, _) = call(
            &h.app,
            "POST",
            "/starred-sessions",
            Some(save_json(ID, t - mins(5), t + mins(10), false, 0)),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        for round in 1..=5i64 {
            let a = call(
                &h.app,
                "POST",
                "/starred-sessions",
                Some(save_json(
                    ID,
                    t - mins(5),
                    t + mins(10 + round),
                    false,
                    round,
                )),
                None,
            );
            let b = call(
                &h.app,
                "POST",
                "/starred-sessions",
                Some(save_json(
                    ID,
                    t - mins(5),
                    t + mins(20 + round),
                    false,
                    round,
                )),
                None,
            );
            let ((sa, _), (sb, _)) = tokio::join!(a, b);
            let mut statuses = [sa, sb];
            statuses.sort();
            assert_eq!(
                statuses,
                [StatusCode::OK, StatusCode::CONFLICT],
                "round {round}"
            );
        }
        let stored = h.db.get_starred_session(ID).await.unwrap().unwrap();
        assert_eq!(stored.revision, 6);
    }

    #[tokio::test]
    async fn invalid_bodies_are_refused_before_writing() {
        let h = harness(false, true).await;
        let t = Utc::now();
        let cases = [
            (save_json(ID, t - mins(25 * 60), t, false, 0), ERR_RANGE),
            (
                save_json(ID, t - mins(5), t - mins(10), false, 0),
                ERR_ORDER,
            ),
            (save_json("nope", t - mins(5), t, false, 0), ERR_IDENTITY),
            (save_json(ID, t - mins(5), t, false, -2), ERR_IDENTITY),
            (
                json!({"id": ID, "start": "soon", "end": "later", "revision": 0}),
                ERR_TIMESTAMPS,
            ),
        ];
        for (body, message) in cases {
            let (status, value) = call(&h.app, "POST", "/starred-sessions", Some(body), None).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
            assert_eq!(value["error"], message);
        }
        let (rows, total) =
            h.db.list_starred_sessions(&StarredListFilter {
                limit: 10,
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(rows.is_empty());
        assert_eq!(total, 0);
    }

    #[tokio::test]
    async fn listing_over_http_filters_by_range_and_reports_audio() {
        let h = harness(false, true).await;
        let t = Utc::now();
        for (id, start, end) in [
            (ID, t - mins(90), t - mins(60)),
            (OTHER, t - mins(30), t - mins(10)),
        ] {
            let (status, v) = call(
                &h.app,
                "POST",
                "/starred-sessions",
                Some(save_json(id, start, end, false, 0)),
                None,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{v}");
        }
        h.db.insert_audio_chunk("/tmp/fp-test/audio.mp4", Some(t - mins(20)))
            .await
            .unwrap();

        let uri = format!(
            "/starred-sessions?start_time={}&end_time={}",
            format_starred_timestamp(t - mins(40)),
            format_starred_timestamp(t)
        );
        let (status, list) = call(&h.app, "GET", &uri, None, None).await;
        assert_eq!(status, StatusCode::OK, "{list}");
        let data = list["data"].as_array().unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0]["id"], OTHER);
        assert_eq!(data[0]["has_audio"], true);
        assert_eq!(list["pagination"]["total"], 1);
        assert_eq!(list["pagination"]["limit"], DEFAULT_LIST_LIMIT);

        // Everything, newest first; pipes may read.
        let (status, all) = call(
            &h.app,
            "GET",
            "/starred-sessions?limit=100",
            None,
            Some("sp_pipe_reader"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{all}");
        let ids: Vec<&str> = all["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec![OTHER, ID]);
        assert_eq!(all["data"][1]["has_audio"], false);

        let backwards = format!(
            "/starred-sessions?start_time={}&end_time={}",
            format_starred_timestamp(t),
            format_starred_timestamp(t - mins(40))
        );
        let (status, v) = call(&h.app, "GET", &backwards, None, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], ERR_ORDER);
        let (status, v) = call(
            &h.app,
            "GET",
            "/starred-sessions?start_time=noon",
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], ERR_QUERY_TIMESTAMPS);

        // `+00:00` sent unencoded (as `isoformat()` prints it in a shell
        // script) decodes to a space; the bound still reads.
        let plus = format!(
            "/starred-sessions?start_time={}&end_time={}",
            (t - mins(40)).to_rfc3339(),
            t.to_rfc3339()
        );
        assert!(plus.contains("+00:00"), "{plus}");
        let (status, v) = call(&h.app, "GET", &plus, None, None).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["pagination"]["total"], 1);
        assert_eq!(v["data"][0]["id"], OTHER);
    }

    #[test]
    fn a_plus_offset_decoded_to_a_space_still_reads() {
        let bound = |s: &str| query_bound(Some(s)).map_err(|(status, _)| status);
        let expect = DateTime::parse_from_rfc3339("2026-10-21T08:00:00.5Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(bound("2026-10-21T10:00:00.5 02:00"), Ok(Some(expect)));
        assert_eq!(bound("2026-10-21T08:00:00.5 00:00"), Ok(Some(expect)));
        assert_eq!(bound("2026-10-21T10:00:00.5+02:00"), Ok(Some(expect)));
        assert_eq!(bound("2026-10-21T06:00:00.5-02:00"), Ok(Some(expect)));
        assert_eq!(bound(" "), Ok(None));
        for bad in [
            "2026-10-21 02:00",
            "noon 02:00",
            "2026-10-21T10:00:00.5 2:00",
        ] {
            assert_eq!(bound(bad), Err(StatusCode::BAD_REQUEST), "{bad}");
        }
    }

    #[tokio::test]
    async fn pipes_cannot_mark() {
        let h = harness(false, true).await;
        let t = Utc::now();
        let (status, v) = call(
            &h.app,
            "POST",
            "/starred-sessions",
            Some(save_json(ID, t - mins(5), t + mins(5), false, 0)),
            Some("sp_pipe_writer"),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
        assert!(h.db.get_starred_session(ID).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn an_hd_session_raises_the_controller_and_ending_it_drops_hd() {
        let h = harness(false, true).await;
        let controller = h.controller.clone().unwrap();
        let t = Utc::now();
        let (status, v) = call(
            &h.app,
            "POST",
            "/starred-sessions",
            Some(save_json(ID, t - mins(1), t + mins(15), true, 0)),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["hd_requested"], true);
        let snap = controller.snapshot();
        assert!(snap.active);
        assert_eq!(snap.kind, Some(SessionKind::Starred));
        let (status, state) = call(&h.app, "GET", "/capture/hd", None, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(state["session"]["kind"], "starred");

        // The user ends the session now: HD stops with the save.
        let end = Utc::now() + ChronoDuration::milliseconds(1);
        let (status, v) = call(
            &h.app,
            "POST",
            "/starred-sessions",
            Some(save_json(ID, t - mins(1), end, true, 1)),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(!controller.snapshot().active);
    }

    #[tokio::test]
    async fn switching_hd_off_drops_hd_at_once() {
        let h = harness(false, true).await;
        let controller = h.controller.clone().unwrap();
        let t = Utc::now();
        call(
            &h.app,
            "POST",
            "/starred-sessions",
            Some(save_json(ID, t - mins(1), t + mins(15), true, 0)),
            None,
        )
        .await;
        assert!(controller.snapshot().active);
        let (status, _) = call(
            &h.app,
            "POST",
            "/starred-sessions",
            Some(save_json(ID, t - mins(1), t + mins(15), false, 1)),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(!controller.snapshot().active);
    }

    #[tokio::test]
    async fn hd_requests_need_screen_recording_and_a_controller() {
        let t = Utc::now();
        let disabled = harness(true, true).await;
        let (status, v) = call(
            &disabled.app,
            "POST",
            "/starred-sessions",
            Some(save_json(ID, t - mins(1), t + mins(15), true, 0)),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(v["error"], ERR_VISION_DISABLED);
        // Without HD the session is saved.
        let (status, _) = call(
            &disabled.app,
            "POST",
            "/starred-sessions",
            Some(save_json(ID, t - mins(1), t + mins(15), false, 0)),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        // A past session keeps its HD flag whatever the settings are now.
        let (status, _) = call(
            &disabled.app,
            "POST",
            "/starred-sessions",
            Some(save_json(OTHER, t - mins(60), t - mins(30), true, 0)),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let no_controller = harness(false, false).await;
        let (status, v) = call(
            &no_controller.app,
            "POST",
            "/starred-sessions",
            Some(save_json(ID, t - mins(1), t + mins(15), true, 0)),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(v["error"], ERR_HD_UNAVAILABLE);
    }

    #[tokio::test]
    async fn the_sync_follows_rows_changed_outside_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(
            DatabaseManager::new(
                dir.path().join("db.sqlite").to_str().unwrap(),
                Default::default(),
            )
            .await
            .unwrap(),
        );
        let clock = ManualClock::at(now());
        let controller =
            Arc::new(HighFpsController::new(None, DefaultMode::Ask, 100).with_clock(clock.clone()));
        // A row written by another client (or screenpipe) before startup.
        sqlx::query(
            "INSERT INTO starred_sessions (id, start, end, hd_requested, revision) \
             VALUES (?1, '2026-10-21T08:55:00.000Z', '2026-10-21T09:10:00.000Z', 1, 3)",
        )
        .bind(ID)
        .execute(db.write_pool())
        .await
        .unwrap();

        assert_eq!(sync_hd_bindings(&db, &controller).await.unwrap(), 1);
        assert_eq!(controller.snapshot().kind, Some(SessionKind::Starred));
        assert_eq!(controller.active_starred_window().unwrap().revision, 3);

        // The row is deleted behind the engine's back.
        sqlx::query("DELETE FROM starred_sessions WHERE id = ?1")
            .bind(ID)
            .execute(db.write_pool())
            .await
            .unwrap();
        assert_eq!(sync_hd_bindings(&db, &controller).await.unwrap(), 0);
        assert!(!controller.snapshot().active);

        // Ended sessions are not loaded at all.
        sqlx::query(
            "INSERT INTO starred_sessions (id, start, end, hd_requested, revision) \
             VALUES (?1, '2026-10-21T08:00:00.000Z', '2026-10-21T08:30:00.000Z', 1, 1)",
        )
        .bind(OTHER)
        .execute(db.write_pool())
        .await
        .unwrap();
        assert_eq!(sync_hd_bindings(&db, &controller).await.unwrap(), 0);
        clock.advance(Duration::from_secs(60));
        assert!(!controller.snapshot().active);
    }

    #[tokio::test]
    async fn the_sync_task_stops_when_the_controller_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(
            DatabaseManager::new(
                dir.path().join("db.sqlite").to_str().unwrap(),
                Default::default(),
            )
            .await
            .unwrap(),
        );
        let controller = Arc::new(HighFpsController::new(None, DefaultMode::Ask, 100));
        let task = tokio::spawn(run_hd_sync(
            Arc::downgrade(&db),
            Arc::downgrade(&controller),
            Duration::from_millis(10),
        ));
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!task.is_finished());
        drop(controller);
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("sync task ends once the controller is gone")
            .unwrap();
    }

    #[tokio::test]
    async fn an_hd_stop_is_written_back_and_holds_after_a_restart() {
        let h = harness(false, true).await;
        let controller = h.controller.clone().unwrap();
        let t = Utc::now();
        let (status, v) = call(
            &h.app,
            "POST",
            "/starred-sessions",
            Some(save_json(ID, t - mins(1), t + mins(30), true, 0)),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(controller.snapshot().kind, Some(SessionKind::Starred));

        // The tray's "Stop HD recording".
        let (status, v) = call(&h.app, "POST", "/capture/hd/stop", None, None).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["active"], false, "{v}");

        // The next sync switches the request off in the row.
        sync_hd_bindings(&h.db, &controller).await.unwrap();
        let row = h.db.get_starred_session(ID).await.unwrap().unwrap();
        assert!(!row.hd_requested);
        assert_eq!(row.revision, 2);
        assert!(controller.take_stopped_starred_windows().is_empty());

        // A restart: a fresh controller reading the same store stays off.
        let fresh = HighFpsController::new(None, DefaultMode::Ask, 100);
        assert_eq!(sync_hd_bindings(&h.db, &fresh).await.unwrap(), 0);
        assert!(!fresh.snapshot().active);

        // A client still holding revision 1 reloads first; turning HD on
        // again from the new revision is a fresh request.
        let (status, v) = call(
            &h.app,
            "POST",
            "/starred-sessions",
            Some(save_json(ID, t - mins(1), t + mins(30), true, 1)),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{v}");
        assert_eq!(v["current_revision"], 2);
        let (status, v) = call(
            &h.app,
            "POST",
            "/starred-sessions",
            Some(save_json(ID, t - mins(1), t + mins(30), true, 2)),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["revision"], 3);
        assert_eq!(controller.snapshot().kind, Some(SessionKind::Starred));
    }

    #[tokio::test]
    async fn an_hd_stop_does_not_touch_a_session_edited_since() {
        let h = harness(false, true).await;
        let controller = h.controller.clone().unwrap();
        let t = Utc::now();
        let (status, v) = call(
            &h.app,
            "POST",
            "/starred-sessions",
            Some(save_json(ID, t - mins(1), t + mins(30), true, 0)),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        controller.stop_session();
        // An edit lands (another client) before the stop is written back.
        sqlx::query("UPDATE starred_sessions SET revision = 5 WHERE id = ?1")
            .bind(ID)
            .execute(h.db.write_pool())
            .await
            .unwrap();
        sync_hd_bindings(&h.db, &controller).await.unwrap();
        let row = h.db.get_starred_session(ID).await.unwrap().unwrap();
        assert!(row.hd_requested, "the newer revision stands");
        assert_eq!(row.revision, 5);
        assert!(controller.take_stopped_starred_windows().is_empty());
        // ...and, being a new revision, it raises HD again.
        assert_eq!(controller.snapshot().kind, Some(SessionKind::Starred));
    }

    #[tokio::test]
    async fn the_sync_task_writes_an_hd_stop_without_waiting_for_its_tick() {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(
            DatabaseManager::new(
                dir.path().join("db.sqlite").to_str().unwrap(),
                Default::default(),
            )
            .await
            .unwrap(),
        );
        let t = Utc::now();
        db.save_starred_session(&StarredSessionWrite {
            id: ID,
            start: t - mins(1),
            end: t + mins(30),
            hd_requested: true,
            expected_revision: 0,
        })
        .await
        .unwrap();
        let controller = Arc::new(HighFpsController::new(None, DefaultMode::Ask, 100));
        // An hour between ticks: only the stop signal can wake it in time.
        let task = tokio::spawn(run_hd_sync(
            Arc::downgrade(&db),
            Arc::downgrade(&controller),
            Duration::from_secs(3600),
        ));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !controller.snapshot().active {
            assert!(
                tokio::time::Instant::now() < deadline,
                "first sync installs"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        controller.stop_session();
        loop {
            let row = db.get_starred_session(ID).await.unwrap().unwrap();
            if !row.hd_requested {
                assert_eq!(row.revision, 2);
                break;
            }
            assert!(tokio::time::Instant::now() < deadline, "stop written back");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!controller.snapshot().active);
        task.abort();
    }

    #[test]
    fn both_methods_are_documented() {
        let routes = documented();
        assert_eq!(routes.paths(), vec!["/starred-sessions".to_string()]);
    }
}
