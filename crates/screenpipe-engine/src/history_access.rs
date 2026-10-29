// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! History access policy: every read of local history is allowed.
//!
//! screenpipe's hosted plans could cap how far back an account was allowed to
//! read. Frames, elements, exports, meetings, raw SQL and starred sessions
//! older than that window were refused with a fixed error code. Handvel has
//! no accounts and no plans, so it never limits history: data the user
//! recorded stays readable for as long as it is on disk.
//!
//! This module is the single place a route handler, the CLI or the desktop
//! app asks the question, so the answer cannot drift between them.
//! [`HistoryAccess`] deliberately has no restricted variant, which means no
//! caller can ever be handed a refusal. [`HistoryAccessErrorCode`] keeps the
//! refusal's wire code defined so clients written against screenpipe still
//! have something to match; nothing in Handvel produces it.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The kind of history read a caller is about to serve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HistoryRead {
    /// Keyword, semantic and element search.
    Search,
    /// Frame images, thumbnails, previews and frame metadata.
    Frames,
    /// Accessibility and OCR elements of a frame.
    Elements,
    /// Audio chunks and their transcriptions.
    Audio,
    /// Meeting records, transcripts and summaries.
    Meetings,
    /// Media exports of a time range.
    Export,
    /// Raw SQL against the local database.
    RawSql,
    /// Starred work sessions and their HD captures.
    StarredSessions,
    /// Activity timelines and summaries for a time range.
    Activity,
    /// The workflow catalog and workflow runs.
    Workflows,
}

impl HistoryRead {
    /// Every kind of read, for callers that describe the policy as a whole.
    pub const ALL: [HistoryRead; 10] = [
        HistoryRead::Search,
        HistoryRead::Frames,
        HistoryRead::Elements,
        HistoryRead::Audio,
        HistoryRead::Meetings,
        HistoryRead::Export,
        HistoryRead::RawSql,
        HistoryRead::StarredSessions,
        HistoryRead::Activity,
        HistoryRead::Workflows,
    ];
}

/// The answer to "may this read reach back that far?".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryAccess {
    /// History of any age may be read.
    Unrestricted,
}

/// Error codes a history-window refusal carries on the wire, as
/// `{"error": "...", "code": "<code>"}`.
///
/// Reserved for compatibility only: Handvel never refuses a read because of
/// its age, so no value of this type is ever sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryAccessErrorCode {
    /// The requested data lies outside the history window the account may see.
    #[serde(rename = "history_access_limited")]
    Limited,
}

/// Decides whether `read`, reaching back to `oldest` (`None` when the read has
/// no lower time bound), may go ahead. Always [`HistoryAccess::Unrestricted`].
pub fn access_for(_read: HistoryRead, _oldest: Option<DateTime<Utc>>) -> HistoryAccess {
    HistoryAccess::Unrestricted
}

/// The install-wide policy. Always [`HistoryAccess::Unrestricted`].
pub fn policy() -> HistoryAccess {
    HistoryAccess::Unrestricted
}

/// True when some reads of older history are refused on this install. This is
/// what the desktop app's `is_history_access_restricted` command reports, and
/// it is always false.
pub fn is_restricted() -> bool {
    policy() != HistoryAccess::Unrestricted
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};
    use std::collections::HashSet;

    fn reach_backs() -> Vec<Option<DateTime<Utc>>> {
        let now = Utc::now();
        vec![
            None,
            Some(now),
            Some(now - Duration::hours(23)),
            Some(now - Duration::hours(25)),
            Some(now - Duration::days(400)),
            Some(Utc.timestamp_opt(0, 0).unwrap()),
            // A clock skewed into the future must not trip anything either.
            Some(now + Duration::days(1)),
        ]
    }

    #[test]
    fn every_caller_gets_unrestricted() {
        for read in HistoryRead::ALL {
            for oldest in reach_backs() {
                assert_eq!(
                    access_for(read, oldest),
                    HistoryAccess::Unrestricted,
                    "{read:?} reaching back to {oldest:?}"
                );
            }
        }
    }

    #[test]
    fn install_wide_policy_is_unrestricted() {
        assert_eq!(policy(), HistoryAccess::Unrestricted);
        assert!(!is_restricted());
    }

    #[test]
    fn every_read_kind_is_listed_once() {
        let distinct: HashSet<_> = HistoryRead::ALL.iter().collect();
        assert_eq!(distinct.len(), HistoryRead::ALL.len());
    }

    #[test]
    fn policy_serializes_in_snake_case() {
        assert_eq!(
            serde_json::to_value(HistoryAccess::Unrestricted).unwrap(),
            serde_json::json!("unrestricted")
        );
    }

    #[test]
    fn reserved_refusal_code_keeps_its_wire_name() {
        // Built from two halves so the code's only literal stays on the enum.
        let wire = concat!("history_access", "_limited");
        assert_eq!(
            serde_json::to_value(HistoryAccessErrorCode::Limited).unwrap(),
            serde_json::json!(wire)
        );
        let parsed: HistoryAccessErrorCode =
            serde_json::from_value(serde_json::json!(wire)).unwrap();
        assert_eq!(parsed, HistoryAccessErrorCode::Limited);
    }
}
