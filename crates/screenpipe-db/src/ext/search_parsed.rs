// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Search v2 additions: parsed content, time-window filters and related tags.
//!
//! - **Parsed content** (`content_type=parsed`): one hit per frame whose
//!   `handled` semantic run matches, carrying the run's metadata and its items
//!   in run order with each item's assigned actor. Text queries go through
//!   `semantic_items_fts`; `actor_id` keeps frames holding an item assigned to
//!   that actor. The projection itself is written by `ext/semantic.rs`; this
//!   module only reads it, on the read pool.
//! - **Time windows** ([`crate::SearchWindows`]): the SQL membership filter the
//!   search queries add when `/search` is restricted to starred sessions.
//! - **Related tags**: for a tag filter, the other tags carried by the matching
//!   frames, audio chunks or memories, grouped by namespace.
//! - [`is_search_busy_error`] tells the route which failures mean "the
//!   database is saturated, retry" rather than "the search is broken".
//!
//! Every value reaches SQL as a bound parameter. The SQL text is assembled
//! only from constants in this crate.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};
use oasgen::OaSchema;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteArguments, SqliteRow};
use sqlx::Row;
use tracing::warn;

use crate::ext::semantic::{
    assigned_from_row, decode_source_node_ids, item_from_row, run_from_row, SemanticAppPlatform,
    SemanticRun, SemanticStoreError, ASSIGNED_COLUMNS, ITEM_COLUMNS, RUN_COLUMNS,
    SEMANTIC_CONTENT_HASH_LEN, SEMANTIC_FINGERPRINT_LEN,
};
use crate::text_normalizer::sanitize_fts5_query;
use crate::{DatabaseManager, SearchScope, SearchWindows};

/// Most tags listed per namespace in a related-tags map.
pub const RELATED_TAGS_PER_NAMESPACE: usize = 20;

/// Namespace for tags without a `namespace:` prefix in a related-tags map.
pub const RELATED_TAGS_PLAIN_NAMESPACE: &str = "other";

/// One frame's parsed content, as `content_type=parsed` returns it.
#[derive(OaSchema, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedFrameResult {
    pub frame_id: i64,
    pub timestamp: DateTime<Utc>,
    pub app_name: String,
    pub window_name: String,
    pub run_id: i64,
    pub parser_id: String,
    pub parser_version: String,
    /// `macos`, `windows` or `linux`.
    pub app_platform: String,
    pub app_id: Option<String>,
    pub app_executable: Option<String>,
    pub parse_duration_us: i64,
    pub text_bytes: i64,
    /// Items in run order. With an `actor_id` filter, only that actor's items
    /// and the items containing them.
    pub items: Vec<ParsedItemRecord>,
}

/// One item of a parsed frame, with its place in the run and its actor.
#[derive(OaSchema, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedItemRecord {
    /// `semantic_items.id`; stable across the runs that saw the same item.
    pub item_id: i64,
    /// Run-local id.
    pub local_id: String,
    pub parent_local_id: Option<String>,
    pub sort_order: i64,
    /// `conversation`, `message`, `document`, `task`, `calendar_event` or
    /// `page`.
    pub kind: String,
    pub item_key: String,
    /// `stable`, `derived` or `ephemeral`.
    pub identity_quality: String,
    pub title: Option<String>,
    pub body: Option<String>,
    /// The actor label the parser saw.
    pub actor: Option<String>,
    pub occurred_at: Option<String>,
    pub timestamp_precision: Option<String>,
    pub status: Option<String>,
    /// Tree node ids the item was read from; `None` when the stored blob is
    /// not in the encoding this build reads.
    pub source_nodes: Option<Vec<u32>>,
    pub assigned_actor_id: Option<i64>,
    /// Canonical name of the assigned actor.
    pub assigned_actor_name: Option<String>,
    /// `heuristic` or `reconciled`.
    pub assignment_source: Option<String>,
}

/// Filters for [`DatabaseManager::search_parsed`] and
/// [`DatabaseManager::count_parsed`]. `frame_id`, `actor_id` and the time
/// windows come from the [`SearchScope`].
#[derive(Debug, Clone, Copy)]
pub struct ParsedSearchFilter<'a> {
    /// Words matched against item title, body, actor, status and metadata.
    pub query: &'a str,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    /// Case-insensitive substring of the frame's app name.
    pub app_name: Option<&'a str>,
    /// Case-insensitive substring of the frame's window title.
    pub window_name: Option<&'a str>,
    /// Case-insensitive substring of the frame's browser URL.
    pub browser_url: Option<&'a str>,
    pub focused: Option<bool>,
    pub machine_id: Option<&'a str>,
    /// Frames must carry all of these tags.
    pub tags: &'a [String],
    pub scope: &'a SearchScope,
}

/// Which stores a related-tags map is computed over.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RelatedTagSources {
    pub frames: bool,
    /// Restrict frames to those with a parser run.
    pub parsed_frames_only: bool,
    pub audio: bool,
    pub memories: bool,
}

/// True when a search failed because the database or the read pool is
/// saturated (SQLite busy or locked, no free connection in time), so the
/// caller should retry later instead of treating it as a bad query.
pub fn is_search_busy_error(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::PoolTimedOut) || crate::sqlite_error::is_sqlite_busy_error(e)
}

/// SQL membership filter for search windows: `outer_id IN (...)` selecting the
/// rows of `table` whose `ts_col` lies in one of the windows bound at `param`
/// (the JSON from [`SearchWindows::to_sql_json`]).
///
/// The inner query walks the windows and range-scans `ts_col` (which should
/// be indexed) with the widened second-precision text bounds, then keeps a row
/// only when `julianday()` places it inside `[start, end)`. Its cost follows
/// the number of rows inside the windows, not the size of the table.
pub(crate) fn window_filter_sql(outer_id: &str, table: &str, ts_col: &str, param: &str) -> String {
    format!(
        "{outer_id} IN (SELECT sw_t.id FROM json_each({param}) AS sw \
         JOIN {table} AS sw_t \
           ON sw_t.{ts_col} >= json_extract(sw.value, '$[2]') \
          AND sw_t.{ts_col} < json_extract(sw.value, '$[3]') \
         WHERE julianday(sw_t.{ts_col}) >= julianday(json_extract(sw.value, '$[0]')) \
           AND julianday(sw_t.{ts_col}) < julianday(json_extract(sw.value, '$[1]')))"
    )
}

