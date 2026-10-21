// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Frame media lookups for thumbnails, preview samples and preview media.
//!
//! A frame's pixels live in one of two places:
//!
//! * a **snapshot** JPEG named by `frames.snapshot_path` (recent frames), or
//! * a **video chunk**, an MP4 named by `video_chunks.file_path`, at frame
//!   position `frames.offset_index` (frames that snapshot compaction folded
//!   into an MP4, and legacy video recordings).
//!
//! Compaction moves a frame from the first to the second while the engine is
//! serving it, and media retention can empty a chunk's `file_path`. Every
//! function here reads the current row, so callers can re-check a frame
//! whose file just disappeared. None of them touch the file system; checking
//! that a file exists is the caller's job.

use chrono::{DateTime, Duration, Utc};
use sqlx::{sqlite::SqliteRow, Row};

use crate::DatabaseManager;

// The queries below are assembled with `format!` from the constant fragments
// in this file only; every value from a caller is a bound parameter. That is
// what each `AssertSqlSafe` vouches for.

/// Columns selected for a [`FrameMediaRow`], against `frames f` left-joined
/// to `video_chunks vc`.
const MEDIA_COLUMNS: &str = r#"
    f.id AS frame_id,
    f.timestamp AS timestamp,
    COALESCE(NULLIF(f.device_name, ''), vc.device_name, '') AS device_name,
    NULLIF(f.snapshot_path, '') AS snapshot_path,
    f.video_chunk_id AS video_chunk_id,
    NULLIF(vc.file_path, '') AS video_path,
    COALESCE(f.offset_index, 0) AS offset_index,
    vc.fps AS fps,
    f.browser_url AS browser_url
"#;

/// A frame filter that keeps frames with a local media location: a snapshot,
/// or a video chunk that still has its file, and neither on another device.
const HAS_LOCAL_MEDIA: &str = r#"
    (NULLIF(f.snapshot_path, '') IS NOT NULL OR NULLIF(vc.file_path, '') IS NOT NULL)
    AND COALESCE(NULLIF(f.snapshot_path, ''), vc.file_path, '') NOT LIKE 'cloud://%'
"#;

/// How far (in frame ids) a neighbour search may scan from its anchor. Keeps
/// the scan bounded when the anchor's device has no other frames nearby.
pub const NEIGHBOUR_ID_WINDOW: i64 = 2_000;

/// How far in time a neighbour may be from its anchor and still stand in
/// for it.
pub const NEIGHBOUR_TIME_WINDOW_SECS: i64 = 10 * 60;

/// Path prefix of frames that were synced from another device and have no
/// local file.
pub const REMOTE_PATH_PREFIX: &str = "cloud://";

/// Most rows one domain-filtered preview lookup reads before it gives up.
/// The SQL `LIKE` pre-filter also passes URLs that only mention the domain
/// outside the host (`google.com/search?q=github.com`), so a lookup pages
/// past those until it has its exact matches, the range runs out, or this
/// many rows were read.
pub const PREVIEW_DOMAIN_SCAN_LIMIT: u32 = 5_000;

/// Page size of a domain-filtered preview lookup.
const PREVIEW_DOMAIN_PAGE: u32 = 64;

/// A frame together with where its pixels are stored.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameMediaRow {
    pub frame_id: i64,
    pub timestamp: DateTime<Utc>,
    /// Monitor the frame came from (the frame's own column, else its chunk's).
    pub device_name: String,
    /// Snapshot JPEG; `None` when the column is NULL or empty.
    pub snapshot_path: Option<String>,
    pub video_chunk_id: Option<i64>,
    /// File of the video chunk; `None` when there is no chunk, or its media was
    /// evicted (retention empties the path).
    pub video_path: Option<String>,
    pub offset_index: i64,
    /// Frame rate of the video chunk, when there is one.
    pub fps: Option<f64>,
    pub browser_url: Option<String>,
}

