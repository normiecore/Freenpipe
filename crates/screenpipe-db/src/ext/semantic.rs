// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Semantic projection storage: runs, items, actors and their full-text index.
//!
//! The tables come from the v2.7.104 migrations `20260724000000` and
//! `20260727000000`. This module is the only code that writes them.
//!
//! # Model
//!
//! * A **run** (`semantic_runs`) is one immutable parse of one parser input. Runs
//!   are keyed by a 32-byte input fingerprint, so frames whose input is the same
//!   share a run. A frame points at its run through `frames.semantic_run_id`.
//! * An **item** (`semantic_items`) is a canonical piece of structured content
//!   (a conversation, message, document, task, calendar event or page). Items
//!   are content addressed by a 32-byte version fingerprint and grouped into
//!   logical entities by an entity fingerprint, so one item row serves every run
//!   that saw the same content.
//! * A **run item** (`semantic_run_items`) places an item inside a run: its
//!   run-local id, the run-local parent, its position and the tree nodes it was
//!   read from.
//! * `semantic_items_fts` indexes item text. Triggers on `semantic_items` keep
//!   it in step, so this module never writes it.
//! * **Actors** are the people behind items. `semantic_items.actor` keeps the
//!   label the parser saw and never changes. `semantic_item_actors` holds the
//!   identity assigned to the item, either by the store-time heuristic or by an
//!   explicit ("reconciled") correction. Aliases map a source's labels to an
//!   actor and are kept when items age out, so corrections keep applying to
//!   future captures.
//!
//! # API at a glance
//!
//! | Purpose | Method |
//! |---|---|
//! | cache lookup before parsing | [`DatabaseManager::find_semantic_run_by_fingerprint`] |
//! | store a parse (run, items, run items, heuristic actors) | [`DatabaseManager::store_semantic_run`] |
//! | point frames at runs, batched | [`DatabaseManager::attach_semantic_runs_to_frames`], [`SemanticFrameAttachmentBuffer`] |
//! | read a run with its items | [`DatabaseManager::get_semantic_run_detail`], [`DatabaseManager::get_semantic_run_for_frame`] |
//! | full-text search | [`DatabaseManager::search_semantic_items`], [`DatabaseManager::search_semantic_frames`] |
//! | actors | `search_`, `get_`, `create_`, `rename_semantic_actor`, `merge_semantic_actors`, `reassign_semantic_item_actor`, `reassign_semantic_actor_alias`, `find_semantic_actor_alias` |
//! | retention | [`DatabaseManager::gc_semantic_projection`] |
//!
//! Every method returns [`SemanticStoreError`], whose variants tell an HTTP
//! layer whether a failure is the caller's (400), a missing row (404) or the
//! store's own (500). Write methods run in one `BEGIN IMMEDIATE` transaction
//! each and roll back on any error.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::time::{Duration, Instant};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize, Serializer};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqliteConnection};
use tracing::{debug, info, warn};

use crate::text_normalizer::sanitize_fts5_query;
use crate::{DatabaseManager, ImmediateTx};

/// Length of every semantic fingerprint (input, entity and version), in bytes.
pub const SEMANTIC_FINGERPRINT_LEN: usize = 32;
/// Length of `semantic_runs.input_content_hash`, in bytes.
pub const SEMANTIC_CONTENT_HASH_LEN: usize = 8;
/// A 32-byte semantic fingerprint.
pub type SemanticFingerprint = [u8; SEMANTIC_FINGERPRINT_LEN];

/// Upper bound on actor names and observed alias labels (characters).
pub const SEMANTIC_ACTOR_NAME_MAX_CHARS: usize = 256;
/// Upper bound on `semantic_actor_aliases.source_key` (characters).
pub const SEMANTIC_SOURCE_KEY_MAX_CHARS: usize = 1024;
/// Upper bound on `semantic_actor_aliases.normalized_label` (characters).
pub const SEMANTIC_NORMALIZED_LABEL_MAX_CHARS: usize = 512;
/// Upper bound on the actor search text (characters).
pub const SEMANTIC_ACTOR_QUERY_MAX_CHARS: usize = 256;
/// Largest page any list or search method returns.
pub const SEMANTIC_MAX_PAGE: u32 = 1000;
/// Default grace period for [`DatabaseManager::gc_semantic_projection`]. Runs
/// younger than this are kept even when no frame points at them yet, because
/// the worker attaches frames in batches after it stores the run. For
/// periodic retention passes only; user-initiated deletes pass
/// `Duration::ZERO`.
pub const SEMANTIC_GC_DEFAULT_GRACE: Duration = Duration::from_secs(10 * 60);

/// Rows deleted per GC transaction, so retention never holds the write lock
/// for long.
const GC_BATCH: i64 = 2000;
/// Ids per `IN (...)` list.
const IN_CHUNK: usize = 500;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Error from the semantic store.
#[derive(Debug)]
pub enum SemanticStoreError {
    /// The request is malformed (HTTP 400). The message names the field.
    InvalidInput(String),
    /// A referenced run, item, actor or alias does not exist (HTTP 404). The
    /// message names what is missing.
    NotFound(String),
    /// A row with the same fingerprint already exists with metadata that does
    /// not match the new write (a fingerprint collision or a caller bug).
    Conflict(String),
    /// A stored value is not one this build understands, for example a
    /// platform or kind added by a newer writer.
    InvalidStoredValue { column: &'static str, value: String },
    /// SQLite failed.
    Database(sqlx::Error),
}

impl SemanticStoreError {
    fn invalid(msg: impl Into<String>) -> Self {
        Self::InvalidInput(msg.into())
    }

    fn not_found(what: impl Into<String>) -> Self {
        Self::NotFound(what.into())
    }

    /// True for errors caused by the request ([`Self::InvalidInput`]).
    pub fn is_invalid_input(&self) -> bool {
        matches!(self, Self::InvalidInput(_))
    }

    /// True when a referenced row is missing ([`Self::NotFound`]).
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound(_))
    }
}

impl fmt::Display for SemanticStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(msg) => write!(f, "invalid semantic store input: {msg}"),
            Self::NotFound(what) => write!(f, "{what} not found"),
            Self::Conflict(msg) => write!(f, "semantic store conflict: {msg}"),
            Self::InvalidStoredValue { column, value } => {
                write!(f, "unrecognised stored value {value:?} in {column}")
            }
            Self::Database(e) => write!(f, "semantic store database error: {e}"),
        }
    }
}

impl std::error::Error for SemanticStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(e) => Some(e),
            _ => None,
        }
    }
}

impl From<sqlx::Error> for SemanticStoreError {
    fn from(e: sqlx::Error) -> Self {
        Self::Database(e)
    }
}

/// Result type of the semantic store.
pub type SemanticResult<T> = std::result::Result<T, SemanticStoreError>;
type Result<T> = SemanticResult<T>;

// ---------------------------------------------------------------------------
// Closed value sets (stored as TEXT, CHECK-constrained by the schema)
// ---------------------------------------------------------------------------