/// Like [`window_filter_sql`] for a table whose timestamp has an expression
/// index on `julianday(ts_col)` (`meeting_transcript_segments.captured_at`).
pub(crate) fn window_filter_julianday_sql(
    outer_id: &str,
    table: &str,
    ts_col: &str,
    param: &str,
) -> String {
    format!(
        "{outer_id} IN (SELECT sw_t.id FROM json_each({param}) AS sw \
         JOIN {table} AS sw_t \
           ON julianday(sw_t.{ts_col}) >= julianday(json_extract(sw.value, '$[0]')) \
          AND julianday(sw_t.{ts_col}) < julianday(json_extract(sw.value, '$[1]')))"
    )
}

/// `AND ...` window condition for a frames query whose frame id column is
/// `outer_id`, or nothing when `scope` has no windows. The windows JSON is
/// bound at `param`.
pub(crate) fn frames_window_condition(scope: &SearchScope, outer_id: &str, param: &str) -> String {
    match scope.windows {
        Some(_) => format!(
            "AND {}",
            window_filter_sql(outer_id, "frames", "timestamp", param)
        ),
        None => String::new(),
    }
}

/// SQL condition keeping `ui_events` rows that carry input context: typed or
/// copied text, or a named UI element.
pub(crate) const UI_EVENT_CONTEXT_CONDITION: &str =
    "(TRIM(COALESCE(text_content, '')) != '' OR TRIM(COALESCE(element_name, '')) != '')";

/// Append `scope`'s conditions for an unaliased `ui_events` query that binds
/// anonymous `?` parameters in the order of `bind_values`.
pub(crate) fn push_ui_event_scope_conditions(
    scope: &SearchScope,
    conditions: &mut Vec<String>,
    bind_values: &mut Vec<String>,
) {
    if scope.input_context_only {
        conditions.push(UI_EVENT_CONTEXT_CONDITION.to_string());
    }
    if let Some(windows) = &scope.windows {
        conditions.push(window_filter_sql("id", "ui_events", "timestamp", "?"));
        bind_values.push(windows.to_sql_json());
    }
}

/// `AND ...` window condition for `meeting_transcript_segments` (live meeting
/// transcripts), or nothing when `scope` has no windows. The windows JSON is
/// bound at `param`.
pub(crate) fn live_segment_window_condition(scope: &SearchScope, param: &str) -> String {
    match scope.windows {
        Some(_) => format!(
            "AND {}",
            window_filter_julianday_sql("id", "meeting_transcript_segments", "captured_at", param)
        ),
        None => String::new(),
    }
}