/// Where a frame's pixels can be read from.
#[derive(Debug, Clone, PartialEq)]
pub enum FrameMediaLocation {
    /// A JPEG file holding exactly this frame.
    Snapshot { path: String },
    /// Frame `offset_index` of the MP4 at `path`.
    Video {
        chunk_id: i64,
        path: String,
        offset_index: i64,
        fps: Option<f64>,
    },
    /// The frame was synced from another device; no local file exists.
    Remote,
    /// The row has no media location at all (no snapshot, no chunk, or the
    /// chunk's media was evicted).
    Unavailable,
}

impl FrameMediaRow {
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            frame_id: row.try_get("frame_id")?,
            timestamp: row.try_get("timestamp")?,
            device_name: row.try_get("device_name")?,
            snapshot_path: row.try_get("snapshot_path")?,
            video_chunk_id: row.try_get("video_chunk_id")?,
            video_path: row.try_get("video_path")?,
            offset_index: row.try_get("offset_index")?,
            fps: row.try_get("fps")?,
            browser_url: row.try_get("browser_url")?,
        })
    }

    /// The frame's media location. A snapshot wins over a video chunk, the
    /// same order the frame image route has always used.
    pub fn location(&self) -> FrameMediaLocation {
        if let Some(path) = self.snapshot_path.as_deref() {
            if path.starts_with(REMOTE_PATH_PREFIX) {
                return FrameMediaLocation::Remote;
            }
            return FrameMediaLocation::Snapshot {
                path: path.to_string(),
            };
        }
        match (self.video_chunk_id, self.video_path.as_deref()) {
            (_, Some(path)) if path.starts_with(REMOTE_PATH_PREFIX) => FrameMediaLocation::Remote,
            (Some(chunk_id), Some(path)) => FrameMediaLocation::Video {
                chunk_id,
                path: path.to_string(),
                offset_index: self.offset_index,
                fps: self.fps,
            },
            _ => FrameMediaLocation::Unavailable,
        }
    }

    /// Position of the frame inside its video chunk, in seconds. `None` for a
    /// frame without a chunk, or a chunk whose frame rate is missing or not
    /// positive.
    pub fn video_offset_seconds(&self) -> Option<f64> {
        let fps = self.fps?;
        if !(fps.is_finite() && fps > 0.0) || self.offset_index < 0 {
            return None;
        }
        let seconds = self.offset_index as f64 / fps;
        seconds.is_finite().then_some(seconds)
    }
}

/// A video chunk as the preview-media route needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewMediaChunk {
    pub id: i64,
    /// Empty when retention evicted the chunk's media.
    pub file_path: String,
    pub evicted: bool,
}

/// What a preview-sample query selects.
#[derive(Debug, Clone, PartialEq)]
pub struct FramePreviewFilter<'a> {
    pub start: DateTime<Utc>,
    /// Exclusive.
    pub end: DateTime<Utc>,
    /// Matched case-insensitively against `frames.app_name`.
    pub app_name: &'a str,
    /// A normalised host (see [`normalize_browser_domain`]); frames whose
    /// `browser_url` host is this domain or one of its subdomains match.
    pub browser_domain: Option<&'a str>,
}