macro_rules! text_enum {
    (
        $(#[$meta:meta])*
        $name:ident, $column:literal { $($variant:ident => $text:literal),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name {
            $(
                #[serde(rename = $text)]
                $variant,
            )+
        }

        impl $name {
            /// Every value, in schema order.
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            /// The text stored in the database.
            pub fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $text,)+
                }
            }

            /// Parse a stored value. Unknown text is an error rather than a
            /// silent default, so data from a newer writer is never misread.
            pub fn from_stored(value: &str) -> Result<Self> {
                match value {
                    $($text => Ok($name::$variant),)+
                    other => Err(SemanticStoreError::InvalidStoredValue {
                        column: $column,
                        value: other.to_string(),
                    }),
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl std::str::FromStr for $name {
            type Err = SemanticStoreError;

            fn from_str(s: &str) -> Result<Self> {
                Self::from_stored(s)
            }
        }
    };
}

text_enum! {
    /// `semantic_runs.app_platform`.
    SemanticAppPlatform, "semantic_runs.app_platform" {
        Macos => "macos",
        Windows => "windows",
        Linux => "linux",
    }
}

impl SemanticAppPlatform {
    /// The platform this binary was built for.
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

text_enum! {
    /// `semantic_runs.status`: whether the parser produced items.
    SemanticRunStatus, "semantic_runs.status" {
        Handled => "handled",
        Empty => "empty",
    }
}

text_enum! {
    /// `semantic_items.kind`.
    SemanticItemKind, "semantic_items.kind" {
        Conversation => "conversation",
        Message => "message",
        Document => "document",
        Task => "task",
        CalendarEvent => "calendar_event",
        Page => "page",
    }
}

text_enum! {
    /// `semantic_items.identity_quality`: how far `item_key` can be trusted to
    /// name the same entity across captures.
    SemanticIdentityQuality, "semantic_items.identity_quality" {
        Stable => "stable",
        Derived => "derived",
        Ephemeral => "ephemeral",
    }
}

text_enum! {
    /// `semantic_item_actors.assignment_source`.
    SemanticAssignmentSource, "semantic_item_actors.assignment_source" {
        Heuristic => "heuristic",
        Reconciled => "reconciled",
    }
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// Pack tree node ids into the `semantic_run_items.source_node_ids` blob:
/// each id as 4 little-endian bytes, in order.
///
/// This layout is inferred, not confirmed: the evidence only calls the
/// column a packed blob of node ids. Until W2-06 or REA confirms screenpipe's
/// layout (open question 2 in `docs/clean-room/open-questions/W2-05.md`),
/// treat blobs written by another writer as opaque.
pub fn encode_source_node_ids(ids: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(ids.len() * 4);
    for id in ids {
        out.extend_from_slice(&id.to_le_bytes());
    }
    out
}

/// Unpack a `source_node_ids` blob written by [`encode_source_node_ids`].
/// An empty blob is an empty list.
///
/// Returns `None` when the length is not a multiple of 4. That is the only
/// check possible: a blob in another layout whose length happens to be a
/// multiple of 4 (big-endian ids, pairs of 16-bit ids, varints that land on
/// a 4-byte boundary) decodes to wrong ids without any error. Only decode
/// blobs this store wrote, or once the layout is confirmed (see
/// [`encode_source_node_ids`]); [`SemanticRunItemDetail::source_node_blob`]
/// keeps the raw bytes for everything else.
pub fn decode_source_node_ids(blob: &[u8]) -> Option<Vec<u32>> {
    if !blob.len().is_multiple_of(4) {
        return None;
    }
    Some(
        blob.chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

/// Control characters (Unicode category Cc) that are not whitespace: NUL and
/// the rest of C0 and C1 apart from tab, line feed, vertical tab, form feed,
/// carriage return and next line. None of them belong in a stored label, and
/// NUL is harmful: SQLite's `length()` stops counting at the first NUL, so a
/// label starting with one fails the alias and actor tables' length checks.
fn is_non_whitespace_control(c: char) -> bool {
    c.is_control() && !c.is_whitespace()
}

/// Format characters that render as nothing and only get in the way of
/// matching labels: soft hyphen, Arabic letter mark, Mongolian vowel
/// separator, zero-width space and joiners, left-to-right and right-to-left
/// marks, embeddings, overrides and isolates (chat apps wrap phone numbers
/// and right-to-left names in these), word joiner and invisible operators,
/// byte-order mark and interlinear annotation marks.
fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
    )
}

/// True for characters a label's display form drops.
fn is_hidden(c: char) -> bool {
    is_invisible(c) || is_non_whitespace_control(c)
}

/// `s` without NUL and the other non-whitespace control characters.
fn strip_controls(s: &str) -> Cow<'_, str> {
    if s.chars().any(is_non_whitespace_control) {
        Cow::Owned(
            s.chars()
                .filter(|c| !is_non_whitespace_control(*c))
                .collect(),
        )
    } else {
        Cow::Borrowed(s)
    }
}

/// The display form of an actor label: control characters other than
/// whitespace (NUL included) and invisible format characters removed, runs
/// of whitespace collapsed to one space, ends trimmed. Empty when the label
/// has no visible characters, which the store treats as no label at all.
pub fn display_semantic_actor_label(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut pending_space = false;
    for c in label.chars().filter(|c| !is_hidden(*c)) {
        if c.is_whitespace() {
            pending_space = !out.is_empty();
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(c);
        }
    }
    out
}

/// The alias lookup key for an actor label: the display form, lower-cased.
/// Two labels with the same normalized form under the same source key are the
/// same alias.
pub fn normalize_semantic_actor_label(label: &str) -> String {
    display_semantic_actor_label(label).to_lowercase()
}

/// Cut `s` to at most `max` characters (SQLite `length()` counts characters
/// for TEXT).
fn clip_chars(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

/// Alias columns derived from one observed actor label.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AliasLabels {
    /// The label as the parser emitted it minus non-whitespace control
    /// characters, clipped to 256 characters.
    source_label: String,
    /// Display form, clipped to 256 characters; also the name of an actor the
    /// heuristic creates.
    observed_label: String,
    /// Lookup key, clipped to 512 characters.
    normalized_label: String,
}

impl AliasLabels {
    /// `None` when the label has no visible characters. Every field of a
    /// `Some` is non-empty and free of NUL, and `observed_label` starts with
    /// a visible character, so the alias row satisfies the schema's
    /// `length()` and `length(trim())` checks.
    fn from_label(label: &str) -> Option<Self> {
        let display = display_semantic_actor_label(label);
        if display.is_empty() {
            return None;
        }
        let observed = clip_chars(&display, SEMANTIC_ACTOR_NAME_MAX_CHARS)
            .trim_end()
            .to_string();
        let normalized =
            clip_chars(&display.to_lowercase(), SEMANTIC_NORMALIZED_LABEL_MAX_CHARS).to_string();
        let source = clip_chars(&strip_controls(label), SEMANTIC_ACTOR_NAME_MAX_CHARS).to_string();
        Some(Self {
            source_label: source,
            observed_label: observed,
            normalized_label: normalized,
        })
    }
}

fn fingerprint_from_blob(column: &'static str, blob: Vec<u8>) -> Result<SemanticFingerprint> {
    let len = blob.len();
    blob.try_into()
        .map_err(|_| SemanticStoreError::InvalidStoredValue {
            column,
            value: format!("{len}-byte blob"),
        })
}

fn content_hash_from_blob(blob: Vec<u8>) -> Result<[u8; SEMANTIC_CONTENT_HASH_LEN]> {
    let len = blob.len();
    blob.try_into()
        .map_err(|_| SemanticStoreError::InvalidStoredValue {
            column: "semantic_runs.input_content_hash",
            value: format!("{len}-byte blob"),
        })
}

fn to_i64(field: &str, value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| SemanticStoreError::invalid(format!("{field} is too large")))
}

fn require_positive(field: &str, id: i64) -> Result<()> {
    if id > 0 {
        Ok(())
    } else {
        Err(SemanticStoreError::invalid(format!(
            "{field} must be positive"
        )))
    }
}

fn page(limit: u32, offset: u32) -> (i64, i64) {
    (limit.clamp(1, SEMANTIC_MAX_PAGE) as i64, offset as i64)
}

/// Validate an actor name and return the form to store: non-whitespace
/// control characters (NUL included) removed, ends trimmed. A name with no
/// visible characters is rejected like an empty one, so a bad name is a 400
/// and never reaches the table's `length(trim(name))` check.
fn clean_actor_name(name: &str) -> Result<String> {
    let stripped = strip_controls(name);
    let trimmed = stripped.trim();
    let chars = trimmed.chars().count();
    if chars == 0 || chars > SEMANTIC_ACTOR_NAME_MAX_CHARS {
        return Err(SemanticStoreError::invalid(format!(
            "name must be between 1 and {SEMANTIC_ACTOR_NAME_MAX_CHARS} characters"
        )));
    }
    if display_semantic_actor_label(trimmed).is_empty() {
        return Err(SemanticStoreError::invalid(
            "name must contain a visible character",
        ));
    }
    Ok(trimmed.to_string())
}

/// Escape `%`, `_` and `\` for a `LIKE ... ESCAPE '\'` substring pattern.
fn like_contains_pattern(q: &str) -> String {
    let mut out = String::with_capacity(q.len() + 2);
    out.push('%');
    for c in q.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// `?first, ?first+1, ...` for an `IN (...)` list of `n` values. Every
/// placeholder in a statement must be numbered: sqlx fills unnumbered `?`
/// from the first argument on, whatever numbered ones the statement has.
fn numbered_placeholders(first: usize, n: usize) -> String {
    (first..first + n)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Timestamps this module writes use the same shape as the schema's
/// `strftime('%Y-%m-%dT%H:%M:%fZ', 'now')` defaults, so text comparison
/// orders them correctly.
fn format_timestamp(ts: &DateTime<Utc>) -> String {
    ts.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn serialize_hex<S: Serializer>(bytes: &[u8], s: S) -> std::result::Result<S::Ok, S::Error> {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    s.serialize_str(&out)
}

fn serialize_node_blob<S: Serializer>(blob: &[u8], s: S) -> std::result::Result<S::Ok, S::Error> {
    decode_source_node_ids(blob).serialize(s)
}

/// Commit `tx` when `out` is a success, roll it back otherwise.
async fn finish_tx<T>(tx: ImmediateTx, out: Result<T>) -> Result<T> {
    match out {
        Ok(value) => {
            tx.commit().await?;
            Ok(value)
        }
        Err(e) => {
            if let Err(rb) = tx.rollback().await {
                warn!("semantic store rollback failed: {rb}");
            }
            Err(e)
        }
    }
}

// ---------------------------------------------------------------------------
// Write-side inputs
// ---------------------------------------------------------------------------

/// One parse to store: the run plus the items it produced, in run order.
#[derive(Debug, Clone)]
pub struct NewSemanticRun {
    /// Fingerprint of the parser input. Runs are deduplicated on it.
    pub input_fingerprint: SemanticFingerprint,
    pub parser_id: String,
    pub parser_version: String,
    /// Version of the item schema the parser emits; must be at least 1.
    pub schema_version: u32,
    pub app_platform: SemanticAppPlatform,
    /// Bundle id or equivalent app identifier, when known.
    pub app_id: Option<String>,
    pub app_executable: Option<String>,
    pub app_version: Option<String>,
    /// Short hash of the parser input's content.
    pub input_content_hash: [u8; SEMANTIC_CONTENT_HASH_LEN],
    /// `Handled` needs at least one item, `Empty` needs none.
    pub status: SemanticRunStatus,
    /// Size of the compact text the parse produced, in bytes.
    pub text_bytes: u64,
    pub parse_duration_us: u64,
    /// Scope for actor aliases (see [`DatabaseManager::store_semantic_run`]).
    /// `None` or blank uses `parser_id`.
    ///
    /// The key may be built from captured text (a workspace or account
    /// name), so a bad one never costs the parse: a key longer than
    /// [`SEMANTIC_SOURCE_KEY_MAX_CHARS`] or containing a control character
    /// is not used, the run and its items are stored without heuristic actor
    /// assignment ([`StoredSemanticRun::actor_assignment_skipped`]) and a
    /// warning is logged.
    pub actor_source_key: Option<String>,
    /// Items in run order. An item's `sort_order` is its index here.
    pub items: Vec<NewSemanticItem>,
}

impl NewSemanticRun {
    /// The source key that this run's actor labels are looked up under:
    /// `actor_source_key` when it is set and not blank, `parser_id`
    /// otherwise. Heuristic assignment only uses it when it is valid (see
    /// [`Self::actor_source_key`]).
    pub fn effective_actor_source_key(&self) -> &str {
        self.explicit_actor_source_key()
            .unwrap_or(self.parser_id.trim())
    }

    /// `actor_source_key`, trimmed, when it is set and not blank.
    fn explicit_actor_source_key(&self) -> Option<&str> {
        self.actor_source_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
    }
}

/// Why `key` cannot scope actor aliases, if it cannot. The alias table
/// requires 1 to [`SEMANTIC_SOURCE_KEY_MAX_CHARS`] characters as SQLite counts
/// them, and SQLite stops counting at a NUL, so control characters are out
/// too.
fn source_key_problem(key: &str) -> Option<String> {
    if key.chars().count() > SEMANTIC_SOURCE_KEY_MAX_CHARS {
        return Some(format!(
            "must be at most {SEMANTIC_SOURCE_KEY_MAX_CHARS} characters"
        ));
    }
    if key.chars().any(char::is_control) {
        return Some("must not contain control characters".into());
    }
    None
}

/// One item of a [`NewSemanticRun`].
#[derive(Debug, Clone)]
pub struct NewSemanticItem {
    /// Run-local id, unique within the run.
    pub local_id: String,
    /// Run-local id of the parent item, if any.
    pub parent_local_id: Option<String>,
    pub entity_fingerprint: SemanticFingerprint,
    pub version_fingerprint: SemanticFingerprint,
    pub kind: SemanticItemKind,
    pub item_key: String,
    pub identity_quality: SemanticIdentityQuality,
    pub title: Option<String>,
    pub body: Option<String>,
    /// The actor label exactly as observed. Drives heuristic assignment.
    pub actor: Option<String>,
    pub occurred_at: Option<DateTime<Utc>>,
    pub timestamp_precision: Option<String>,
    pub status: Option<String>,
    pub metadata: serde_json::Map<String, serde_json::Value>,
    /// Ids of the tree nodes the item was read from.
    pub source_node_ids: Vec<u32>,
}

impl NewSemanticItem {
    /// An item with the required fields set and everything else empty.
    pub fn new(
        local_id: impl Into<String>,
        kind: SemanticItemKind,
        item_key: impl Into<String>,
        identity_quality: SemanticIdentityQuality,
        entity_fingerprint: SemanticFingerprint,
        version_fingerprint: SemanticFingerprint,
    ) -> Self {
        Self {
            local_id: local_id.into(),
            parent_local_id: None,
            entity_fingerprint,
            version_fingerprint,
            kind,
            item_key: item_key.into(),
            identity_quality,
            title: None,
            body: None,
            actor: None,
            occurred_at: None,
            timestamp_precision: None,
            status: None,
            metadata: serde_json::Map::new(),
            source_node_ids: Vec::new(),
        }
    }
}

/// Point one frame at one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SemanticFrameAttachment {
    pub frame_id: i64,
    pub run_id: i64,
}

/// Result of [`DatabaseManager::attach_semantic_runs_to_frames`] and
/// [`SemanticFrameAttachmentBuffer::flush`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SemanticAttachOutcome {
    /// Frame rows updated. Frames that no longer exist are not counted and
    /// never reported in [`Self::skipped`], whether or not their run still
    /// exists: there is nothing left to attach a run to, and storing their
    /// parse again would bring deleted text back.
    pub frames_updated: u64,
    /// Attachments not written because their run no longer exists, for
    /// frames that still exist when the batch is written (checked in the same
    /// transaction), ordered by frame id. GC collected the run between the
    /// worker getting it (stored, or found by fingerprint) and the flush:
    /// either a reused run older than the GC grace that nothing pointed at,
    /// or any unattached run during a zero-grace pass after a user delete.
    /// These frames are still there without a run: the caller should store
    /// the parse again (which creates a new run) or queue the frame for
    /// parsing again, then attach the new run.
    pub skipped: Vec<SemanticFrameAttachment>,
    /// Runs deleted because the batch named them only for frames that no
    /// longer exist and no other frame points at them, typically a parse the
    /// worker stored after its frame was deleted (see
    /// [`DatabaseManager::attach_semantic_runs_to_frames`]).
    pub runs_discarded: u64,
    /// Items deleted with [`Self::runs_discarded`] because no other run
    /// contains them. Their FTS text and actor assignments go with them.
    pub items_discarded: u64,
}

// ---------------------------------------------------------------------------
// Read-side records
// ---------------------------------------------------------------------------

/// Result of [`DatabaseManager::store_semantic_run`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoredSemanticRun {
    pub run_id: i64,
    /// False when a run with the same input fingerprint already existed. The
    /// existing run is returned unchanged and the new items are not written.
    pub created: bool,
    pub status: SemanticRunStatus,
    /// Item ids in run order.
    pub item_ids: Vec<i64>,
    /// Item rows this call inserted (the rest already existed).
    pub items_inserted: usize,
    /// Actors (each with its first alias) the heuristic created.
    pub actors_created: usize,
    /// Items the heuristic assigned an actor to.
    pub assignments_created: usize,
    /// True when the run's `actor_source_key` could not be used (too long,
    /// or containing a control character), so the run and its items were
    /// stored without heuristic actor assignment. Their items can still be
    /// assigned by a later run that sees them with a valid key, or by hand.
    pub actor_assignment_skipped: bool,
}

/// A `semantic_runs` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SemanticRun {
    pub id: i64,
    #[serde(serialize_with = "serialize_hex")]
    pub input_fingerprint: SemanticFingerprint,
    pub parser_id: String,
    pub parser_version: String,
    pub schema_version: i64,
    pub app_platform: SemanticAppPlatform,
    pub app_id: Option<String>,
    pub app_executable: Option<String>,
    pub app_version: Option<String>,
    #[serde(serialize_with = "serialize_hex")]
    pub input_content_hash: [u8; SEMANTIC_CONTENT_HASH_LEN],
    pub status: SemanticRunStatus,
    pub item_count: i64,
    pub text_bytes: i64,
    pub parse_duration_us: i64,
    pub created_at: DateTime<Utc>,
}

/// A `semantic_items` row.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SemanticItem {
    pub id: i64,
    #[serde(serialize_with = "serialize_hex")]
    pub entity_fingerprint: SemanticFingerprint,
    #[serde(serialize_with = "serialize_hex")]
    pub version_fingerprint: SemanticFingerprint,
    pub kind: SemanticItemKind,
    pub item_key: String,
    pub identity_quality: SemanticIdentityQuality,
    pub title: Option<String>,
    pub body: Option<String>,
    /// Observed actor label, as stored.
    pub actor: Option<String>,
    /// As stored (this store writes RFC 3339 UTC with milliseconds).
    pub occurred_at: Option<String>,
    pub timestamp_precision: Option<String>,
    pub status: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// The identity assigned to an item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SemanticAssignedActor {
    pub actor_id: i64,
    pub name: String,
    pub observed_alias_id: Option<i64>,
    pub assignment_source: SemanticAssignmentSource,
}