/// Compact text for a parsed frame: one line per item in run order, indented
/// two spaces per level of nesting. A line is `actor: title - body`, leaving
/// out the parts an item does not have (the assigned actor's name is
/// preferred over the label the parser saw). Whitespace inside a part
/// collapses to single spaces; items with nothing to show are skipped.
pub fn compact_parsed_text(items: &[ParsedItemRecord]) -> String {
    let parents: HashMap<&str, Option<&str>> = items
        .iter()
        .map(|i| (i.local_id.as_str(), i.parent_local_id.as_deref()))
        .collect();
    let depth_of = |item: &ParsedItemRecord| -> usize {
        let mut depth = 0;
        let mut seen: HashSet<&str> = HashSet::new();
        let mut cursor = item.parent_local_id.as_deref();
        while let Some(parent) = cursor {
            // A parent outside this list (narrowed items) or a loop ends
            // the walk.
            if !seen.insert(parent) {
                break;
            }
            match parents.get(parent) {
                Some(next) => {
                    depth += 1;
                    cursor = *next;
                }
                None => break,
            }
        }
        depth
    };

    let mut lines = Vec::with_capacity(items.len());
    for item in items {
        let text: Vec<String> = [item.title.as_deref(), item.body.as_deref()]
            .into_iter()
            .flatten()
            .map(collapse_whitespace)
            .filter(|s| !s.is_empty())
            .collect();
        if text.is_empty() {
            continue;
        }
        let who = item
            .assigned_actor_name
            .as_deref()
            .or(item.actor.as_deref())
            .map(collapse_whitespace)
            .filter(|s| !s.is_empty());
        let mut line = "  ".repeat(depth_of(item));
        if let Some(who) = who {
            line.push_str(&who);
            line.push_str(": ");
        }
        line.push_str(&text.join(" - "));
        lines.push(line);
    }
    lines.join("\n")
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Keep the items assigned to `actor_id` and every item containing one of
/// them, in their original order.
pub fn narrow_items_to_actor(items: Vec<ParsedItemRecord>, actor_id: i64) -> Vec<ParsedItemRecord> {
    let parents: HashMap<String, Option<String>> = items
        .iter()
        .map(|i| (i.local_id.clone(), i.parent_local_id.clone()))
        .collect();
    let mut keep: HashSet<String> = HashSet::new();
    for item in items
        .iter()
        .filter(|i| i.assigned_actor_id == Some(actor_id))
    {
        let mut cursor = Some(item.local_id.clone());
        while let Some(id) = cursor {
            if !keep.insert(id.clone()) {
                break;
            }
            cursor = parents.get(&id).cloned().flatten();
        }
    }
    items
        .into_iter()
        .filter(|i| keep.contains(&i.local_id))
        .collect()
}

/// Group `(tag, uses)` counts into namespace -> tags, most used first (then by
/// name), at most [`RELATED_TAGS_PER_NAMESPACE`] per namespace. The namespace
/// is the part before the first `:`; tags without one (or with an empty
/// prefix) go under [`RELATED_TAGS_PLAIN_NAMESPACE`]. Tags are listed in full
/// (`person:ada`), so they can be passed back as `tags=`.
pub fn group_related_tags(
    counts: impl IntoIterator<Item = (String, i64)>,
) -> BTreeMap<String, Vec<String>> {
    let mut totals: HashMap<String, i64> = HashMap::new();
    for (name, n) in counts {
        if name.trim().is_empty() || n <= 0 {
            continue;
        }
        *totals.entry(name).or_default() += n;
    }
    let mut by_namespace: BTreeMap<String, Vec<(String, i64)>> = BTreeMap::new();
    for (name, n) in totals {
        let namespace = match name.split_once(':') {
            Some((ns, rest)) if !ns.trim().is_empty() && !rest.is_empty() => ns.to_string(),
            _ => RELATED_TAGS_PLAIN_NAMESPACE.to_string(),
        };
        by_namespace.entry(namespace).or_default().push((name, n));
    }
    by_namespace
        .into_iter()
        .map(|(ns, mut tags)| {
            tags.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            tags.truncate(RELATED_TAGS_PER_NAMESPACE);
            (ns, tags.into_iter().map(|(name, _)| name).collect())
        })
        .collect()
}

/// A value bound to a numbered placeholder.
#[derive(Debug, Clone)]
enum Bind {
    Text(String),
    Int(i64),
    Time(DateTime<Utc>),
}

/// Collects bound values and hands out their `?N` placeholders.
#[derive(Debug, Default)]
struct Binds(Vec<Bind>);

type QueryAs<'q, O> = sqlx::query::QueryAs<'q, sqlx::Sqlite, O, SqliteArguments>;
type QueryScalar<'q, O> = sqlx::query::QueryScalar<'q, sqlx::Sqlite, O, SqliteArguments>;

impl Binds {
    fn push(&mut self, value: Bind) -> String {
        self.0.push(value);
        format!("?{}", self.0.len())
    }

    fn text(&mut self, value: impl Into<String>) -> String {
        self.push(Bind::Text(value.into()))
    }

    fn int(&mut self, value: i64) -> String {
        self.push(Bind::Int(value))
    }

    fn time(&mut self, value: DateTime<Utc>) -> String {
        self.push(Bind::Time(value))
    }

    fn apply<'q, O>(&self, mut q: QueryAs<'q, O>) -> QueryAs<'q, O> {
        for b in &self.0 {
            q = match b {
                Bind::Text(s) => q.bind(s.clone()),
                Bind::Int(n) => q.bind(*n),
                Bind::Time(t) => q.bind(*t),
            };
        }
        q
    }

    fn apply_scalar<'q, O>(&self, mut q: QueryScalar<'q, O>) -> QueryScalar<'q, O> {
        for b in &self.0 {
            q = match b {
                Bind::Text(s) => q.bind(s.clone()),
                Bind::Int(n) => q.bind(*n),
                Bind::Time(t) => q.bind(*t),
            };
        }
        q
    }
}

/// `%` and `_` (and the escape itself) taken literally in a `LIKE ... ESCAPE
/// '\'` pattern.
fn like_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Conditions on `r` that hold for every run [`run_from_row`] can read: the
/// enum and blob columns it rejects otherwise (values a newer writer may
/// store). With them in the shared WHERE, the count never includes a run the
/// page would have to skip, so `pagination.total` matches what the pages can
/// return. Built from this crate's constants only.
fn readable_run_conditions() -> [String; 3] {
    let platforms: Vec<String> = SemanticAppPlatform::ALL
        .iter()
        .map(|p| format!("'{}'", p.as_str()))
        .collect();
    [
        format!("r.app_platform IN ({})", platforms.join(", ")),
        format!("length(r.input_fingerprint) = {SEMANTIC_FINGERPRINT_LEN}"),
        format!("length(r.input_content_hash) = {SEMANTIC_CONTENT_HASH_LEN}"),
    ]
}

/// FROM/WHERE shared by the parsed page and count queries.
fn parsed_from_where(filter: &ParsedSearchFilter<'_>, binds: &mut Binds) -> String {
    let mut conditions: Vec<String> = vec![
        "f.semantic_run_id IS NOT NULL".to_string(),
        "r.status = 'handled'".to_string(),
    ];
    conditions.extend(readable_run_conditions());
    let scope = filter.scope;
    if let Some(frame_id) = scope.frame_id {
        conditions.push(format!("f.id = {}", binds.int(frame_id)));
    }
    if let Some(start) = filter.start_time {
        conditions.push(format!("f.timestamp >= {}", binds.time(start)));
    }
    if let Some(end) = filter.end_time {
        conditions.push(format!("f.timestamp <= {}", binds.time(end)));
    }
    for (column, value) in [
        ("f.app_name", filter.app_name),
        ("f.window_name", filter.window_name),
        ("f.browser_url", filter.browser_url),
    ] {
        if let Some(v) = value.map(str::trim).filter(|v| !v.is_empty()) {
            let p = binds.text(like_literal(v));
            conditions.push(format!("{column} LIKE '%' || {p} || '%' ESCAPE '\\'"));
        }
    }
    if let Some(focused) = filter.focused {
        conditions.push(format!("f.focused = {}", binds.int(i64::from(focused))));
    }
    if let Some(machine) = filter.machine_id {
        conditions.push(format!("f.machine_id = {}", binds.text(machine)));
    }
    if !filter.tags.is_empty() {
        let json = serde_json::to_string(filter.tags).unwrap_or_else(|_| "[]".to_string());
        let p = binds.text(json);
        conditions.push(format!(
            "f.id IN (SELECT vt.vision_id FROM vision_tags vt JOIN tags t ON vt.tag_id = t.id \
             WHERE t.name IN (SELECT value FROM json_each({p})) \
             GROUP BY vt.vision_id HAVING COUNT(DISTINCT t.name) = json_array_length({p}))"
        ));
    }

    // Item-level conditions: one item must satisfy both the text query and
    // the actor filter.
    let mut item_conditions: Vec<String> = Vec::new();
    let fts = sanitize_fts5_query(filter.query);
    if !fts.is_empty() {
        let p = binds.text(fts);
        item_conditions.push(format!(
            "ri.item_id IN (SELECT rowid FROM semantic_items_fts WHERE semantic_items_fts MATCH {p})"
        ));
    }
    if let Some(actor_id) = scope.actor_id {
        let p = binds.int(actor_id);
        item_conditions.push(format!(
            "ri.item_id IN (SELECT ia.item_id FROM semantic_item_actors ia WHERE ia.actor_id = {p})"
        ));
    }
    if !item_conditions.is_empty() {
        conditions.push(format!(
            "f.semantic_run_id IN (SELECT ri.run_id FROM semantic_run_items ri WHERE {})",
            item_conditions.join(" AND ")
        ));
    }

    if let Some(windows) = &scope.windows {
        let p = binds.text(windows.to_sql_json());
        conditions.push(window_filter_sql("f.id", "frames", "timestamp", &p));
    }

    // CROSS JOIN keeps `frames` as the outer loop (SQLite never reorders
    // it), so every plan starts from a frames index: the time range, the
    // window ids, the run ids the item filters select, or, when browsing
    // without filters, the non-null part of `idx_frames_semantic_run_id`.
    // Left to itself the planner prefers a full scan of `semantic_runs` for
    // that last case.
    format!(
        "FROM frames f CROSS JOIN semantic_runs r ON r.id = f.semantic_run_id WHERE {}",
        conditions.join(" AND ")
    )
}