impl DatabaseManager {
    /// The exact frame with its current media location, or `None` when no
    /// frame has this id.
    pub async fn get_frame_media(
        &self,
        frame_id: i64,
    ) -> Result<Option<FrameMediaRow>, sqlx::Error> {
        let sql = format!(
            "SELECT {MEDIA_COLUMNS} FROM frames f \
             LEFT JOIN video_chunks vc ON vc.id = f.video_chunk_id \
             WHERE f.id = ?1"
        );
        let row = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(frame_id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(FrameMediaRow::from_row).transpose()
    }

    /// Frames next to `anchor` that could stand in for it: same device (when
    /// the anchor names one), at most [`NEIGHBOUR_TIME_WINDOW_SECS`] away, with
    /// a local media location. Nearest first; `forward` picks newer frames,
    /// otherwise older ones. The id scan is bounded by [`NEIGHBOUR_ID_WINDOW`].
    pub async fn get_frame_media_neighbours(
        &self,
        anchor: &FrameMediaRow,
        forward: bool,
        limit: u32,
    ) -> Result<Vec<FrameMediaRow>, sqlx::Error> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let (range, order) = if forward {
            ("f.id > ?1 AND f.id <= ?1 + ?2", "ASC")
        } else {
            ("f.id < ?1 AND f.id >= ?1 - ?2", "DESC")
        };
        let device_filter = if anchor.device_name.is_empty() {
            ""
        } else {
            "AND COALESCE(NULLIF(f.device_name, ''), vc.device_name, '') = ?4"
        };
        let sql = format!(
            "SELECT {MEDIA_COLUMNS} FROM frames f \
             LEFT JOIN video_chunks vc ON vc.id = f.video_chunk_id \
             WHERE {range} AND {HAS_LOCAL_MEDIA} {device_filter} \
             ORDER BY f.id {order} LIMIT ?3"
        );
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(anchor.frame_id)
            .bind(NEIGHBOUR_ID_WINDOW)
            .bind(i64::from(limit));
        if !anchor.device_name.is_empty() {
            query = query.bind(&anchor.device_name);
        }
        let rows = query.fetch_all(&self.pool).await?;
        let window = Duration::seconds(NEIGHBOUR_TIME_WINDOW_SECS);
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            let candidate = FrameMediaRow::from_row(row)?;
            if (candidate.timestamp - anchor.timestamp).abs() <= window {
                out.push(candidate);
            }
        }
        Ok(out)
    }

    /// Candidate frames for a preview strip, spread over the filter's range.
    ///
    /// The span between the first and the last matching frame is cut into
    /// `buckets` equal slices, and each slice contributes up to `per_bucket`
    /// frames in time order, so a caller that finds a file missing can move on
    /// to the next frame of the same slice. Slices without a matching frame
    /// are left out, so the result has at most `buckets` lists, each non-empty
    /// and in time order. Only frames with a local media location are
    /// returned; nothing is decoded or read from disk.
    pub async fn get_frame_preview_candidates(
        &self,
        filter: &FramePreviewFilter<'_>,
        buckets: u32,
        per_bucket: u32,
    ) -> Result<Vec<Vec<FrameMediaRow>>, sqlx::Error> {
        if buckets == 0 || per_bucket == 0 || filter.start >= filter.end {
            return Ok(Vec::new());
        }
        let like = filter.browser_domain.map(like_contains_pattern);

        let Some(first) = self
            .preview_edge_timestamp(filter, like.as_deref(), true)
            .await?
        else {
            return Ok(Vec::new());
        };
        let last = self
            .preview_edge_timestamp(filter, like.as_deref(), false)
            .await?
            .unwrap_or(first)
            .max(first);

        let mut out = Vec::new();
        for (bucket_start, bucket_end) in bucket_bounds(first, last, buckets) {
            let rows = self
                .preview_rows(
                    filter,
                    like.as_deref(),
                    bucket_start,
                    bucket_end,
                    per_bucket,
                    true,
                )
                .await?;
            if !rows.is_empty() {
                out.push(rows);
            }
        }
        Ok(out)
    }

    /// The video chunk with this id, or `None`.
    pub async fn get_preview_media_chunk(
        &self,
        video_chunk_id: i64,
    ) -> Result<Option<PreviewMediaChunk>, sqlx::Error> {
        let row: Option<(i64, Option<String>, bool)> = sqlx::query_as(
            "SELECT id, file_path, evicted_at IS NOT NULL FROM video_chunks WHERE id = ?1",
        )
        .bind(video_chunk_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(id, file_path, evicted)| PreviewMediaChunk {
            id,
            file_path: file_path.unwrap_or_default(),
            evicted,
        }))
    }

    /// Timestamp of the first (or last) frame matching `filter`.
    async fn preview_edge_timestamp(
        &self,
        filter: &FramePreviewFilter<'_>,
        like: Option<&str>,
        first: bool,
    ) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
        let rows = self
            .preview_rows(filter, like, filter.start, filter.end, 1, first)
            .await?;
        Ok(rows.into_iter().next().map(|r| r.timestamp))
    }

    /// Up to `want` frames in `[start, end)` that match `filter`, oldest
    /// first when `ascending`, else newest first.
    ///
    /// Without a domain every row the query returns matches, so one query
    /// with `LIMIT want` is enough. With a domain, SQL pre-filters with
    /// `LIKE` and each row's host is checked exactly here; rows are read a
    /// page at a time until `want` matches are found, the range has no more
    /// rows, or [`PREVIEW_DOMAIN_SCAN_LIMIT`] rows were read.
    async fn preview_rows(
        &self,
        filter: &FramePreviewFilter<'_>,
        like: Option<&str>,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        want: u32,
        ascending: bool,
    ) -> Result<Vec<FrameMediaRow>, sqlx::Error> {
        let want = want.max(1);
        let page = if like.is_some() {
            want.saturating_mul(4).max(PREVIEW_DOMAIN_PAGE)
        } else {
            want
        };
        let mut out = Vec::new();
        let mut offset = 0u32;
        loop {
            let rows = self
                .preview_page(filter, like, start, end, page, offset, ascending)
                .await?;
            let fetched = rows.len();
            for candidate in rows {
                let keep = match filter.browser_domain {
                    None => true,
                    Some(domain) => candidate
                        .browser_url
                        .as_deref()
                        .and_then(browser_url_host)
                        .is_some_and(|host| host_matches_domain(&host, domain)),
                };
                if keep {
                    out.push(candidate);
                    if out.len() >= want as usize {
                        return Ok(out);
                    }
                }
            }
            offset = offset.saturating_add(page);
            if like.is_none() || fetched < page as usize || offset >= PREVIEW_DOMAIN_SCAN_LIMIT {
                return Ok(out);
            }
        }
    }

    /// One page of [`Self::preview_rows`]: rows in `[start, end)` for the
    /// app (and the `LIKE` pattern when given), skipping `offset` rows.
    #[allow(clippy::too_many_arguments)]
    async fn preview_page(
        &self,
        filter: &FramePreviewFilter<'_>,
        like: Option<&str>,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        limit: u32,
        offset: u32,
        ascending: bool,
    ) -> Result<Vec<FrameMediaRow>, sqlx::Error> {
        let domain_filter = if like.is_some() {
            "AND f.browser_url LIKE ?6 ESCAPE '\\'"
        } else {
            ""
        };
        let order = if ascending { "ASC" } else { "DESC" };
        // `COLLATE NOCASE` matches the app name case-insensitively and keeps
        // the planner on the timestamp index rather than the app-name index,
        // which would walk every frame the app ever had.
        let sql = format!(
            "SELECT {MEDIA_COLUMNS} FROM frames f \
             LEFT JOIN video_chunks vc ON vc.id = f.video_chunk_id \
             WHERE f.timestamp >= ?1 AND f.timestamp < ?2 \
               AND f.app_name = ?3 COLLATE NOCASE \
               AND {HAS_LOCAL_MEDIA} {domain_filter} \
             ORDER BY f.timestamp {order}, f.id {order} LIMIT ?4 OFFSET ?5"
        );
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(start)
            .bind(end)
            .bind(filter.app_name)
            .bind(i64::from(limit.max(1)))
            .bind(i64::from(offset));
        if let Some(like) = like {
            query = query.bind(like);
        }
        let rows = query.fetch_all(&self.pool).await?;
        rows.iter().map(FrameMediaRow::from_row).collect()
    }
}