/// One item as placed in a run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SemanticRunItemDetail {
    pub local_id: String,
    pub parent_local_id: Option<String>,
    pub sort_order: i64,
    /// The stored blob. Serialized as the decoded id list, or null when the
    /// blob is not in the 4-byte encoding.
    #[serde(rename = "source_node_ids", serialize_with = "serialize_node_blob")]
    pub source_node_blob: Vec<u8>,
    pub item: SemanticItem,
    pub assigned_actor: Option<SemanticAssignedActor>,
}

impl SemanticRunItemDetail {
    /// Decoded source node ids; see [`decode_source_node_ids`].
    pub fn source_node_ids(&self) -> Option<Vec<u32>> {
        decode_source_node_ids(&self.source_node_blob)
    }
}

/// A run with its items in run order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SemanticRunDetail {
    pub run: SemanticRun,
    pub items: Vec<SemanticRunItemDetail>,
}

/// Filter for the full-text search methods.
#[derive(Debug, Clone, Default)]
pub struct SemanticItemQuery {
    /// Plain words. Each becomes a quoted FTS5 term and all must match
    /// somewhere in title, body, actor, status or metadata.
    pub text: String,
    /// Only items currently assigned to this actor.
    pub actor_id: Option<i64>,
    pub kind: Option<SemanticItemKind>,
    /// Page size, clamped to 1..=[`SEMANTIC_MAX_PAGE`].
    pub limit: u32,
    pub offset: u32,
}

/// One item hit from [`DatabaseManager::search_semantic_items`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SemanticItemHit {
    pub item: SemanticItem,
    pub assigned_actor: Option<SemanticAssignedActor>,
    /// FTS5 bm25 score. Lower is a better match.
    pub score: f64,
}

/// One frame hit from [`DatabaseManager::search_semantic_frames`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SemanticFrameHit {
    pub frame_id: i64,
    pub timestamp: DateTime<Utc>,
    pub run_id: i64,
    /// Items of the run that matched, ascending.
    pub item_ids: Vec<i64>,
}

/// A `semantic_actors` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SemanticActor {
    pub id: i64,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A `semantic_actor_aliases` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SemanticActorAlias {
    pub id: i64,
    pub actor_id: i64,
    pub source_key: String,
    pub source_label: String,
    pub normalized_label: String,
    pub observed_label: String,
    pub created_at: DateTime<Utc>,
}

/// An actor with its assignment count and aliases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SemanticActorSummary {
    pub id: i64,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Items currently assigned to the actor.
    pub item_count: i64,
    /// Oldest first.
    pub aliases: Vec<SemanticActorAlias>,
}

/// Result of [`DatabaseManager::merge_semantic_actors`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SemanticActorMerge {
    /// The kept actor after the merge.
    pub actor: SemanticActor,
    pub merged_actor_id: i64,
    pub aliases_moved: u64,
    pub items_moved: u64,
}

/// Result of [`DatabaseManager::reassign_semantic_item_actor`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SemanticItemActorReassignment {
    pub item_id: i64,
    pub actor_id: i64,
    pub previous_actor_id: Option<i64>,
    pub observed_alias_id: Option<i64>,
    pub assignment_source: SemanticAssignmentSource,
    pub updated_at: DateTime<Utc>,
}

/// Result of [`DatabaseManager::reassign_semantic_actor_alias`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SemanticAliasReassignment {
    /// The alias after the move.
    pub alias: SemanticActorAlias,
    pub previous_actor_id: i64,
    /// Heuristic assignments that moved with the alias.
    pub items_moved: u64,
}

/// Result of [`DatabaseManager::gc_semantic_projection`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct SemanticGcStats {
    pub runs_deleted: u64,
    pub items_deleted: u64,
}

// ---------------------------------------------------------------------------
// Row mapping
// ---------------------------------------------------------------------------

const RUN_COLUMNS: &str = "r.id, r.input_fingerprint, r.parser_id, r.parser_version, \
     r.schema_version, r.app_platform, r.app_id, r.app_executable, r.app_version, \
     r.input_content_hash, r.status, r.item_count, r.text_bytes, r.parse_duration_us, \
     r.created_at";

const ITEM_COLUMNS: &str = "i.id AS item_id, i.entity_fingerprint, i.version_fingerprint, \
     i.kind, i.item_key, i.identity_quality, i.title, i.body, i.actor, i.occurred_at, \
     i.timestamp_precision, i.status AS item_status, i.metadata_json, \
     i.created_at AS item_created_at";

const ASSIGNED_COLUMNS: &str = "ia.actor_id AS assigned_actor_id, \
     a.name AS assigned_actor_name, ia.observed_alias_id, ia.assignment_source";

const ALIAS_COLUMNS: &str =
    "id, actor_id, source_key, source_label, normalized_label, observed_label, created_at";

fn run_from_row(row: &SqliteRow) -> Result<SemanticRun> {
    Ok(SemanticRun {
        id: row.try_get("id")?,
        input_fingerprint: fingerprint_from_blob(
            "semantic_runs.input_fingerprint",
            row.try_get("input_fingerprint")?,
        )?,
        parser_id: row.try_get("parser_id")?,
        parser_version: row.try_get("parser_version")?,
        schema_version: row.try_get("schema_version")?,
        app_platform: SemanticAppPlatform::from_stored(&row.try_get::<String, _>("app_platform")?)?,
        app_id: row.try_get("app_id")?,
        app_executable: row.try_get("app_executable")?,
        app_version: row.try_get("app_version")?,
        input_content_hash: content_hash_from_blob(row.try_get("input_content_hash")?)?,
        status: SemanticRunStatus::from_stored(&row.try_get::<String, _>("status")?)?,
        item_count: row.try_get("item_count")?,
        text_bytes: row.try_get("text_bytes")?,
        parse_duration_us: row.try_get("parse_duration_us")?,
        created_at: row.try_get("created_at")?,
    })
}

fn item_from_row(row: &SqliteRow) -> Result<SemanticItem> {
    let metadata_text: String = row.try_get("metadata_json")?;
    let metadata = match serde_json::from_str(&metadata_text) {
        Ok(v) => v,
        Err(_) => {
            return Err(SemanticStoreError::InvalidStoredValue {
                column: "semantic_items.metadata_json",
                value: metadata_text,
            })
        }
    };
    Ok(SemanticItem {
        id: row.try_get("item_id")?,
        entity_fingerprint: fingerprint_from_blob(
            "semantic_items.entity_fingerprint",
            row.try_get("entity_fingerprint")?,
        )?,
        version_fingerprint: fingerprint_from_blob(
            "semantic_items.version_fingerprint",
            row.try_get("version_fingerprint")?,
        )?,
        kind: SemanticItemKind::from_stored(&row.try_get::<String, _>("kind")?)?,
        item_key: row.try_get("item_key")?,
        identity_quality: SemanticIdentityQuality::from_stored(
            &row.try_get::<String, _>("identity_quality")?,
        )?,
        title: row.try_get("title")?,
        body: row.try_get("body")?,
        actor: row.try_get("actor")?,
        occurred_at: row.try_get("occurred_at")?,
        timestamp_precision: row.try_get("timestamp_precision")?,
        status: row.try_get("item_status")?,
        metadata,
        created_at: row.try_get("item_created_at")?,
    })
}

fn assigned_from_row(row: &SqliteRow) -> Result<Option<SemanticAssignedActor>> {
    let actor_id: Option<i64> = row.try_get("assigned_actor_id")?;
    let Some(actor_id) = actor_id else {
        return Ok(None);
    };
    Ok(Some(SemanticAssignedActor {
        actor_id,
        name: row
            .try_get::<Option<String>, _>("assigned_actor_name")?
            .unwrap_or_default(),
        observed_alias_id: row.try_get("observed_alias_id")?,
        assignment_source: SemanticAssignmentSource::from_stored(
            &row.try_get::<String, _>("assignment_source")?,
        )?,
    }))
}