fn store_error_to_sqlx(e: SemanticStoreError) -> sqlx::Error {
    match e {
        SemanticStoreError::Database(e) => e,
        other => sqlx::Error::Protocol(other.to_string()),
    }
}

fn record_from_row(row: &SqliteRow) -> Result<(i64, ParsedItemRecord), SemanticStoreError> {
    let item = item_from_row(row)?;
    let assigned = assigned_from_row(row)?;
    let blob: Vec<u8> = row.try_get("source_node_ids")?;
    Ok((
        row.try_get("item_run_id")?,
        ParsedItemRecord {
            item_id: item.id,
            local_id: row.try_get("local_id")?,
            parent_local_id: row.try_get("parent_local_id")?,
            sort_order: row.try_get("sort_order")?,
            kind: item.kind.as_str().to_string(),
            item_key: item.item_key,
            identity_quality: item.identity_quality.as_str().to_string(),
            title: item.title,
            body: item.body,
            actor: item.actor,
            occurred_at: item.occurred_at,
            timestamp_precision: item.timestamp_precision,
            status: item.status,
            source_nodes: decode_source_node_ids(&blob),
            assigned_actor_id: assigned.as_ref().map(|a| a.actor_id),
            assigned_actor_name: assigned.as_ref().map(|a| a.name.clone()),
            assignment_source: assigned.map(|a| a.assignment_source.as_str().to_string()),
        },
    ))
}

#[derive(Debug, sqlx::FromRow)]
struct ParsedFrameRow {
    frame_id: i64,
    timestamp: DateTime<Utc>,
    app_name: String,
    window_name: String,
    run_id: i64,
}

impl DatabaseManager {
    /// Frames with parsed content matching `filter`, newest first, with their
    /// runs and items. Runs this build cannot read (a value it does not know)
    /// are excluded in SQL, from this page and from [`Self::count_parsed`]
    /// alike; one that still fails to read is left out with a warning, and so
    /// is an unreadable item, so one odd row never fails the search.
    pub async fn search_parsed(
        &self,
        filter: &ParsedSearchFilter<'_>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<ParsedFrameResult>, sqlx::Error> {
        if filter.scope.matches_nothing() {
            return Ok(Vec::new());
        }
        let mut binds = Binds::default();
        let from_where = parsed_from_where(filter, &mut binds);
        let limit_p = binds.int(i64::from(limit));
        let offset_p = binds.int(i64::from(offset));
        let sql = format!(
            "SELECT f.id AS frame_id, f.timestamp AS timestamp, \
                    COALESCE(f.app_name, '') AS app_name, \
                    COALESCE(f.window_name, '') AS window_name, \
                    f.semantic_run_id AS run_id \
             {from_where} \
             ORDER BY f.timestamp DESC, f.id DESC \
             LIMIT {limit_p} OFFSET {offset_p}"
        );
        // Interrupted if the request that wants these results goes away.
        let mut reader = self.cancellable_read().await?;
        let frames: Vec<ParsedFrameRow> = binds
            .apply(sqlx::query_as(sqlx::AssertSqlSafe(sql)))
            .fetch_all(reader.conn())
            .await?;
        reader.finish();
        if frames.is_empty() {
            return Ok(Vec::new());
        }

        let mut run_ids: Vec<i64> = frames.iter().map(|f| f.run_id).collect();
        run_ids.sort_unstable();
        run_ids.dedup();
        let (runs, items) = self.load_parsed_runs(&run_ids).await?;

        let mut out = Vec::with_capacity(frames.len());
        for frame in frames {
            let Some(run) = runs.get(&frame.run_id) else {
                continue;
            };
            let mut run_items = items.get(&frame.run_id).cloned().unwrap_or_default();
            if let Some(actor_id) = filter.scope.actor_id {
                run_items = narrow_items_to_actor(run_items, actor_id);
            }
            out.push(ParsedFrameResult {
                frame_id: frame.frame_id,
                timestamp: frame.timestamp,
                app_name: frame.app_name,
                window_name: frame.window_name,
                run_id: frame.run_id,
                parser_id: run.parser_id.clone(),
                parser_version: run.parser_version.clone(),
                app_platform: run.app_platform.as_str().to_string(),
                app_id: run.app_id.clone(),
                app_executable: run.app_executable.clone(),
                parse_duration_us: run.parse_duration_us,
                text_bytes: run.text_bytes,
                items: run_items,
            });
        }
        Ok(out)
    }