/// Splits `[first, last]` into `buckets` slices of equal length. Every slice
/// is half-open except the last, which is extended by one microsecond so it
/// still contains `last`. Slices that would be empty (a span shorter than
/// `buckets` microseconds) are dropped; `first == last` gives one slice.
pub fn bucket_bounds(
    first: DateTime<Utc>,
    last: DateTime<Utc>,
    buckets: u32,
) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    let tick = Duration::microseconds(1);
    if buckets == 0 {
        return Vec::new();
    }
    let last = last.max(first);
    let span_us = (last - first).num_microseconds().unwrap_or(i64::MAX);
    if span_us == 0 || buckets == 1 {
        return vec![(first, last + tick)];
    }
    let n = i128::from(buckets);
    let span = i128::from(span_us);
    (0..n)
        .map(|i| {
            // i * span / n never exceeds span, so it fits back into i64.
            let lo = first + Duration::microseconds((i * span / n) as i64);
            let hi = if i + 1 == n {
                last + tick
            } else {
                first + Duration::microseconds(((i + 1) * span / n) as i64)
            };
            (lo, hi)
        })
        .filter(|(lo, hi)| lo < hi)
        .collect()
}

/// `LIKE` pattern matching any text that contains `needle`, with `%`, `_`
/// and `\` escaped (use with `ESCAPE '\'`).
fn like_contains_pattern(needle: &str) -> String {
    let mut out = String::with_capacity(needle.len() + 2);
    out.push('%');
    for c in needle.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// Lowercased host of a browser URL, without port or trailing dot. Accepts
/// URLs without a scheme (`github.com/x`), as browsers sometimes report them.
pub fn browser_url_host(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = match url.find("://") {
        Some(i) => &url[i + 3..],
        None => url,
    };
    let authority = rest
        .split(|c| matches!(c, '/' | '?' | '#'))
        .next()
        .unwrap_or("");
    // Drop any userinfo.
    let host_port = authority.rsplit('@').next().unwrap_or("");
    let host = if let Some(stripped) = host_port.strip_prefix('[') {
        // IPv6 literal: keep what is inside the brackets.
        stripped.split(']').next().unwrap_or("")
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

/// True when `host` is `domain` or a subdomain of it. Both must already be
/// lowercase, as [`browser_url_host`] and [`normalize_browser_domain`] give.
pub fn host_matches_domain(host: &str, domain: &str) -> bool {
    if domain.is_empty() {
        return false;
    }
    host == domain
        || (host.len() > domain.len()
            && host.ends_with(domain)
            && host.as_bytes()[host.len() - domain.len() - 1] == b'.')
}

/// Validates a `browser_domain` query value and returns it lowercased,
/// without a trailing dot. Only a bare host is accepted: DNS labels of
/// letters, digits, `-` and `_` (1 to 63 characters each, no leading or
/// trailing `-`), at most 253 characters in all. Schemes, paths, ports,
/// spaces and wildcards are rejected.
pub fn normalize_browser_domain(input: &str) -> Option<String> {
    let host = input.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || host.len() > 253 {
        return None;
    }
    let labels_ok = host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    });
    labels_ok.then_some(host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_790_000_000 + secs, 0).unwrap()
    }

    fn row() -> FrameMediaRow {
        FrameMediaRow {
            frame_id: 7,
            timestamp: ts(0),
            device_name: "monitor_1".into(),
            snapshot_path: None,
            video_chunk_id: None,
            video_path: None,
            offset_index: 0,
            fps: None,
            browser_url: None,
        }
    }

    #[test]
    fn snapshot_location_wins_over_a_video_chunk() {
        let mut r = row();
        r.snapshot_path = Some("/d/a.jpg".into());
        r.video_chunk_id = Some(3);
        r.video_path = Some("/d/c.mp4".into());
        assert_eq!(
            r.location(),
            FrameMediaLocation::Snapshot {
                path: "/d/a.jpg".into()
            }
        );
    }

    #[test]
    fn video_location_and_offset_seconds() {
        let mut r = row();
        r.video_chunk_id = Some(3);
        r.video_path = Some("/d/c.mp4".into());
        r.offset_index = 5;
        r.fps = Some(0.5);
        assert_eq!(
            r.location(),
            FrameMediaLocation::Video {
                chunk_id: 3,
                path: "/d/c.mp4".into(),
                offset_index: 5,
                fps: Some(0.5)
            }
        );
        assert_eq!(r.video_offset_seconds(), Some(10.0));
        r.fps = Some(0.0);
        assert_eq!(r.video_offset_seconds(), None);
        r.fps = Some(f64::NAN);
        assert_eq!(r.video_offset_seconds(), None);
        r.fps = None;
        assert_eq!(r.video_offset_seconds(), None);
    }

    #[test]
    fn remote_and_missing_locations() {
        let mut r = row();
        assert_eq!(r.location(), FrameMediaLocation::Unavailable);
        r.video_chunk_id = Some(3); // evicted chunk: its path was emptied
        assert_eq!(r.location(), FrameMediaLocation::Unavailable);
        r.video_path = Some("cloud://chunk/3".into());
        assert_eq!(r.location(), FrameMediaLocation::Remote);
        r.snapshot_path = Some("cloud://frame/7".into());
        assert_eq!(r.location(), FrameMediaLocation::Remote);
    }

    #[test]
    fn buckets_cover_the_span_without_gaps() {
        let b = bucket_bounds(ts(0), ts(600), 6);
        assert_eq!(b.len(), 6);
        assert_eq!(b[0].0, ts(0));
        assert_eq!(b[1].0, ts(100));
        assert_eq!(b[5].1, ts(600) + Duration::microseconds(1));
        for pair in b.windows(2) {
            assert_eq!(pair[0].1, pair[1].0);
        }
    }

    #[test]
    fn single_instant_gives_one_bucket() {
        let b = bucket_bounds(ts(5), ts(5), 6);
        assert_eq!(b, vec![(ts(5), ts(5) + Duration::microseconds(1))]);
        assert!(bucket_bounds(ts(0), ts(10), 0).is_empty());
        // A reversed pair is treated as a single instant.
        assert_eq!(bucket_bounds(ts(5), ts(1), 3).len(), 1);
    }

    #[test]
    fn tiny_span_drops_empty_buckets() {
        let first = ts(0);
        let last = first + Duration::microseconds(2);
        let b = bucket_bounds(first, last, 6);
        assert!(!b.is_empty() && b.len() <= 3, "{b:?}");
        assert!(b.iter().all(|(lo, hi)| lo < hi));
        assert_eq!(b.last().unwrap().1, last + Duration::microseconds(1));
    }

    #[test]
    fn url_host_extraction() {
        assert_eq!(
            browser_url_host("https://Docs.GitHub.com:443/a?b#c").as_deref(),
            Some("docs.github.com")
        );
        assert_eq!(
            browser_url_host("github.com/x").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            browser_url_host("http://user:pw@example.org./").as_deref(),
            Some("example.org")
        );
        assert_eq!(
            browser_url_host("http://[::1]:8080/").as_deref(),
            Some("::1")
        );
        assert_eq!(browser_url_host("https:///nohost"), None);
        assert_eq!(browser_url_host("   "), None);
    }

    #[test]
    fn domain_matching_respects_label_boundaries() {
        assert!(host_matches_domain("github.com", "github.com"));
        assert!(host_matches_domain("docs.github.com", "github.com"));
        assert!(!host_matches_domain("notgithub.com", "github.com"));
        assert!(!host_matches_domain("github.com.evil.io", "github.com"));
        assert!(!host_matches_domain("com", "github.com"));
        assert!(!host_matches_domain("github.com", ""));
    }

    #[test]
    fn domain_validation() {
        assert_eq!(
            normalize_browser_domain(" GitHub.com. ").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            normalize_browser_domain("localhost").as_deref(),
            Some("localhost")
        );
        assert_eq!(
            normalize_browser_domain("my_host.local").as_deref(),
            Some("my_host.local")
        );
        for bad in [
            "",
            "https://github.com",
            "github.com/x",
            "github.com:443",
            "-bad.com",
            "bad-.com",
            "a..b",
            "*.github.com",
            "git hub.com",
            "100%.com",
        ] {
            assert_eq!(
                normalize_browser_domain(bad),
                None,
                "{bad:?} must be rejected"
            );
        }
        let long_label = format!("{}.com", "a".repeat(64));
        assert_eq!(normalize_browser_domain(&long_label), None);
        let long_host = vec!["abcdefghi"; 26].join(".");
        assert!(long_host.len() > 253);
        assert_eq!(normalize_browser_domain(&long_host), None);
    }

    #[test]
    fn like_pattern_escapes_wildcards() {
        assert_eq!(like_contains_pattern("a_b%c\\"), "%a\\_b\\%c\\\\%");
        assert_eq!(like_contains_pattern("github.com"), "%github.com%");
    }
}