fn actor_from_row(row: &SqliteRow) -> Result<SemanticActor> {
    Ok(SemanticActor {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn alias_from_row(row: &SqliteRow) -> Result<SemanticActorAlias> {
    Ok(SemanticActorAlias {
        id: row.try_get("id")?,
        actor_id: row.try_get("actor_id")?,
        source_key: row.try_get("source_key")?,
        source_label: row.try_get("source_label")?,
        normalized_label: row.try_get("normalized_label")?,
        observed_label: row.try_get("observed_label")?,
        created_at: row.try_get("created_at")?,
    })
}

// ---------------------------------------------------------------------------
// Validation of a new run
// ---------------------------------------------------------------------------

fn validate_new_run(run: &NewSemanticRun) -> Result<()> {
    if run.parser_id.trim().is_empty() {
        return Err(SemanticStoreError::invalid("parser_id must not be empty"));
    }
    if run.parser_version.trim().is_empty() {
        return Err(SemanticStoreError::invalid(
            "parser_version must not be empty",
        ));
    }
    if run.schema_version == 0 {
        return Err(SemanticStoreError::invalid(
            "schema_version must be positive",
        ));
    }
    to_i64("text_bytes", run.text_bytes)?;
    to_i64("parse_duration_us", run.parse_duration_us)?;
    match run.status {
        SemanticRunStatus::Empty if !run.items.is_empty() => {
            return Err(SemanticStoreError::invalid(
                "a run with status 'empty' cannot carry items",
            ));
        }
        SemanticRunStatus::Handled if run.items.is_empty() => {
            return Err(SemanticStoreError::invalid(
                "a run with status 'handled' needs at least one item",
            ));
        }
        _ => {}
    }
    // When the parser id is the alias source key it must be a valid one. The
    // parser id is a constant of the parser's code, so a bad one is a caller
    // bug and fails the run here, instead of failing the alias table's length
    // check later with an opaque constraint error. An explicit
    // `actor_source_key` may come from captured text, so a bad one only skips
    // heuristic assignment (see `store_run_tx`).
    if run.explicit_actor_source_key().is_none() {
        if let Some(problem) = source_key_problem(run.parser_id.trim()) {
            return Err(SemanticStoreError::invalid(format!("parser_id {problem}")));
        }
    }

    let mut parents: HashMap<&str, Option<&str>> = HashMap::with_capacity(run.items.len());
    for item in &run.items {
        if item.local_id.is_empty() {
            return Err(SemanticStoreError::invalid("local_id must not be empty"));
        }
        if parents
            .insert(item.local_id.as_str(), item.parent_local_id.as_deref())
            .is_some()
        {
            return Err(SemanticStoreError::invalid(format!(
                "duplicate local_id {:?}",
                item.local_id
            )));
        }
    }
    for item in &run.items {
        if let Some(parent) = item.parent_local_id.as_deref() {
            if !parents.contains_key(parent) {
                return Err(SemanticStoreError::invalid(format!(
                    "item {:?} names unknown parent {:?}",
                    item.local_id, parent
                )));
            }
        }
    }
    // Parent links must form a forest. Walking up from an item takes at most
    // `len` steps unless the links loop.
    for item in &run.items {
        let mut current = item.parent_local_id.as_deref();
        let mut steps = 0usize;
        while let Some(id) = current {
            if id == item.local_id {
                return Err(SemanticStoreError::invalid(format!(
                    "item {:?} is its own ancestor",
                    item.local_id
                )));
            }
            steps += 1;
            if steps > run.items.len() {
                return Err(SemanticStoreError::invalid("parent links form a cycle"));
            }
            current = parents.get(id).copied().flatten();
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Statements run inside a transaction or on a pooled connection
// ---------------------------------------------------------------------------

async fn fetch_run_by_fingerprint(
    conn: &mut SqliteConnection,
    fingerprint: &[u8],
) -> Result<Option<SemanticRun>> {
    let sql = format!("SELECT {RUN_COLUMNS} FROM semantic_runs r WHERE r.input_fingerprint = ?1");
    let row = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(fingerprint)
        .fetch_optional(&mut *conn)
        .await?;
    row.as_ref().map(run_from_row).transpose()
}

async fn fetch_run_item_ids(conn: &mut SqliteConnection, run_id: i64) -> Result<Vec<i64>> {
    Ok(sqlx::query_scalar(
        "SELECT item_id FROM semantic_run_items WHERE run_id = ?1 ORDER BY sort_order",
    )
    .bind(run_id)
    .fetch_all(&mut *conn)
    .await?)
}

async fn fetch_actor(conn: &mut SqliteConnection, id: i64) -> Result<Option<SemanticActor>> {
    let row =
        sqlx::query("SELECT id, name, created_at, updated_at FROM semantic_actors WHERE id = ?1")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
    row.as_ref().map(actor_from_row).transpose()
}

async fn require_actor(conn: &mut SqliteConnection, id: i64) -> Result<SemanticActor> {
    fetch_actor(conn, id)
        .await?
        .ok_or_else(|| SemanticStoreError::not_found("semantic actor"))
}

async fn fetch_alias(conn: &mut SqliteConnection, id: i64) -> Result<Option<SemanticActorAlias>> {
    let sql = format!("SELECT {ALIAS_COLUMNS} FROM semantic_actor_aliases WHERE id = ?1");
    let row = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    row.as_ref().map(alias_from_row).transpose()
}

async fn touch_actors(conn: &mut SqliteConnection, ids: &[i64]) -> Result<()> {
    for id in ids {
        sqlx::query(
            "UPDATE semantic_actors \
             SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
        )
        .bind(id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// Aliases of the given actors, grouped by actor, oldest first.
async fn aliases_for_actors(
    conn: &mut SqliteConnection,
    actor_ids: &[i64],
) -> Result<HashMap<i64, Vec<SemanticActorAlias>>> {
    let mut out: HashMap<i64, Vec<SemanticActorAlias>> = HashMap::new();
    for chunk in actor_ids.chunks(IN_CHUNK) {
        let sql = format!(
            "SELECT {ALIAS_COLUMNS} FROM semantic_actor_aliases \
             WHERE actor_id IN ({}) ORDER BY created_at, id",
            numbered_placeholders(1, chunk.len())
        );
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
        for id in chunk {
            q = q.bind(id);
        }
        for row in q.fetch_all(&mut *conn).await? {
            let alias = alias_from_row(&row)?;
            out.entry(alias.actor_id).or_default().push(alias);
        }
    }
    Ok(out)
}

/// Find the alias for `labels` under `source_key`, creating it together with
/// a new actor named after the observed label when it is missing. Returns
/// `(alias_id, actor_id, created)`.
async fn resolve_alias(
    conn: &mut SqliteConnection,
    source_key: &str,
    labels: &AliasLabels,
) -> Result<(i64, i64, bool)> {
    let existing: Option<(i64, i64)> = sqlx::query_as(
        "SELECT id, actor_id FROM semantic_actor_aliases \
         WHERE source_key = ?1 AND normalized_label = ?2",
    )
    .bind(source_key)
    .bind(&labels.normalized_label)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some((alias_id, actor_id)) = existing {
        return Ok((alias_id, actor_id, false));
    }

    let actor_id = sqlx::query("INSERT INTO semantic_actors (name) VALUES (?1)")
        .bind(&labels.observed_label)
        .execute(&mut *conn)
        .await?
        .last_insert_rowid();
    let alias_id = sqlx::query(
        "INSERT INTO semantic_actor_aliases \
         (actor_id, source_key, source_label, normalized_label, observed_label) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(actor_id)
    .bind(source_key)
    .bind(&labels.source_label)
    .bind(&labels.normalized_label)
    .bind(&labels.observed_label)
    .execute(&mut *conn)
    .await?
    .last_insert_rowid();
    Ok((alias_id, actor_id, true))
}

async fn store_run_tx(
    conn: &mut SqliteConnection,
    run: &NewSemanticRun,
) -> Result<StoredSemanticRun> {
    let inserted = sqlx::query(
        "INSERT INTO semantic_runs \
         (input_fingerprint, parser_id, parser_version, schema_version, app_platform, \
          app_id, app_executable, app_version, input_content_hash, status, item_count, \
          text_bytes, parse_duration_us) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13) \
         ON CONFLICT(input_fingerprint) DO NOTHING",
    )
    .bind(&run.input_fingerprint[..])
    .bind(&run.parser_id)
    .bind(&run.parser_version)
    .bind(run.schema_version as i64)
    .bind(run.app_platform.as_str())
    .bind(&run.app_id)
    .bind(&run.app_executable)
    .bind(&run.app_version)
    .bind(&run.input_content_hash[..])
    .bind(run.status.as_str())
    .bind(run.items.len() as i64)
    .bind(to_i64("text_bytes", run.text_bytes)?)
    .bind(to_i64("parse_duration_us", run.parse_duration_us)?)
    .execute(&mut *conn)
    .await?;

    if inserted.rows_affected() == 0 {
        let existing = fetch_run_by_fingerprint(conn, &run.input_fingerprint[..])
            .await?
            .ok_or_else(|| {
                SemanticStoreError::Conflict(
                    "the run insert was ignored but no run has its fingerprint".into(),
                )
            })?;
        let compatible = existing.parser_id == run.parser_id
            && existing.parser_version == run.parser_version
            && existing.schema_version == run.schema_version as i64
            && existing.app_platform == run.app_platform
            && existing.input_content_hash == run.input_content_hash;
        if !compatible {
            return Err(SemanticStoreError::Conflict(format!(
                "run {} has this input fingerprint but came from {} {} (schema {}, {})",
                existing.id,
                existing.parser_id,
                existing.parser_version,
                existing.schema_version,
                existing.app_platform
            )));
        }
        let item_ids = fetch_run_item_ids(conn, existing.id).await?;
        return Ok(StoredSemanticRun {
            run_id: existing.id,
            created: false,
            status: existing.status,
            item_ids,
            items_inserted: 0,
            actors_created: 0,
            assignments_created: 0,
            actor_assignment_skipped: false,
        });
    }
    let run_id = inserted.last_insert_rowid();

    let mut item_ids = Vec::with_capacity(run.items.len());
    let mut items_inserted = 0usize;
    for item in &run.items {
        let metadata = serde_json::Value::Object(item.metadata.clone()).to_string();
        let res = sqlx::query(
            "INSERT INTO semantic_items \
             (entity_fingerprint, version_fingerprint, kind, item_key, identity_quality, \
              title, body, actor, occurred_at, timestamp_precision, status, metadata_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
             ON CONFLICT(version_fingerprint) DO NOTHING",
        )
        .bind(&item.entity_fingerprint[..])
        .bind(&item.version_fingerprint[..])
        .bind(item.kind.as_str())
        .bind(&item.item_key)
        .bind(item.identity_quality.as_str())
        .bind(&item.title)
        .bind(&item.body)
        .bind(&item.actor)
        .bind(item.occurred_at.as_ref().map(format_timestamp))
        .bind(&item.timestamp_precision)
        .bind(&item.status)
        .bind(metadata)
        .execute(&mut *conn)
        .await?;

        let item_id = if res.rows_affected() == 1 {
            items_inserted += 1;
            res.last_insert_rowid()
        } else {
            let (id, entity, kind): (i64, Vec<u8>, String) = sqlx::query_as(
                "SELECT id, entity_fingerprint, kind FROM semantic_items \
                 WHERE version_fingerprint = ?1",
            )
            .bind(&item.version_fingerprint[..])
            .fetch_one(&mut *conn)
            .await?;
            if entity != item.entity_fingerprint || kind != item.kind.as_str() {
                return Err(SemanticStoreError::Conflict(format!(
                    "item {id} has the version fingerprint of {:?} but a different \
                     entity fingerprint or kind",
                    item.local_id
                )));
            }
            id
        };

        sqlx::query(
            "INSERT INTO semantic_run_items \
             (run_id, local_id, item_id, parent_local_id, sort_order, source_node_ids) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(run_id)
        .bind(&item.local_id)
        .bind(item_id)
        .bind(&item.parent_local_id)
        .bind(item_ids.len() as i64)
        .bind(encode_source_node_ids(&item.source_node_ids))
        .execute(&mut *conn)
        .await?;
        item_ids.push(item_id);
    }

    // Heuristic actor assignment. The cache turns one sender with many
    // messages into one alias lookup. `validate_new_run` has already failed
    // the run for a bad parser id, so a problem here is an explicit key,
    // which may hold captured text: it skips this step and nothing else, and
    // the warning leaves the key itself out of the log.
    let source_key = run.effective_actor_source_key();
    let key_problem = source_key_problem(source_key);
    if let Some(problem) = &key_problem {
        warn!(
            run_id,
            parser_id = %run.parser_id,
            key_chars = source_key.chars().count(),
            "semantic: actor_source_key {problem}; run stored without actor assignment"
        );
    }
    let to_assign: &[NewSemanticItem] = if key_problem.is_some() {
        &[]
    } else {
        &run.items
    };
    let mut alias_cache: HashMap<String, (i64, i64)> = HashMap::new();
    let mut seen_items: HashSet<i64> = HashSet::new();
    let mut actors_created = 0usize;
    let mut assignments_created = 0usize;
    for (item, &item_id) in to_assign.iter().zip(&item_ids) {
        let Some(labels) = item.actor.as_deref().and_then(AliasLabels::from_label) else {
            continue;
        };
        if !seen_items.insert(item_id) {
            continue;
        }
        let assigned: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM semantic_item_actors WHERE item_id = ?1")
                .bind(item_id)
                .fetch_optional(&mut *conn)
                .await?;
        if assigned.is_some() {
            continue;
        }
        let (alias_id, actor_id) = match alias_cache.get(&labels.normalized_label) {
            Some(hit) => *hit,
            None => {
                let (alias_id, actor_id, created) =
                    resolve_alias(conn, source_key, &labels).await?;
                if created {
                    actors_created += 1;
                }
                alias_cache.insert(labels.normalized_label.clone(), (alias_id, actor_id));
                (alias_id, actor_id)
            }
        };
        let res = sqlx::query(
            "INSERT OR IGNORE INTO semantic_item_actors \
             (item_id, actor_id, observed_alias_id, assignment_source) \
             VALUES (?1, ?2, ?3, 'heuristic')",
        )
        .bind(item_id)
        .bind(actor_id)
        .bind(alias_id)
        .execute(&mut *conn)
        .await?;
        assignments_created += res.rows_affected() as usize;
    }

    Ok(StoredSemanticRun {
        run_id,
        created: true,
        status: run.status,
        item_ids,
        items_inserted,
        actors_created,
        assignments_created,
        actor_assignment_skipped: key_problem.is_some(),
    })
}

/// The ids in `ids` (at most [`IN_CHUNK`] of them) that still have a
/// `frames` row, ascending.
async fn existing_frame_ids(conn: &mut SqliteConnection, ids: &[i64]) -> Result<Vec<i64>> {
    let sql = format!(
        "SELECT id FROM frames WHERE id IN ({}) ORDER BY id",
        numbered_placeholders(1, ids.len())
    );
    let mut q = sqlx::query_scalar(sqlx::AssertSqlSafe(sql));
    for id in ids {
        q = q.bind(id);
    }
    Ok(q.fetch_all(&mut *conn).await?)
}

/// Delete run `run_id` unless a frame points at it, together with its items
/// that no other run contains; their actor assignments (cascade) and FTS rows
/// (delete trigger) go with them. Returns the number of items deleted, or
/// `None` when the run is referenced (or already gone) and nothing changed.
async fn discard_unreferenced_run(conn: &mut SqliteConnection, run_id: i64) -> Result<Option<u64>> {
    let referenced: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM frames WHERE semantic_run_id = ?1 LIMIT 1")
            .bind(run_id)
            .fetch_optional(&mut *conn)
            .await?;
    if referenced.is_some() {
        return Ok(None);
    }
    // Read the item ids first: deleting the run cascades to its run items.
    let item_ids: Vec<i64> =
        sqlx::query_scalar("SELECT DISTINCT item_id FROM semantic_run_items WHERE run_id = ?1")
            .bind(run_id)
            .fetch_all(&mut *conn)
            .await?;
    let runs_deleted = sqlx::query("DELETE FROM semantic_runs WHERE id = ?1")
        .bind(run_id)
        .execute(&mut *conn)
        .await?
        .rows_affected();
    if runs_deleted == 0 {
        return Ok(None);
    }
    let mut items_deleted = 0u64;
    for chunk in item_ids.chunks(IN_CHUNK) {
        let sql = format!(
            "DELETE FROM semantic_items WHERE id IN ({}) \
             AND NOT EXISTS ( \
                 SELECT 1 FROM semantic_run_items ri WHERE ri.item_id = semantic_items.id)",
            numbered_placeholders(1, chunk.len())
        );
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
        for id in chunk {
            q = q.bind(id);
        }
        items_deleted += q.execute(&mut *conn).await?.rows_affected();
    }
    Ok(Some(items_deleted))
}

/// The body of [`DatabaseManager::attach_semantic_runs_to_frames`]. It runs
/// inside one `BEGIN IMMEDIATE` transaction, which holds the write lock, so
/// neither a GC pass nor a frame delete can land between the checks below
/// and the writes they decide.
async fn attach_tx(
    conn: &mut SqliteConnection,
    by_run: BTreeMap<i64, Vec<i64>>,
) -> Result<SemanticAttachOutcome> {
    let mut outcome = SemanticAttachOutcome::default();
    for (run_id, frame_ids) in by_run {
        let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM semantic_runs WHERE id = ?1")
            .bind(run_id)
            .fetch_optional(&mut *conn)
            .await?;
        if exists.is_none() {
            // Report only the frames that are still there. A frame deleted
            // since the worker parsed it has nothing to attach to, and
            // reporting it would have the worker store its text again.
            for chunk in frame_ids.chunks(IN_CHUNK) {
                let live = existing_frame_ids(conn, chunk).await?;
                outcome.skipped.extend(
                    live.into_iter()
                        .map(|frame_id| SemanticFrameAttachment { frame_id, run_id }),
                );
            }
            continue;
        }
        let mut updated = 0u64;
        for chunk in frame_ids.chunks(IN_CHUNK) {
            let sql = format!(
                "UPDATE frames SET semantic_run_id = ?1 WHERE id IN ({})",
                numbered_placeholders(2, chunk.len())
            );
            let mut q = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(run_id);
            for id in chunk {
                q = q.bind(id);
            }
            updated += q.execute(&mut *conn).await?.rows_affected();
        }
        outcome.frames_updated += updated;
        // SQLite counts every row the UPDATE matched, so zero means none of
        // the frames named for this run exist any more. Unless another frame
        // points at the run, drop it now; otherwise a parse the worker
        // stored after its frame was deleted would stay searchable until the
        // next GC pass.
        if updated == 0 {
            if let Some(items) = discard_unreferenced_run(conn, run_id).await? {
                outcome.runs_discarded += 1;
                outcome.items_discarded += items;
            }
        }
    }
    outcome.skipped.sort_unstable_by_key(|a| a.frame_id);
    Ok(outcome)
}

async fn create_actor_tx(conn: &mut SqliteConnection, name: &str) -> Result<SemanticActor> {
    let id = sqlx::query("INSERT INTO semantic_actors (name) VALUES (?1)")
        .bind(name)
        .execute(&mut *conn)
        .await?
        .last_insert_rowid();
    require_actor(conn, id).await
}

async fn rename_actor_tx(
    conn: &mut SqliteConnection,
    id: i64,
    name: &str,
) -> Result<SemanticActor> {
    let changed = sqlx::query(
        "UPDATE semantic_actors \
         SET name = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE id = ?2",
    )
    .bind(name)
    .bind(id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(SemanticStoreError::not_found("semantic actor"));
    }
    require_actor(conn, id).await
}

async fn merge_tx(
    conn: &mut SqliteConnection,
    keep_id: i64,
    merge_id: i64,
) -> Result<SemanticActorMerge> {
    require_actor(conn, keep_id).await?;
    require_actor(conn, merge_id).await?;
    let aliases_moved =
        sqlx::query("UPDATE semantic_actor_aliases SET actor_id = ?1 WHERE actor_id = ?2")
            .bind(keep_id)
            .bind(merge_id)
            .execute(&mut *conn)
            .await?
            .rows_affected();
    let items_moved = sqlx::query(
        "UPDATE semantic_item_actors \
         SET actor_id = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE actor_id = ?2",
    )
    .bind(keep_id)
    .bind(merge_id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    sqlx::query("DELETE FROM semantic_actors WHERE id = ?1")
        .bind(merge_id)
        .execute(&mut *conn)
        .await?;
    touch_actors(conn, &[keep_id]).await?;
    Ok(SemanticActorMerge {
        actor: require_actor(conn, keep_id).await?,
        merged_actor_id: merge_id,
        aliases_moved,
        items_moved,
    })
}

async fn reassign_item_tx(
    conn: &mut SqliteConnection,
    item_id: i64,
    actor_id: i64,
) -> Result<SemanticItemActorReassignment> {
    let label: Option<Option<String>> =
        sqlx::query_scalar("SELECT actor FROM semantic_items WHERE id = ?1")
            .bind(item_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some(label) = label else {
        return Err(SemanticStoreError::not_found("semantic item"));
    };
    let has_label = label
        .as_deref()
        .is_some_and(|l| !display_semantic_actor_label(l).is_empty());
    if !has_label {
        return Err(SemanticStoreError::invalid(
            "the item has no observed actor to reassign",
        ));
    }
    require_actor(conn, actor_id).await?;
    let previous_actor_id: Option<i64> =
        sqlx::query_scalar("SELECT actor_id FROM semantic_item_actors WHERE item_id = ?1")
            .bind(item_id)
            .fetch_optional(&mut *conn)
            .await?;
    sqlx::query(
        "INSERT INTO semantic_item_actors \
         (item_id, actor_id, observed_alias_id, assignment_source, updated_at) \
         VALUES (?1, ?2, NULL, 'reconciled', strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) \
         ON CONFLICT(item_id) DO UPDATE SET \
             actor_id = excluded.actor_id, \
             assignment_source = 'reconciled', \
             updated_at = excluded.updated_at",
    )
    .bind(item_id)
    .bind(actor_id)
    .execute(&mut *conn)
    .await?;
    let mut touched = vec![actor_id];
    if let Some(prev) = previous_actor_id.filter(|p| *p != actor_id) {
        touched.push(prev);
    }
    touch_actors(conn, &touched).await?;
    let (observed_alias_id, source, updated_at): (Option<i64>, String, DateTime<Utc>) =
        sqlx::query_as(
            "SELECT observed_alias_id, assignment_source, updated_at \
             FROM semantic_item_actors WHERE item_id = ?1",
        )
        .bind(item_id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(SemanticItemActorReassignment {
        item_id,
        actor_id,
        previous_actor_id,
        observed_alias_id,
        assignment_source: SemanticAssignmentSource::from_stored(&source)?,
        updated_at,
    })
}

async fn reassign_alias_tx(
    conn: &mut SqliteConnection,
    alias_id: i64,
    actor_id: i64,
) -> Result<SemanticAliasReassignment> {
    let Some(alias) = fetch_alias(conn, alias_id).await? else {
        return Err(SemanticStoreError::not_found("semantic actor alias"));
    };
    require_actor(conn, actor_id).await?;
    let previous_actor_id = alias.actor_id;
    sqlx::query("UPDATE semantic_actor_aliases SET actor_id = ?1 WHERE id = ?2")
        .bind(actor_id)
        .bind(alias_id)
        .execute(&mut *conn)
        .await?;
    let items_moved = sqlx::query(
        "UPDATE semantic_item_actors \
         SET actor_id = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE observed_alias_id = ?2 \
           AND assignment_source = 'heuristic' \
           AND actor_id != ?1",
    )
    .bind(actor_id)
    .bind(alias_id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    let mut touched = vec![actor_id];
    if previous_actor_id != actor_id {
        touched.push(previous_actor_id);
    }
    touch_actors(conn, &touched).await?;
    let alias = fetch_alias(conn, alias_id)
        .await?
        .ok_or_else(|| SemanticStoreError::not_found("semantic actor alias"))?;
    Ok(SemanticAliasReassignment {
        alias,
        previous_actor_id,
        items_moved,
    })
}

// ---------------------------------------------------------------------------
// DatabaseManager methods
// ---------------------------------------------------------------------------

impl DatabaseManager {
    /// The run stored for an input fingerprint, if any. The semantic worker
    /// calls this before parsing so an unchanged input reuses its run.
    pub async fn find_semantic_run_by_fingerprint(
        &self,
        input_fingerprint: &SemanticFingerprint,
    ) -> Result<Option<SemanticRun>> {
        let mut conn = self.pool.acquire().await?;
        fetch_run_by_fingerprint(&mut conn, &input_fingerprint[..]).await
    }

    /// Store one parse in a single write transaction.
    ///
    /// 1. The run is inserted unless its input fingerprint is already stored.
    ///    An existing run must have the same parser id, parser version, schema
    ///    version, platform and content hash, otherwise the call fails with
    ///    [`SemanticStoreError::Conflict`]. A matching existing run is returned
    ///    as is (`created == false`). Runs are immutable, so the new items are
    ///    then ignored.
    /// 2. Each item is inserted unless its version fingerprint is already
    ///    stored. An existing item must have the same entity fingerprint and
    ///    kind.
    /// 3. Run items record each item's run-local id, parent, position (its index
    ///    in `items`) and source nodes.
    /// 4. Heuristic actor assignment: every item with a visible actor label and
    ///    no assignment yet is assigned to the actor of the alias
    ///    `(source key, normalized label)`. A missing alias is created together
    ///    with a new actor named after the label. Items that already have an
    ///    assignment, heuristic or reconciled, keep it.
    pub async fn store_semantic_run(&self, run: &NewSemanticRun) -> Result<StoredSemanticRun> {
        validate_new_run(run)?;
        let mut tx = self.begin_immediate_with_retry().await?;
        let out = store_run_tx(tx.conn(), run).await;
        let stored = finish_tx(tx, out).await?;
        debug!(
            run_id = stored.run_id,
            created = stored.created,
            items = stored.item_ids.len(),
            items_inserted = stored.items_inserted,
            actors_created = stored.actors_created,
            assignments_created = stored.assignments_created,
            "semantic run stored"
        );
        Ok(stored)
    }

    /// Point frames at runs (`frames.semantic_run_id`) in one transaction.
    ///
    /// When a frame appears more than once, the last entry wins. The
    /// transaction holds the write lock, so frames and runs cannot change
    /// while it decides what to do with each entry:
    ///
    /// - A frame that no longer exists (retention or a user delete removed it
    ///   after the worker read it) is ignored: not written, not counted and
    ///   never reported in [`SemanticAttachOutcome::skipped`], whatever state
    ///   its run is in. Storing its parse again would bring deleted text back.
    /// - A frame that exists but whose run no longer exists (collected by GC
    ///   in the meantime) is not written and does not fail the batch; it
    ///   comes back in [`SemanticAttachOutcome::skipped`] so the caller can
    ///   store or parse that frame again.
    /// - A run that the batch names only for frames that no longer exist, and
    ///   that no other frame points at, is deleted together with its items
    ///   that no other run contains (so their FTS text goes too), as a
    ///   zero-grace GC pass would. This covers a parse the worker stored after
    ///   its frame was deleted, which would otherwise stay searchable until
    ///   the next GC pass. A later attachment of another frame to that run
    ///   comes back in `skipped`. Counted in
    ///   [`SemanticAttachOutcome::runs_discarded`] and
    ///   [`SemanticAttachOutcome::items_discarded`].
    pub async fn attach_semantic_runs_to_frames(
        &self,
        attachments: &[SemanticFrameAttachment],
    ) -> Result<SemanticAttachOutcome> {
        if attachments.is_empty() {
            return Ok(SemanticAttachOutcome::default());
        }
        let mut latest: HashMap<i64, i64> = HashMap::with_capacity(attachments.len());
        for a in attachments {
            require_positive("frame_id", a.frame_id)?;
            require_positive("run_id", a.run_id)?;
            latest.insert(a.frame_id, a.run_id);
        }
        let mut by_run: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
        for (frame_id, run_id) in latest {
            by_run.entry(run_id).or_default().push(frame_id);
        }
        for frame_ids in by_run.values_mut() {
            frame_ids.sort_unstable();
        }
        let mut tx = self.begin_immediate_with_retry().await?;
        let out = attach_tx(tx.conn(), by_run).await;
        finish_tx(tx, out).await
    }

    /// A run and its items in run order, with each item's assigned actor.
    pub async fn get_semantic_run_detail(&self, run_id: i64) -> Result<Option<SemanticRunDetail>> {
        let mut conn = self.pool.acquire().await?;
        let sql = format!("SELECT {RUN_COLUMNS} FROM semantic_runs r WHERE r.id = ?1");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(run_id)
            .fetch_optional(&mut *conn)
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let run = run_from_row(&row)?;

        let sql = format!(
            "SELECT ri.local_id, ri.parent_local_id, ri.sort_order, ri.source_node_ids, \
                    {ITEM_COLUMNS}, {ASSIGNED_COLUMNS} \
             FROM semantic_run_items ri \
             JOIN semantic_items i ON i.id = ri.item_id \
             LEFT JOIN semantic_item_actors ia ON ia.item_id = i.id \
             LEFT JOIN semantic_actors a ON a.id = ia.actor_id \
             WHERE ri.run_id = ?1 \
             ORDER BY ri.sort_order"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(run_id)
            .fetch_all(&mut *conn)
            .await?;
        let mut items = Vec::with_capacity(rows.len());
        for row in &rows {
            items.push(SemanticRunItemDetail {
                local_id: row.try_get("local_id")?,
                parent_local_id: row.try_get("parent_local_id")?,
                sort_order: row.try_get("sort_order")?,
                source_node_blob: row.try_get("source_node_ids")?,
                item: item_from_row(row)?,
                assigned_actor: assigned_from_row(row)?,
            });
        }
        Ok(Some(SemanticRunDetail { run, items }))
    }

    /// The run a frame points at, with its items. `None` when the frame has
    /// no run or does not exist. Runs of either status are returned.
    pub async fn get_semantic_run_for_frame(
        &self,
        frame_id: i64,
    ) -> Result<Option<SemanticRunDetail>> {
        let run_id: Option<Option<i64>> =
            sqlx::query_scalar("SELECT semantic_run_id FROM frames WHERE id = ?1")
                .bind(frame_id)
                .fetch_optional(&self.pool)
                .await?;
        match run_id.flatten() {
            Some(run_id) => self.get_semantic_run_detail(run_id).await,
            None => Ok(None),
        }
    }

    /// Full-text search over item title, body, actor, status and metadata,
    /// best match first. Blank text returns nothing.
    pub async fn search_semantic_items(
        &self,
        query: &SemanticItemQuery,
    ) -> Result<Vec<SemanticItemHit>> {
        let fts = sanitize_fts5_query(&query.text);
        if fts.is_empty() {
            return Ok(Vec::new());
        }
        if let Some(actor_id) = query.actor_id {
            require_positive("actor_id", actor_id)?;
        }
        let (limit, offset) = page(query.limit, query.offset);
        let sql = format!(
            "SELECT {ITEM_COLUMNS}, {ASSIGNED_COLUMNS}, \
                    bm25(semantic_items_fts) AS score \
             FROM semantic_items_fts \
             JOIN semantic_items i ON i.id = semantic_items_fts.rowid \
             LEFT JOIN semantic_item_actors ia ON ia.item_id = i.id \
             LEFT JOIN semantic_actors a ON a.id = ia.actor_id \
             WHERE semantic_items_fts MATCH ?1 \
               AND (?2 IS NULL OR ia.actor_id = ?2) \
               AND (?3 IS NULL OR i.kind = ?3) \
             ORDER BY score, i.id DESC \
             LIMIT ?4 OFFSET ?5"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(fts)
            .bind(query.actor_id)
            .bind(query.kind.map(|k| k.as_str()))
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|row| {
                Ok(SemanticItemHit {
                    item: item_from_row(row)?,
                    assigned_actor: assigned_from_row(row)?,
                    score: row.try_get("score")?,
                })
            })
            .collect()
    }

    /// Frames whose handled run contains a matching item, newest first, along
    /// the join `frames -> semantic_runs (handled) -> semantic_run_items ->
    /// semantic_items_fts`. Blank text returns nothing.
    pub async fn search_semantic_frames(
        &self,
        query: &SemanticItemQuery,
    ) -> Result<Vec<SemanticFrameHit>> {
        let fts = sanitize_fts5_query(&query.text);
        if fts.is_empty() {
            return Ok(Vec::new());
        }
        if let Some(actor_id) = query.actor_id {
            require_positive("actor_id", actor_id)?;
        }
        let (limit, offset) = page(query.limit, query.offset);
        let rows: Vec<(i64, DateTime<Utc>, i64, String)> = sqlx::query_as(
            "SELECT f.id, f.timestamp, r.id, group_concat(DISTINCT ri.item_id) \
             FROM semantic_items_fts \
             JOIN semantic_items i ON i.id = semantic_items_fts.rowid \
             JOIN semantic_run_items ri ON ri.item_id = i.id \
             JOIN semantic_runs r ON r.id = ri.run_id AND r.status = 'handled' \
             JOIN frames f ON f.semantic_run_id = r.id \
             LEFT JOIN semantic_item_actors ia ON ia.item_id = i.id \
             WHERE semantic_items_fts MATCH ?1 \
               AND (?2 IS NULL OR ia.actor_id = ?2) \
               AND (?3 IS NULL OR i.kind = ?3) \
             GROUP BY f.id \
             ORDER BY f.timestamp DESC, f.id DESC \
             LIMIT ?4 OFFSET ?5",
        )
        .bind(fts)
        .bind(query.actor_id)
        .bind(query.kind.map(|k| k.as_str()))
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(frame_id, timestamp, run_id, ids)| {
                let mut item_ids: Vec<i64> = ids
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
                item_ids.sort_unstable();
                SemanticFrameHit {
                    frame_id,
                    timestamp,
                    run_id,
                    item_ids,
                }
            })
            .collect())
    }

    /// Delete runs no frame points at, then items no run contains, in batches
    /// of a few thousand rows per transaction.
    ///
    /// Runs younger than `grace` survive even when unreferenced, so a run the
    /// worker stored but has not attached yet is not lost. Deleting a run
    /// removes its run items (cascade); deleting an item removes its actor
    /// assignment (cascade) and its FTS row (trigger). Actors and aliases are
    /// kept on purpose.
    ///
    /// The grace counts from the run's `created_at`, not from its last use:
    /// the schema records no use time. A run the worker reuses (a fingerprint
    /// hit, or `store_semantic_run` returning `created == false`) can be
    /// older than the grace and, if retention just removed the frames that
    /// pointed at it, unreferenced. A pass before the next flush then
    /// collects it, and the flush reports the frames still waiting on it in
    /// [`SemanticAttachOutcome::skipped`] instead of attaching them. The
    /// worker must store or parse those frames again.
    ///
    /// Which grace to pass depends on why frames went away:
    ///
    /// - Periodic retention passes (old frames aged out on a schedule) pass
    ///   [`SEMANTIC_GC_DEFAULT_GRACE`]. That is what the grace is for: the
    ///   worker may be holding a fresh run in its attachment buffer.
    /// - User-initiated deletes (W3-09: delete-range, `delete_by_machine_id`)
    ///   must pass `Duration::ZERO`, so the deleted frames' runs and items,
    ///   and their text in the FTS index, are gone when the call returns
    ///   instead of lingering until a later pass. The cost: a run the worker
    ///   stored moments ago for a frame outside the deleted range, and has
    ///   not attached yet, is collected too. Its pending attachment is then
    ///   skipped and reported (see [`Self::attach_semantic_runs_to_frames`]),
    ///   so the worker can store or parse that frame again. Pending
    ///   attachments of the deleted frames themselves are dropped without
    ///   being reported, so the worker never stores their text again, and a
    ///   parse it stored for one of them after this pass is deleted by the
    ///   flush that tries to attach it.
    pub async fn gc_semantic_projection(&self, grace: Duration) -> Result<SemanticGcStats> {
        let grace =
            chrono::Duration::from_std(grace).unwrap_or_else(|_| chrono::Duration::days(365 * 100));
        let cutoff = Utc::now()
            .checked_sub_signed(grace)
            .unwrap_or(DateTime::<Utc>::MIN_UTC);
        let cutoff = format_timestamp(&cutoff);

        let mut stats = SemanticGcStats::default();
        loop {
            let mut tx = self.begin_immediate_with_retry().await?;
            let out = sqlx::query(
                "DELETE FROM semantic_runs WHERE id IN ( \
                     SELECT r.id FROM semantic_runs r \
                     WHERE NOT EXISTS ( \
                         SELECT 1 FROM frames WHERE frames.semantic_run_id = r.id) \
                       AND r.created_at <= ?1 \
                     LIMIT ?2)",
            )
            .bind(&cutoff)
            .bind(GC_BATCH)
            .execute(&mut **tx.conn())
            .await
            .map(|r| r.rows_affected())
            .map_err(SemanticStoreError::from);
            let deleted = finish_tx(tx, out).await?;
            stats.runs_deleted += deleted;
            if deleted < GC_BATCH as u64 {
                break;
            }
        }
        loop {
            let mut tx = self.begin_immediate_with_retry().await?;
            let out = sqlx::query(
                "DELETE FROM semantic_items WHERE id IN ( \
                     SELECT i.id FROM semantic_items i \
                     WHERE NOT EXISTS ( \
                         SELECT 1 FROM semantic_run_items \
                         WHERE semantic_run_items.item_id = i.id) \
                     LIMIT ?1)",
            )
            .bind(GC_BATCH)
            .execute(&mut **tx.conn())
            .await
            .map(|r| r.rows_affected())
            .map_err(SemanticStoreError::from);
            let deleted = finish_tx(tx, out).await?;
            stats.items_deleted += deleted;
            if deleted < GC_BATCH as u64 {
                break;
            }
        }
        if stats.runs_deleted > 0 || stats.items_deleted > 0 {
            info!(
                runs = stats.runs_deleted,
                items = stats.items_deleted,
                "semantic gc removed unreferenced rows"
            );
        }
        Ok(stats)
    }

    // ----- actors ---------------------------------------------------------

    /// Actors whose name or any alias's observed label contains `q`
    /// (case-insensitive for ASCII), most assigned items first, then by name
    /// and id. A blank `q` lists every actor. `q` may be up to 256 characters.
    pub async fn search_semantic_actors(
        &self,
        q: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SemanticActorSummary>> {
        let q = q.trim();
        if q.chars().count() > SEMANTIC_ACTOR_QUERY_MAX_CHARS {
            return Err(SemanticStoreError::invalid(format!(
                "q must be at most {SEMANTIC_ACTOR_QUERY_MAX_CHARS} characters"
            )));
        }
        let pattern = (!q.is_empty()).then(|| like_contains_pattern(q));
        let (limit, offset) = page(limit, offset);
        let mut conn = self.pool.acquire().await?;
        let rows = sqlx::query(
            "SELECT a.id, a.name, a.created_at, a.updated_at, \
                    (SELECT COUNT(*) FROM semantic_item_actors ia WHERE ia.actor_id = a.id) \
                        AS item_count \
             FROM semantic_actors a \
             WHERE ?1 IS NULL \
                OR a.name LIKE ?1 ESCAPE '\\' \
                OR EXISTS (SELECT 1 FROM semantic_actor_aliases al \
                           WHERE al.actor_id = a.id \
                             AND al.observed_label LIKE ?1 ESCAPE '\\') \
             ORDER BY item_count DESC, a.name COLLATE NOCASE, a.id \
             LIMIT ?2 OFFSET ?3",
        )
        .bind(pattern)
        .bind(limit)
        .bind(offset)
        .fetch_all(&mut *conn)
        .await?;

        let mut summaries = Vec::with_capacity(rows.len());
        for row in &rows {
            let actor = actor_from_row(row)?;
            summaries.push(SemanticActorSummary {
                id: actor.id,
                name: actor.name,
                created_at: actor.created_at,
                updated_at: actor.updated_at,
                item_count: row.try_get("item_count")?,
                aliases: Vec::new(),
            });
        }
        let ids: Vec<i64> = summaries.iter().map(|s| s.id).collect();
        let mut aliases = aliases_for_actors(&mut conn, &ids).await?;
        for s in &mut summaries {
            s.aliases = aliases.remove(&s.id).unwrap_or_default();
        }
        Ok(summaries)
    }

    /// One actor with its item count and aliases.
    pub async fn get_semantic_actor(&self, id: i64) -> Result<Option<SemanticActorSummary>> {
        require_positive("id", id)?;
        let mut conn = self.pool.acquire().await?;
        let Some(actor) = fetch_actor(&mut conn, id).await? else {
            return Ok(None);
        };
        let item_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM semantic_item_actors WHERE actor_id = ?1")
                .bind(id)
                .fetch_one(&mut *conn)
                .await?;
        let aliases = aliases_for_actors(&mut conn, &[id])
            .await?
            .remove(&id)
            .unwrap_or_default();
        Ok(Some(SemanticActorSummary {
            id: actor.id,
            name: actor.name,
            created_at: actor.created_at,
            updated_at: actor.updated_at,
            item_count,
            aliases,
        }))
    }

    /// The alias a label resolves to under `source_key`, without creating
    /// anything. This is the lookup the store-time heuristic performs.
    pub async fn find_semantic_actor_alias(
        &self,
        source_key: &str,
        label: &str,
    ) -> Result<Option<SemanticActorAlias>> {
        let Some(labels) = AliasLabels::from_label(label) else {
            return Ok(None);
        };
        let sql = format!(
            "SELECT {ALIAS_COLUMNS} FROM semantic_actor_aliases \
             WHERE source_key = ?1 AND normalized_label = ?2"
        );
        let row = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(source_key.trim())
            .bind(labels.normalized_label)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(alias_from_row).transpose()
    }

    /// Create a separate actor. The name is trimmed and must be 1 to 256
    /// characters.
    pub async fn create_semantic_actor(&self, name: &str) -> Result<SemanticActor> {
        let name = clean_actor_name(name)?;
        let mut tx = self.begin_immediate_with_retry().await?;
        let out = create_actor_tx(tx.conn(), &name).await;
        finish_tx(tx, out).await
    }

    /// Rename an actor and bump its `updated_at`. The name is trimmed and
    /// must be 1 to 256 characters.
    pub async fn rename_semantic_actor(&self, id: i64, name: &str) -> Result<SemanticActor> {
        require_positive("id", id)?;
        let name = clean_actor_name(name)?;
        let mut tx = self.begin_immediate_with_retry().await?;
        let out = rename_actor_tx(tx.conn(), id, &name).await;
        finish_tx(tx, out).await
    }

    /// Fold `merge_id` into `keep_id`: its aliases and item assignments move to
    /// the kept actor (assignment sources are preserved), the merged actor is
    /// deleted and the kept one is touched. Later observations of the moved
    /// aliases resolve to the kept actor.
    pub async fn merge_semantic_actors(
        &self,
        keep_id: i64,
        merge_id: i64,
    ) -> Result<SemanticActorMerge> {
        require_positive("actor_to_keep_id", keep_id)?;
        require_positive("actor_to_merge_id", merge_id)?;
        if keep_id == merge_id {
            return Err(SemanticStoreError::invalid(
                "actor_to_keep_id and actor_to_merge_id must differ",
            ));
        }
        let mut tx = self.begin_immediate_with_retry().await?;
        let out = merge_tx(tx.conn(), keep_id, merge_id).await;
        finish_tx(tx, out).await
    }

    /// Explicitly assign an item to an actor (`assignment_source =
    /// 'reconciled'`). The item must carry an observed actor label. A
    /// reconciled assignment is never changed by the heuristic or by alias
    /// moves. The observed alias of an existing assignment is kept.
    pub async fn reassign_semantic_item_actor(
        &self,
        item_id: i64,
        actor_id: i64,
    ) -> Result<SemanticItemActorReassignment> {
        require_positive("item_id", item_id)?;
        require_positive("actor_id", actor_id)?;
        let mut tx = self.begin_immediate_with_retry().await?;
        let out = reassign_item_tx(tx.conn(), item_id, actor_id).await;
        finish_tx(tx, out).await
    }

    /// Move an alias to another actor together with every heuristic
    /// assignment made through it. Reconciled assignments stay where they
    /// are. Later observations of the alias resolve to the new actor.
    pub async fn reassign_semantic_actor_alias(
        &self,
        alias_id: i64,
        actor_id: i64,
    ) -> Result<SemanticAliasReassignment> {
        require_positive("alias_id", alias_id)?;
        require_positive("actor_id", actor_id)?;
        let mut tx = self.begin_immediate_with_retry().await?;
        let out = reassign_alias_tx(tx.conn(), alias_id, actor_id).await;
        finish_tx(tx, out).await
    }
}

// ---------------------------------------------------------------------------
// Batched frame attachment
// ---------------------------------------------------------------------------

/// Collects frame-to-run attachments so the worker writes them in batches
/// instead of one UPDATE per frame.
///
/// A later push for the same frame replaces the earlier one. The buffer is
/// due once it holds `max_len` frames or its oldest entry is `max_age` old.
#[derive(Debug)]
pub struct SemanticFrameAttachmentBuffer {
    pending: HashMap<i64, i64>,
    oldest: Option<Instant>,
    max_len: usize,
    max_age: Duration,
}

impl SemanticFrameAttachmentBuffer {
    pub fn new(max_len: usize, max_age: Duration) -> Self {
        Self {
            pending: HashMap::new(),
            oldest: None,
            max_len: max_len.max(1),
            max_age,
        }
    }

    /// Queue an attachment. Returns whether the buffer is now due for a
    /// flush.
    ///
    /// Ids must be positive, as [`DatabaseManager::attach_semantic_runs_to_frames`]
    /// requires. An invalid pair is rejected here with
    /// [`SemanticStoreError::InvalidInput`] and the buffer is left unchanged,
    /// so one bad entry can never cost the rest of a batch at flush time.
    pub fn push(&mut self, frame_id: i64, run_id: i64) -> Result<bool> {
        require_positive("frame_id", frame_id)?;
        require_positive("run_id", run_id)?;
        self.pending.insert(frame_id, run_id);
        self.oldest.get_or_insert_with(Instant::now);
        Ok(self.is_due(Instant::now()))
    }

    /// Frames waiting to be attached.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// True when the buffer is full or its oldest entry has waited `max_age`.
    pub fn is_due(&self, now: Instant) -> bool {
        if self.pending.len() >= self.max_len {
            return true;
        }
        match self.oldest {
            Some(t) => now.saturating_duration_since(t) >= self.max_age,
            None => false,
        }
    }

    /// Everything queued, ordered by frame id, without removing it.
    fn queued(&self) -> Vec<SemanticFrameAttachment> {
        let mut out: Vec<SemanticFrameAttachment> = self
            .pending
            .iter()
            .map(|(&frame_id, &run_id)| SemanticFrameAttachment { frame_id, run_id })
            .collect();
        out.sort_unstable_by_key(|a| a.frame_id);
        out
    }

    /// Remove and return everything queued, ordered by frame id.
    pub fn take(&mut self) -> Vec<SemanticFrameAttachment> {
        let out = self.queued();
        self.pending.clear();
        self.oldest = None;
        out
    }

    /// Write everything queued in one transaction and log the flush (at
    /// debug level), with the rules of
    /// [`DatabaseManager::attach_semantic_runs_to_frames`]: entries whose run
    /// GC removed in the meantime are not written, and those whose frame
    /// still exists come back in [`SemanticAttachOutcome::skipped`] for the
    /// caller to store or parse again. Entries whose frame was deleted are
    /// dropped without being reported.
    ///
    /// The entries leave the buffer only after the transaction commits:
    ///
    /// - [`Self::push`] only queues valid ids, so a failed flush is a
    ///   database failure (locked, busy, I/O). The transaction rolls back,
    ///   every entry stays queued and the age clock restarts, so the next
    ///   flush that is due retries the whole batch.
    /// - Cancel safety: dropping the future before it completes (a task
    ///   abort, or the losing branch of a `tokio::select!`) loses nothing.
    ///   An unfinished transaction rolls back and every entry stays queued.
    ///   A drop that lands while the commit itself is in flight may leave the
    ///   rows written with the entries still queued; the next flush then
    ///   writes the same values again, which changes nothing.
    pub async fn flush(&mut self, db: &DatabaseManager) -> Result<SemanticAttachOutcome> {
        if self.pending.is_empty() {
            return Ok(SemanticAttachOutcome::default());
        }
        let batch = self.queued();
        match db.attach_semantic_runs_to_frames(&batch).await {
            Ok(outcome) => {
                // `&mut self` is held across the await, so nothing was pushed
                // in between: everything queued is in `batch`.
                self.pending.clear();
                self.oldest = None;
                debug!(
                    queued = batch.len(),
                    updated = outcome.frames_updated,
                    skipped = outcome.skipped.len(),
                    runs_discarded = outcome.runs_discarded,
                    items_discarded = outcome.items_discarded,
                    "semantic: flushed frame attachments"
                );
                Ok(outcome)
            }
            Err(e) => {
                self.oldest = Some(Instant::now());
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(n: u8) -> SemanticFingerprint {
        [n; 32]
    }

    fn item(local: &str, parent: Option<&str>) -> NewSemanticItem {
        let mut it = NewSemanticItem::new(
            local,
            SemanticItemKind::Message,
            local,
            SemanticIdentityQuality::Derived,
            fp(1),
            fp(2),
        );
        it.parent_local_id = parent.map(str::to_string);
        it
    }

    fn run(items: Vec<NewSemanticItem>) -> NewSemanticRun {
        NewSemanticRun {
            input_fingerprint: fp(9),
            parser_id: "p".into(),
            parser_version: "1".into(),
            schema_version: 1,
            app_platform: SemanticAppPlatform::Windows,
            app_id: None,
            app_executable: None,
            app_version: None,
            input_content_hash: [0; 8],
            status: if items.is_empty() {
                SemanticRunStatus::Empty
            } else {
                SemanticRunStatus::Handled
            },
            text_bytes: 0,
            parse_duration_us: 0,
            actor_source_key: None,
            items,
        }
    }

    #[test]
    fn stored_text_round_trips_and_rejects_unknown_values() {
        for k in SemanticItemKind::ALL {
            assert_eq!(SemanticItemKind::from_stored(k.as_str()).unwrap(), *k);
        }
        for p in SemanticAppPlatform::ALL {
            assert_eq!(p.as_str().parse::<SemanticAppPlatform>().unwrap(), *p);
        }
        assert_eq!(SemanticItemKind::CalendarEvent.as_str(), "calendar_event");
        let err = SemanticAppPlatform::from_stored("android").unwrap_err();
        assert!(matches!(
            err,
            SemanticStoreError::InvalidStoredValue {
                column: "semantic_runs.app_platform",
                ..
            }
        ));
        assert!(SemanticIdentityQuality::from_stored("Stable").is_err());
        assert_eq!(
            serde_json::to_string(&SemanticItemKind::CalendarEvent).unwrap(),
            "\"calendar_event\""
        );
    }

    #[test]
    fn source_node_ids_pack_little_endian() {
        let blob = encode_source_node_ids(&[1, 0x0102_0304]);
        assert_eq!(blob, vec![1, 0, 0, 0, 4, 3, 2, 1]);
        assert_eq!(decode_source_node_ids(&blob), Some(vec![1, 0x0102_0304]));
        assert_eq!(decode_source_node_ids(&[]), Some(vec![]));
        assert_eq!(decode_source_node_ids(&[1, 2, 3]), None);
        // The length check is the only guard: a big-endian blob of the same
        // length decodes without error to different ids. This is why the
        // layout is recorded as an open interop question.
        assert_eq!(
            decode_source_node_ids(&[0, 0, 0, 1]),
            Some(vec![0x0100_0000])
        );
    }

    #[test]
    fn labels_normalize_by_whitespace_invisibles_and_case() {
        assert_eq!(
            display_semantic_actor_label("  Ada \t\n Lovelace "),
            "Ada Lovelace"
        );
        assert_eq!(display_semantic_actor_label("Ada\u{200B}"), "Ada");
        assert_eq!(
            normalize_semantic_actor_label(" ADA  lovelace"),
            "ada lovelace"
        );
        assert_eq!(normalize_semantic_actor_label("\u{FEFF} \u{200C}"), "");
        assert!(AliasLabels::from_label("   ").is_none());

        let long = "x".repeat(300);
        let labels = AliasLabels::from_label(&long).unwrap();
        assert_eq!(labels.source_label.chars().count(), 256);
        assert_eq!(labels.observed_label.chars().count(), 256);
        assert_eq!(labels.normalized_label.chars().count(), 300);

        let labels = AliasLabels::from_label(" Ünïcode  Name ").unwrap();
        assert_eq!(labels.source_label, " Ünïcode  Name ");
        assert_eq!(labels.observed_label, "Ünïcode Name");
        assert_eq!(labels.normalized_label, "ünïcode name");
    }

    /// SQLite's `length()` of the stored text: characters before the first
    /// NUL. Every alias column must come out at 1 or more.
    fn sqlite_length(s: &str) -> usize {
        s.chars().take_while(|c| *c != '\0').count()
    }

    #[test]
    fn labels_drop_nul_and_other_control_characters() {
        // A leading NUL used to reach the alias table and fail its length
        // checks, rolling the whole run back.
        let labels = AliasLabels::from_label("\0Ada").unwrap();
        assert_eq!(labels.source_label, "Ada");
        assert_eq!(labels.observed_label, "Ada");
        assert_eq!(labels.normalized_label, "ada");
        for column in [
            &labels.source_label,
            &labels.observed_label,
            &labels.normalized_label,
        ] {
            assert!(sqlite_length(column) >= 1, "{column:?}");
        }

        // Only non-whitespace control characters: no visible label at all.
        assert!(AliasLabels::from_label("\u{1}\u{2}").is_none());
        assert!(AliasLabels::from_label("\0").is_none());
        assert!(AliasLabels::from_label("\0 \u{7F}\u{9F}\u{200B}").is_none());
        assert_eq!(display_semantic_actor_label("\u{1}\u{2}"), "");

        // Controls inside a label vanish; whitespace controls still count as
        // whitespace, both in the display form and kept in source_label.
        let labels = AliasLabels::from_label("A\u{7}da\u{1B}\tLove\u{0}lace").unwrap();
        assert_eq!(labels.source_label, "Ada\tLovelace");
        assert_eq!(labels.observed_label, "Ada Lovelace");
        assert_eq!(labels.normalized_label, "ada lovelace");

        // Directional marks wrapped around a phone number or name are
        // invisible: they neither form a label on their own nor split one
        // alias into two.
        assert!(AliasLabels::from_label("\u{202A}\u{202C}").is_none());
        assert_eq!(
            normalize_semantic_actor_label("\u{202A}+1 555 0100\u{202C}"),
            normalize_semantic_actor_label("+1 555 0100")
        );
        assert_eq!(display_semantic_actor_label("\u{200F}Dana\u{00AD}"), "Dana");

        // A long invisible prefix still leaves every column non-empty.
        let padded = format!("{}\0Ada", "\u{200B}".repeat(300));
        let labels = AliasLabels::from_label(&padded).unwrap();
        assert_eq!(labels.source_label.chars().count(), 256);
        assert_eq!(sqlite_length(&labels.source_label), 256);
        assert_eq!(labels.observed_label, "Ada");
    }

    #[test]
    fn actor_names_drop_controls_and_need_a_visible_character() {
        assert_eq!(clean_actor_name("  Ada Lovelace ").unwrap(), "Ada Lovelace");
        assert_eq!(clean_actor_name("\0Ada").unwrap(), "Ada");
        assert_eq!(clean_actor_name("Ada\u{0}\u{1}").unwrap(), "Ada");
        for bad in ["", "   ", "\u{1}\u{2}", "\0", "\u{200B}", "\u{1} \u{202A}"] {
            let err = clean_actor_name(bad).unwrap_err();
            assert!(err.is_invalid_input(), "{bad:?}: {err}");
        }
        assert!(clean_actor_name(&"n".repeat(257)).is_err());
        // Controls do not count toward the limit.
        let padded = format!("{}\u{1}", "n".repeat(256));
        assert_eq!(clean_actor_name(&padded).unwrap().chars().count(), 256);
    }

    #[test]
    fn clip_respects_char_boundaries() {
        assert_eq!(clip_chars("héllo", 2), "hé");
        assert_eq!(clip_chars("hi", 5), "hi");
    }

    #[test]
    fn in_list_placeholders_are_numbered() {
        assert_eq!(numbered_placeholders(2, 3), "?2, ?3, ?4");
        assert_eq!(numbered_placeholders(1, 1), "?1");
    }

    #[test]
    fn like_pattern_escapes_wildcards() {
        assert_eq!(like_contains_pattern("a%b_c\\"), "%a\\%b\\_c\\\\%");
    }

    #[test]
    fn validation_rejects_bad_runs() {
        assert!(validate_new_run(&run(vec![item("a", None), item("b", Some("a"))])).is_ok());
        assert!(validate_new_run(&run(vec![])).is_ok());

        let dup = run(vec![item("a", None), item("a", None)]);
        assert!(validate_new_run(&dup).unwrap_err().is_invalid_input());

        let orphan = run(vec![item("a", Some("zz"))]);
        assert!(validate_new_run(&orphan).unwrap_err().is_invalid_input());

        let self_parent = run(vec![item("a", Some("a"))]);
        assert!(validate_new_run(&self_parent).is_err());

        let cycle = run(vec![item("a", Some("b")), item("b", Some("a"))]);
        assert!(validate_new_run(&cycle).is_err());

        let mut empty_with_items = run(vec![item("a", None)]);
        empty_with_items.status = SemanticRunStatus::Empty;
        assert!(validate_new_run(&empty_with_items).is_err());

        let mut handled_without_items = run(vec![]);
        handled_without_items.status = SemanticRunStatus::Handled;
        assert!(validate_new_run(&handled_without_items).is_err());

        let mut bad_schema = run(vec![]);
        bad_schema.schema_version = 0;
        assert!(validate_new_run(&bad_schema).is_err());

        let mut huge = run(vec![]);
        huge.text_bytes = u64::MAX;
        assert!(validate_new_run(&huge).is_err());

        // A bad explicit key may come from screen text: it never fails the
        // run (heuristic assignment is skipped instead, see the integration
        // tests).
        let mut long_key = run(vec![]);
        long_key.actor_source_key = Some("k".repeat(1025));
        assert!(validate_new_run(&long_key).is_ok());
        let mut nul_key = run(vec![]);
        nul_key.actor_source_key = Some("\0slack:T123".into());
        assert!(validate_new_run(&nul_key).is_ok());

        // A bad parser id as the default key is a caller bug.
        let mut nul_parser = run(vec![]);
        nul_parser.parser_id = "chat\u{1}".into();
        let err = validate_new_run(&nul_parser).unwrap_err();
        assert!(err.to_string().contains("parser_id"), "{err}");
        let mut long_parser = run(vec![]);
        long_parser.parser_id = "p".repeat(1025);
        let err = validate_new_run(&long_parser).unwrap_err();
        assert!(err.to_string().contains("parser_id"), "{err}");
        // With a valid explicit key the parser id is not an alias key.
        long_parser.actor_source_key = Some("slack:T123".into());
        assert!(validate_new_run(&long_parser).is_ok());
    }

    #[test]
    fn source_key_problems() {
        assert_eq!(source_key_problem("slack:T123"), None);
        assert_eq!(source_key_problem(&"k".repeat(1024)), None);
        assert!(source_key_problem(&"k".repeat(1025))
            .unwrap()
            .contains("at most 1024"));
        for bad in ["\0slack", "slack\u{1}", "a\tb", "a\nb", "\u{7f}"] {
            assert!(
                source_key_problem(bad)
                    .unwrap()
                    .contains("control characters"),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn source_key_defaults_to_parser_id() {
        let mut r = run(vec![]);
        assert_eq!(r.effective_actor_source_key(), "p");
        r.actor_source_key = Some("  ".into());
        assert_eq!(r.effective_actor_source_key(), "p");
        r.actor_source_key = Some("slack:T123".into());
        assert_eq!(r.effective_actor_source_key(), "slack:T123");
    }

    #[test]
    fn attachment_buffer_coalesces_and_reports_due() {
        let mut buf = SemanticFrameAttachmentBuffer::new(3, Duration::from_secs(3600));
        assert!(!buf.is_due(Instant::now()));
        assert!(!buf.push(1, 10).unwrap());
        assert!(!buf.push(1, 11).unwrap());
        assert_eq!(buf.len(), 1);
        assert!(!buf.push(2, 10).unwrap());
        assert!(buf.push(3, 10).unwrap());
        let batch = buf.take();
        assert_eq!(
            batch,
            vec![
                SemanticFrameAttachment {
                    frame_id: 1,
                    run_id: 11
                },
                SemanticFrameAttachment {
                    frame_id: 2,
                    run_id: 10
                },
                SemanticFrameAttachment {
                    frame_id: 3,
                    run_id: 10
                },
            ]
        );
        assert!(buf.is_empty());

        let mut aged = SemanticFrameAttachmentBuffer::new(100, Duration::ZERO);
        assert!(aged.push(5, 1).unwrap());
    }

    #[test]
    fn attachment_buffer_rejects_invalid_ids_and_keeps_valid_ones() {
        let mut buf = SemanticFrameAttachmentBuffer::new(10, Duration::from_secs(3600));
        assert!(!buf.push(1, 10).unwrap());
        for (frame_id, run_id) in [(0, 10), (-5, 10), (2, 0), (2, -1)] {
            let err = buf.push(frame_id, run_id).unwrap_err();
            assert!(err.is_invalid_input(), "({frame_id}, {run_id}): {err}");
        }
        // A rejected push for a queued frame leaves its entry alone.
        assert!(buf.push(1, -3).is_err());
        assert!(!buf.push(3, 30).unwrap());
        assert_eq!(
            buf.take(),
            vec![
                SemanticFrameAttachment {
                    frame_id: 1,
                    run_id: 10
                },
                SemanticFrameAttachment {
                    frame_id: 3,
                    run_id: 30
                },
            ]
        );
        // Rejected pushes never start the age clock.
        let mut aged = SemanticFrameAttachmentBuffer::new(100, Duration::ZERO);
        assert!(aged.push(0, 1).is_err());
        assert!(!aged.is_due(Instant::now()));
        assert!(aged.is_empty());
    }

    #[test]
    fn fingerprints_serialize_as_hex() {
        #[derive(Serialize)]
        struct W {
            #[serde(serialize_with = "serialize_hex")]
            v: [u8; 3],
        }
        assert_eq!(
            serde_json::to_string(&W { v: [0, 0xab, 0x10] }).unwrap(),
            r#"{"v":"00ab10"}"#
        );
    }
}