    /// Number of frames [`Self::search_parsed`] can return for `filter`.
    pub async fn count_parsed(
        &self,
        filter: &ParsedSearchFilter<'_>,
    ) -> Result<usize, sqlx::Error> {
        if filter.scope.matches_nothing() {
            return Ok(0);
        }
        let mut binds = Binds::default();
        let from_where = parsed_from_where(filter, &mut binds);
        let sql = format!("SELECT COUNT(*) {from_where}");
        let mut reader = self.cancellable_read().await?;
        let n: i64 = binds
            .apply_scalar(sqlx::query_scalar(sqlx::AssertSqlSafe(sql)))
            .fetch_one(reader.conn())
            .await?;
        reader.finish();
        Ok(n.max(0) as usize)
    }

    /// Runs and their items (run order, with assigned actors) for `run_ids`.
    async fn load_parsed_runs(
        &self,
        run_ids: &[i64],
    ) -> Result<
        (
            HashMap<i64, SemanticRun>,
            HashMap<i64, Vec<ParsedItemRecord>>,
        ),
        sqlx::Error,
    > {
        let ids_json = serde_json::to_string(run_ids).unwrap_or_else(|_| "[]".to_string());
        let mut conn = self.pool.acquire().await?;

        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM semantic_runs r \
             WHERE r.id IN (SELECT value FROM json_each(?1))"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(&ids_json)
            .fetch_all(&mut *conn)
            .await?;
        let mut runs = HashMap::with_capacity(rows.len());
        for row in &rows {
            match run_from_row(row) {
                Ok(run) => {
                    runs.insert(run.id, run);
                }
                Err(SemanticStoreError::InvalidStoredValue { column, value }) => {
                    warn!("parsed search: skipping a run with unreadable {column} {value:?}");
                }
                Err(e) => return Err(store_error_to_sqlx(e)),
            }
        }

        let sql = format!(
            "SELECT ri.run_id AS item_run_id, ri.local_id, ri.parent_local_id, ri.sort_order, \
                    ri.source_node_ids, {ITEM_COLUMNS}, {ASSIGNED_COLUMNS} \
             FROM semantic_run_items ri \
             JOIN semantic_items i ON i.id = ri.item_id \
             LEFT JOIN semantic_item_actors ia ON ia.item_id = i.id \
             LEFT JOIN semantic_actors a ON a.id = ia.actor_id \
             WHERE ri.run_id IN (SELECT value FROM json_each(?1)) \
             ORDER BY ri.run_id, ri.sort_order"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(&ids_json)
            .fetch_all(&mut *conn)
            .await?;
        let mut items: HashMap<i64, Vec<ParsedItemRecord>> = HashMap::new();
        for row in &rows {
            match record_from_row(row) {
                Ok((run_id, item)) => items.entry(run_id).or_default().push(item),
                Err(SemanticStoreError::InvalidStoredValue { column, value }) => {
                    warn!("parsed search: skipping an item with unreadable {column} {value:?}");
                }
                Err(e) => return Err(store_error_to_sqlx(e)),
            }
        }
        Ok((runs, items))
    }

    /// Tags that co-occur with `tags` (all of them) on the frames, audio
    /// chunks or memories selected by `sources`, inside the time range and
    /// windows, grouped by namespace (see [`group_related_tags`]). The
    /// requested tags themselves are not listed. Empty `tags` gives an empty
    /// map.
    pub async fn related_search_tags(
        &self,
        tags: &[String],
        sources: RelatedTagSources,
        start_time: Option<DateTime<Utc>>,
        end_time: Option<DateTime<Utc>>,
        windows: Option<&SearchWindows>,
    ) -> Result<BTreeMap<String, Vec<String>>, sqlx::Error> {
        if tags.is_empty() || windows.is_some_and(SearchWindows::is_empty) {
            return Ok(BTreeMap::new());
        }
        let tags_json = serde_json::to_string(tags).unwrap_or_else(|_| "[]".to_string());
        let mut counts: Vec<(String, i64)> = Vec::new();

        if sources.frames {
            let mut binds = Binds::default();
            let p = binds.text(tags_json.clone());
            let mut conditions = vec![format!("t.name NOT IN (SELECT value FROM json_each({p}))")];
            if let Some(start) = start_time {
                conditions.push(format!("f.timestamp >= {}", binds.time(start)));
            }
            if let Some(end) = end_time {
                conditions.push(format!("f.timestamp <= {}", binds.time(end)));
            }
            if sources.parsed_frames_only {
                conditions.push("f.semantic_run_id IS NOT NULL".to_string());
            }
            if let Some(w) = windows {
                let wp = binds.text(w.to_sql_json());
                conditions.push(window_filter_sql("f.id", "frames", "timestamp", &wp));
            }
            let sql = format!(
                "WITH hit AS ( \
                     SELECT vt2.vision_id AS id FROM vision_tags vt2 \
                     JOIN tags t2 ON t2.id = vt2.tag_id \
                     WHERE t2.name IN (SELECT value FROM json_each({p})) \
                     GROUP BY vt2.vision_id \
                     HAVING COUNT(DISTINCT t2.name) = json_array_length({p})) \
                 SELECT t.name, COUNT(DISTINCT hit.id) FROM hit \
                 JOIN frames f ON f.id = hit.id \
                 JOIN vision_tags vt ON vt.vision_id = hit.id \
                 JOIN tags t ON t.id = vt.tag_id \
                 WHERE {} GROUP BY t.name",
                conditions.join(" AND ")
            );
            let rows: Vec<(String, i64)> = binds
                .apply(sqlx::query_as(sqlx::AssertSqlSafe(sql)))
                .fetch_all(&self.pool)
                .await?;
            counts.extend(rows);
        }

        if sources.audio {
            let mut binds = Binds::default();
            let p = binds.text(tags_json.clone());
            let mut conditions = vec![format!("t.name NOT IN (SELECT value FROM json_each({p}))")];
            if let Some(start) = start_time {
                conditions.push(format!("c.timestamp >= {}", binds.time(start)));
            }
            if let Some(end) = end_time {
                conditions.push(format!("c.timestamp <= {}", binds.time(end)));
            }
            if let Some(w) = windows {
                let wp = binds.text(w.to_sql_json());
                conditions.push(window_filter_sql("c.id", "audio_chunks", "timestamp", &wp));
            }
            let sql = format!(
                "WITH hit AS ( \
                     SELECT at2.audio_chunk_id AS id FROM audio_tags at2 \
                     JOIN tags t2 ON t2.id = at2.tag_id \
                     WHERE t2.name IN (SELECT value FROM json_each({p})) \
                     GROUP BY at2.audio_chunk_id \
                     HAVING COUNT(DISTINCT t2.name) = json_array_length({p})) \
                 SELECT t.name, COUNT(DISTINCT hit.id) FROM hit \
                 JOIN audio_chunks c ON c.id = hit.id \
                 JOIN audio_tags atg ON atg.audio_chunk_id = hit.id \
                 JOIN tags t ON t.id = atg.tag_id \
                 WHERE {} GROUP BY t.name",
                conditions.join(" AND ")
            );
            let rows: Vec<(String, i64)> = binds
                .apply(sqlx::query_as(sqlx::AssertSqlSafe(sql)))
                .fetch_all(&self.pool)
                .await?;
            counts.extend(rows);
        }

        // Memories have no capture time, so a window restriction excludes
        // them (as it does from the search itself).
        if sources.memories && windows.is_none() {
            let mut binds = Binds::default();
            let p = binds.text(tags_json.clone());
            let mut conditions = vec![
                "je.type = 'text'".to_string(),
                format!("je.value NOT IN (SELECT value FROM json_each({p}))"),
                format!(
                    "(SELECT COUNT(DISTINCT x.value) FROM json_each(m.safe_tags) x \
                      WHERE x.value IN (SELECT value FROM json_each({p}))) = json_array_length({p})"
                ),
            ];
            if let Some(start) = start_time {
                let s = binds.text(start.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string());
                conditions.push(format!("m.created_at >= {s}"));
            }
            if let Some(end) = end_time {
                let e = binds.text(end.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string());
                conditions.push(format!("m.created_at <= {e}"));
            }
            let sql = format!(
                "SELECT je.value, COUNT(DISTINCT m.id) FROM ( \
                     SELECT id, created_at, \
                            CASE WHEN json_valid(tags) AND json_type(tags) = 'array' \
                                 THEN tags ELSE '[]' END AS safe_tags \
                     FROM memories) m, json_each(m.safe_tags) je \
                 WHERE {} GROUP BY je.value",
                conditions.join(" AND ")
            );
            let rows: Vec<(String, i64)> = binds
                .apply(sqlx::query_as(sqlx::AssertSqlSafe(sql)))
                .fetch_all(&self.pool)
                .await?;
            counts.extend(rows);
        }

        Ok(group_related_tags(counts))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(local: &str, parent: Option<&str>) -> ParsedItemRecord {
        ParsedItemRecord {
            item_id: 1,
            local_id: local.to_string(),
            parent_local_id: parent.map(str::to_string),
            sort_order: 0,
            kind: "message".to_string(),
            item_key: format!("key-{local}"),
            identity_quality: "stable".to_string(),
            title: None,
            body: None,
            actor: None,
            occurred_at: None,
            timestamp_precision: None,
            status: None,
            source_nodes: Some(vec![]),
            assigned_actor_id: None,
            assigned_actor_name: None,
            assignment_source: None,
        }
    }

    #[test]
    fn compact_text_indents_and_prefers_assigned_names() {
        let mut chat = item("c", None);
        chat.kind = "conversation".to_string();
        chat.title = Some("Design  review".to_string());
        let mut m1 = item("m1", Some("c"));
        m1.actor = Some("ada".to_string());
        m1.assigned_actor_name = Some("Ada Lovelace".to_string());
        m1.body = Some("ship it\non friday".to_string());
        let mut m2 = item("m2", Some("c"));
        m2.actor = Some("bob".to_string());
        m2.title = Some("Re: plan".to_string());
        m2.body = Some("agreed".to_string());
        let empty = item("e", Some("c"));
        let text = compact_parsed_text(&[chat, m1, m2, empty]);
        assert_eq!(
            text,
            "Design review\n  Ada Lovelace: ship it on friday\n  bob: Re: plan - agreed"
        );
    }

    #[test]
    fn compact_text_survives_parent_loops_and_missing_parents() {
        let mut a = item("a", Some("b"));
        a.body = Some("one".to_string());
        let mut b = item("b", Some("a"));
        b.body = Some("two".to_string());
        let mut orphan = item("o", Some("gone"));
        orphan.body = Some("three".to_string());
        let text = compact_parsed_text(&[a, b, orphan]);
        assert_eq!(text.lines().count(), 3);
        assert!(text.ends_with("three"), "{text}");
    }

    #[test]
    fn narrowing_keeps_actor_items_and_their_containers() {
        let chat = item("c", None);
        let mut thread = item("t", Some("c"));
        thread.kind = "conversation".to_string();
        let mut mine = item("m1", Some("t"));
        mine.assigned_actor_id = Some(7);
        let mut other = item("m2", Some("t"));
        other.assigned_actor_id = Some(8);
        let unrelated = item("d", None);
        let kept: Vec<String> =
            narrow_items_to_actor(vec![chat, thread, mine, other, unrelated], 7)
                .into_iter()
                .map(|i| i.local_id)
                .collect();
        assert_eq!(kept, vec!["c", "t", "m1"]);
    }

    #[test]
    fn related_tags_group_by_namespace_and_frequency() {
        let map = group_related_tags([
            ("person:bob".to_string(), 2),
            ("person:ada".to_string(), 5),
            ("project:atlas".to_string(), 1),
            ("urgent".to_string(), 3),
            (":odd".to_string(), 1),
            ("person:bob".to_string(), 4),
            ("".to_string(), 9),
        ]);
        assert_eq!(map["person"], vec!["person:bob", "person:ada"]);
        assert_eq!(map["project"], vec!["project:atlas"]);
        assert_eq!(map[RELATED_TAGS_PLAIN_NAMESPACE], vec!["urgent", ":odd"]);
        assert_eq!(map.len(), 3);
    }

    #[test]
    fn related_tags_cap_each_namespace() {
        let map = group_related_tags(
            (0..RELATED_TAGS_PER_NAMESPACE + 5).map(|i| (format!("person:p{i:02}"), 1)),
        );
        assert_eq!(map["person"].len(), RELATED_TAGS_PER_NAMESPACE);
        assert_eq!(map["person"][0], "person:p00");
    }

    #[test]
    fn like_patterns_take_wildcards_literally() {
        assert_eq!(like_literal("50%_off\\"), "50\\%\\_off\\\\");
    }

    #[test]
    fn window_filter_sql_names_its_columns() {
        let sql = window_filter_sql("f.id", "frames", "timestamp", "?3");
        assert!(sql.starts_with("f.id IN (SELECT sw_t.id FROM json_each(?3)"));
        assert!(sql.contains("JOIN frames AS sw_t"));
        assert!(sql.contains("julianday(sw_t.timestamp) < julianday"));
        let sql =
            window_filter_julianday_sql("id", "meeting_transcript_segments", "captured_at", "?");
        assert!(sql.contains("ON julianday(sw_t.captured_at) >="));
    }

    // ── query plans on a database with realistic row counts ─────────────

    /// 30,000 frames (one every 2 s), 30,000 input events, 5,000 audio
    /// transcriptions, 5,000 live meeting segments and 3,000 parser runs with
    /// 9,000 items and 50 actors, then `ANALYZE` as the maintenance task does.
    async fn seeded_db() -> DatabaseManager {
        let db = DatabaseManager::new("sqlite::memory:", Default::default())
            .await
            .unwrap();
        let seq = |n: u32| {
            format!(
                "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < {n} - 1)"
            )
        };
        let at = |step: &str| {
            format!(
                "strftime('%Y-%m-%dT%H:%M:%S', '2026-10-01 00:00:00', '+' || ({step}) || ' seconds') \
                 || '.000+00:00'"
            )
        };
        let statements = vec![
            format!(
                "{} INSERT INTO frames (timestamp, app_name, window_name, device_name, full_text) \
                 SELECT {}, 'App' || (i % 7), 'Window ' || (i % 50), 'monitor_1', 'text ' || i FROM n",
                seq(30_000),
                at("i * 2")
            ),
            format!(
                "{} INSERT INTO ui_events (timestamp, event_type, text_content, app_name) \
                 SELECT {}, 'text', 'typed ' || i, 'App' || (i % 7) FROM n",
                seq(30_000),
                at("i * 2")
            ),
            format!(
                "{} INSERT INTO audio_chunks (file_path, timestamp) SELECT 'a' || i || '.mp4', {} FROM n",
                seq(5_000),
                at("i * 12")
            ),
            format!(
                "{} INSERT INTO audio_transcriptions (audio_chunk_id, offset_index, timestamp, \
                 transcription, transcription_engine, device, is_input_device) \
                 SELECT i + 1, 0, {}, 'spoken ' || i, 'test', 'mic', 1 FROM n",
                seq(5_000),
                at("i * 12")
            ),
            "INSERT INTO meetings (meeting_start, meeting_end, meeting_app) \
             VALUES ('2026-10-01T00:00:00Z', '2026-10-01T20:00:00Z', 'zoom')"
                .to_string(),
            format!(
                "{} INSERT INTO meeting_transcript_segments (meeting_id, provider, item_id, \
                 transcript, captured_at) SELECT 1, 'test', 'item' || i, 'live ' || i, {} FROM n",
                seq(5_000),
                at("i * 12")
            ),
            format!(
                "{} INSERT INTO semantic_actors (name) SELECT 'Actor ' || i FROM n",
                seq(50)
            ),
            format!(
                "{} INSERT INTO semantic_runs (input_fingerprint, parser_id, parser_version, \
                 schema_version, app_platform, input_content_hash, status, item_count) \
                 SELECT randomblob(32), 'chat.test', '1', 1, 'windows', randomblob(8), 'handled', 3 \
                 FROM n",
                seq(3_000)
            ),
            format!(
                "{} INSERT INTO semantic_items (entity_fingerprint, version_fingerprint, kind, \
                 item_key, identity_quality, body, actor) \
                 SELECT randomblob(32), randomblob(32), 'message', 'k' || i, 'derived', \
                        'body word' || i, 'actor' || (i % 50) FROM n",
                seq(9_000)
            ),
            format!(
                "{} INSERT INTO semantic_run_items (run_id, local_id, item_id, sort_order) \
                 SELECT (i / 3) + 1, 'l' || (i % 3), i + 1, i % 3 FROM n",
                seq(9_000)
            ),
            format!(
                "{} INSERT INTO semantic_item_actors (item_id, actor_id) SELECT i + 1, (i % 50) + 1 FROM n",
                seq(9_000)
            ),
            "UPDATE frames SET semantic_run_id = id / 10 WHERE id % 10 = 0 AND id / 10 <= 3000"
                .to_string(),
            "ANALYZE".to_string(),
        ];
        for sql in statements {
            sqlx::query(sqlx::AssertSqlSafe(sql.clone()))
                .execute(db.write_pool())
                .await
                .unwrap_or_else(|e| panic!("{sql}: {e}"));
        }
        db
    }

    async fn plan(db: &DatabaseManager, sql: &str) -> Vec<String> {
        let rows: Vec<(i64, i64, i64, String)> =
            sqlx::query_as(sqlx::AssertSqlSafe(format!("EXPLAIN QUERY PLAN {sql}")))
                .fetch_all(&db.pool)
                .await
                .unwrap_or_else(|e| panic!("{sql}: {e}"));
        rows.into_iter().map(|r| r.3).collect()
    }

    fn assert_plan(plan: &[String], must_contain: &[&str], must_not_contain: &[&str]) {
        let text = plan.join("\n");
        for needle in must_contain {
            assert!(
                plan.iter().any(|line| line.contains(needle)),
                "plan lacks `{needle}`:\n{text}"
            );
        }
        for needle in must_not_contain {
            assert!(
                !plan.iter().any(|line| line.contains(needle)),
                "plan has `{needle}`:\n{text}"
            );
        }
    }

    /// `SCAN <alias>` without `USING` is a full table scan.
    fn full_scans(plan: &[String]) -> Vec<String> {
        plan.iter()
            .filter(|l| {
                let l = l.trim();
                l.starts_with("SCAN ") && !l.contains("USING") && !l.contains("VIRTUAL TABLE")
                    && !l.starts_with("SCAN sw") // json_each over the windows
                    && !l.starts_with("SCAN json_each")
                    && !l.contains("CONSTANT ROW")
            })
            .cloned()
            .collect()
    }

    #[tokio::test]
    async fn window_filters_range_scan_the_timestamp_indexes() {
        let db = seeded_db().await;
        for (table, index, sql) in [
            (
                "frames",
                "idx_frames_timestamp",
                format!(
                    "SELECT frames.id FROM frames WHERE frames.timestamp >= ?1 \
                     AND frames.timestamp <= ?2 AND {} \
                     ORDER BY frames.timestamp DESC LIMIT 20",
                    window_filter_sql("frames.id", "frames", "timestamp", "?3")
                ),
            ),
            (
                "ui_events",
                "idx_ui_events_timestamp",
                format!(
                    "SELECT id FROM ui_events WHERE {} ORDER BY timestamp DESC LIMIT 20",
                    window_filter_sql("id", "ui_events", "timestamp", "?1")
                ),
            ),
            (
                "audio_transcriptions",
                "idx_audio_transcriptions_timestamp",
                format!(
                    "SELECT audio_transcriptions.id FROM audio_transcriptions WHERE {} \
                     ORDER BY audio_transcriptions.timestamp DESC LIMIT 20",
                    window_filter_sql(
                        "audio_transcriptions.id",
                        "audio_transcriptions",
                        "timestamp",
                        "?1"
                    )
                ),
            ),
            (
                "meeting_transcript_segments",
                "idx_meeting_transcript_segments_captured_at_julianday",
                format!(
                    "SELECT id FROM meeting_transcript_segments WHERE 1=1 {} LIMIT 20",
                    live_segment_window_condition(
                        &SearchScope {
                            windows: Some(SearchWindows::default()),
                            ..Default::default()
                        },
                        "?1"
                    )
                ),
            ),
        ] {
            let p = plan(&db, &sql).await;
            println!("{table}:\n{}", p.join("\n"));
            assert_plan(&p, &["SEARCH sw_t USING", index], &["SCAN sw_t"]);
            assert!(full_scans(&p).is_empty(), "{table}: {p:?}");
        }
    }

    #[tokio::test]
    async fn parsed_queries_start_from_their_indexes() {
        let db = seeded_db().await;
        let scope_none = SearchScope::default();
        let actor = SearchScope {
            actor_id: Some(3),
            ..Default::default()
        };
        let windows = SearchScope {
            windows: Some(SearchWindows::default()),
            ..Default::default()
        };
        let base = |scope| ParsedSearchFilter {
            query: "",
            start_time: None,
            end_time: None,
            app_name: None,
            window_name: None,
            browser_url: None,
            focused: None,
            machine_id: None,
            tags: &[],
            scope,
        };
        let t0 = Utc::now();
        let cases: Vec<(&str, ParsedSearchFilter<'_>, Vec<&str>)> = vec![
            (
                "text",
                ParsedSearchFilter {
                    query: "word5",
                    ..base(&scope_none)
                },
                vec![
                    "semantic_items_fts VIRTUAL TABLE",
                    "idx_frames_semantic_run_id",
                ],
            ),
            (
                "actor",
                base(&actor),
                vec![
                    "idx_semantic_item_actors_actor",
                    "idx_frames_semantic_run_id",
                ],
            ),
            (
                "time range",
                ParsedSearchFilter {
                    start_time: Some(t0),
                    end_time: Some(t0),
                    ..base(&scope_none)
                },
                vec!["SEARCH f USING INDEX idx_frames_timestamp"],
            ),
            ("windows", base(&windows), vec!["SEARCH sw_t USING"]),
            (
                "browse",
                base(&scope_none),
                vec!["SEARCH f USING", "idx_frames_semantic_run_id"],
            ),
        ];
        for (name, filter, wanted) in cases {
            let mut binds = Binds::default();
            let from_where = parsed_from_where(&filter, &mut binds);
            for sql in [
                format!("SELECT f.id {from_where} ORDER BY f.timestamp DESC, f.id DESC LIMIT 20"),
                format!("SELECT COUNT(*) {from_where}"),
            ] {
                let p = plan(&db, &sql).await;
                println!("{name}:\n{}", p.join("\n"));
                assert_plan(&p, &wanted, &[]);
                assert!(full_scans(&p).is_empty(), "{name}: {p:?}");
            }
        }
    }

    #[test]
    fn pool_timeouts_count_as_busy() {
        assert!(is_search_busy_error(&sqlx::Error::PoolTimedOut));
        assert!(!is_search_busy_error(&sqlx::Error::RowNotFound));
    }
}
