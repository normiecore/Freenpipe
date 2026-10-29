// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Tag lookups added after the base: autocomplete with usage counts.
//!
//! Tags live in two stores that share one name space:
//!
//! - the `tags` table, attached to frames through `vision_tags` and to audio
//!   chunks through `audio_tags` (`POST /tags/:content_type/:id`);
//! - the JSON `tags` array on each `memories` row.
//!
//! [`DatabaseManager::autocomplete_tags`] merges the two by exact name.

use sqlx::FromRow;

use crate::DatabaseManager;

/// Entries returned when the caller gives no limit.
pub const DEFAULT_TAG_AUTOCOMPLETE_LIMIT: u32 = 50;
/// The most entries one autocomplete call returns.
pub const MAX_TAG_AUTOCOMPLETE_LIMIT: u32 = 500;

/// One tag name and how many frames, audio chunks and memories carry it.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct TagUsage {
    pub name: String,
    pub frame_count: i64,
    pub audio_count: i64,
    pub memory_count: i64,
}

impl TagUsage {
    /// Uses across all three stores.
    pub fn total(&self) -> i64 {
        self.frame_count + self.audio_count + self.memory_count
    }
}

// `?1` is the search text, LIKE-escaped with `\`; `?2` is the row limit.
//
// Matching is a case-insensitive (ASCII) substring test. Ranking puts names
// that start with the text, or whose part after a `namespace:` does, ahead
// of other matches; then the most used; then by name. Tags whose junction
// rows were all removed have no uses left and are not listed. A memory whose
// `tags` column is not a JSON array contributes nothing instead of failing
// the query, and a name listed twice in one memory counts that memory once.
const AUTOCOMPLETE_SQL: &str = r#"
WITH junction AS (
    SELECT t.name AS name,
           (SELECT COUNT(*) FROM vision_tags vt WHERE vt.tag_id = t.id) AS frame_count,
           (SELECT COUNT(*) FROM audio_tags atg WHERE atg.tag_id = t.id) AS audio_count,
           0 AS memory_count
    FROM tags t
    WHERE ?1 = '' OR t.name LIKE '%' || ?1 || '%' ESCAPE '\'
),
memory AS (
    SELECT j.value AS name,
           0 AS frame_count,
           0 AS audio_count,
           COUNT(DISTINCT m.id) AS memory_count
    FROM memories m,
         json_each(
             CASE WHEN json_valid(m.tags)
                  THEN CASE WHEN json_type(m.tags) = 'array' THEN m.tags ELSE '[]' END
                  ELSE '[]'
             END
         ) j
    WHERE j.type = 'text'
      AND j.value <> ''
      AND (?1 = '' OR j.value LIKE '%' || ?1 || '%' ESCAPE '\')
    GROUP BY j.value
),
merged AS (
    SELECT name,
           SUM(frame_count) AS frame_count,
           SUM(audio_count) AS audio_count,
           SUM(memory_count) AS memory_count
    FROM (SELECT * FROM junction UNION ALL SELECT * FROM memory)
    GROUP BY name
)
SELECT name, frame_count, audio_count, memory_count
FROM merged
WHERE frame_count + audio_count + memory_count > 0
ORDER BY
    CASE
        WHEN ?1 = '' THEN 0
        WHEN name LIKE ?1 || '%' ESCAPE '\' THEN 0
        WHEN name LIKE '%:' || ?1 || '%' ESCAPE '\' THEN 0
        ELSE 1
    END,
    frame_count + audio_count + memory_count DESC,
    name COLLATE NOCASE,
    name
LIMIT ?2
"#;

/// The search text a caller typed, as matched: surrounding space removed and
/// one leading `#` dropped, since tags are displayed as `#name`.
pub fn normalize_tag_query(query: &str) -> &str {
    let query = query.trim();
    query.strip_prefix('#').unwrap_or(query).trim_start()
}

/// Escape `\`, `%` and `_` so a LIKE pattern with `ESCAPE '\'` matches them
/// literally.
fn like_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

impl DatabaseManager {
    /// Tag names containing `query` (case-insensitively, after
    /// [`normalize_tag_query`]), each with its frame, audio and memory use
    /// counts. An empty query lists the most used tags. `limit` is clamped
    /// to 1..=[`MAX_TAG_AUTOCOMPLETE_LIMIT`].
    pub async fn autocomplete_tags(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<TagUsage>, sqlx::Error> {
        let needle = like_escape(normalize_tag_query(query));
        let limit = limit.clamp(1, MAX_TAG_AUTOCOMPLETE_LIMIT);
        sqlx::query_as::<_, TagUsage>(AUTOCOMPLETE_SQL)
            .bind(needle)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_text_is_trimmed_and_loses_one_hash() {
        assert_eq!(normalize_tag_query("  wo "), "wo");
        assert_eq!(normalize_tag_query("#wo"), "wo");
        assert_eq!(normalize_tag_query("# wo"), "wo");
        assert_eq!(normalize_tag_query("##wo"), "#wo");
        assert_eq!(normalize_tag_query(""), "");
        assert_eq!(normalize_tag_query("#"), "");
    }

    #[test]
    fn like_wildcards_are_escaped() {
        assert_eq!(like_escape("100%_done\\"), "100\\%\\_done\\\\");
        assert_eq!(like_escape("person:ada"), "person:ada");
    }
}
