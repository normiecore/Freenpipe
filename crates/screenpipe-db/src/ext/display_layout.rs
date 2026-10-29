// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Display layout snapshots (`display_layout`).
//!
//! Click coordinates in `ui_events` are global desktop points, while element
//! and OCR bounds are normalised per monitor. Joining the two on a
//! multi-monitor machine needs to know where each display sat when the data
//! was captured, so the monitor watcher records the arrangement: one row when
//! capture starts and one more every time the arrangement changes.
//!
//! `layout_json` is a canonical, compact JSON array of displays sorted by id,
//! each with the keys `id, stable_id, name, x, y, width, height, is_primary`
//! in that order. Canonical form makes "did the layout change?" a plain
//! string comparison against the newest row.

use serde::{Deserialize, Deserializer, Serialize};

use crate::DatabaseManager;

/// Why a `display_layout` row was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayLayoutReason {
    /// First arrangement observed after capture started.
    Startup,
    /// The arrangement differs from the previous row.
    Change,
}

impl DisplayLayoutReason {
    /// Value stored in `display_layout.reason`.
    pub fn as_str(self) -> &'static str {
        match self {
            DisplayLayoutReason::Startup => "startup",
            DisplayLayoutReason::Change => "change",
        }
    }
}

/// One display in a layout snapshot.
///
/// Field order is the serialized key order and part of the canonical form,
/// so do not reorder the fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayLayoutEntry {
    /// Numeric monitor id as the capture backend reports it.
    pub id: u32,
    /// `{name}_{width}x{height}_{x},{y}`, the id that survives reboots.
    pub stable_id: String,
    pub name: String,
    /// Origin of the display in the global desktop coordinate space.
    #[serde(deserialize_with = "number_as_i32")]
    pub x: i32,
    #[serde(deserialize_with = "number_as_i32")]
    pub y: i32,
    /// Size as the capture backend reports it.
    #[serde(deserialize_with = "number_as_u32")]
    pub width: u32,
    #[serde(deserialize_with = "number_as_u32")]
    pub height: u32,
    pub is_primary: bool,
}

/// Accept integers and floats (rows written by other builds may carry
/// fractional points) and round to the nearest integer.
fn number_as_f64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    let value = serde_json::Number::deserialize(deserializer)?;
    value
        .as_f64()
        .ok_or_else(|| serde::de::Error::custom("display layout number out of range"))
}

fn number_as_i32<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i32, D::Error> {
    let v = number_as_f64(deserializer)?.round();
    if v < i32::MIN as f64 || v > i32::MAX as f64 {
        return Err(serde::de::Error::custom("display coordinate out of range"));
    }
    Ok(v as i32)
}

fn number_as_u32<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let v = number_as_f64(deserializer)?.round();
    if v < 0.0 || v > u32::MAX as f64 {
        return Err(serde::de::Error::custom("display size out of range"));
    }
    Ok(v as u32)
}

/// Serialize `displays` in canonical form: sorted by id (ties broken by
/// `stable_id`, so the output never depends on enumeration order), compact,
/// fixed key order.
pub fn canonical_display_layout_json(displays: &[DisplayLayoutEntry]) -> String {
    let mut sorted: Vec<&DisplayLayoutEntry> = displays.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id).then_with(|| a.stable_id.cmp(&b.stable_id)));
    // Serializing plain structs and strings cannot fail.
    serde_json::to_string(&sorted).unwrap_or_else(|_| "[]".to_string())
}

/// Parse a stored `layout_json` value.
pub fn parse_display_layout_json(
    layout_json: &str,
) -> Result<Vec<DisplayLayoutEntry>, serde_json::Error> {
    serde_json::from_str(layout_json)
}

impl DatabaseManager {
    /// Append a layout snapshot and return its row id. `layout_json` should be
    /// the output of [`canonical_display_layout_json`].
    pub async fn insert_display_layout(
        &self,
        reason: DisplayLayoutReason,
        layout_json: &str,
    ) -> Result<i64, sqlx::Error> {
        let mut tx = self.begin_immediate_with_retry().await?;
        let inserted =
            sqlx::query("INSERT INTO display_layout (reason, layout_json) VALUES (?1, ?2)")
                .bind(reason.as_str())
                .bind(layout_json)
                .execute(&mut **tx.conn())
                .await;
        match inserted {
            Ok(result) => {
                let id = result.last_insert_rowid();
                tx.commit().await?;
                Ok(id)
            }
            Err(e) => {
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }

    /// `layout_json` of the newest snapshot, if any row exists.
    pub async fn latest_display_layout(&self) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar::<_, String>(
            "SELECT layout_json FROM display_layout ORDER BY id DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(id: u32, name: &str, x: i32, primary: bool) -> DisplayLayoutEntry {
        DisplayLayoutEntry {
            id,
            stable_id: format!("{name}_1920x1080_{x},0"),
            name: name.to_string(),
            x,
            y: 0,
            width: 1920,
            height: 1080,
            is_primary: primary,
        }
    }

    #[test]
    fn canonical_json_is_sorted_compact_and_keyed_in_order() {
        let right = display(7, "Right", 1920, false);
        let left = display(3, "Left", 0, true);
        let json = canonical_display_layout_json(&[right.clone(), left.clone()]);
        assert_eq!(
            json,
            "[{\"id\":3,\"stable_id\":\"Left_1920x1080_0,0\",\"name\":\"Left\",\"x\":0,\"y\":0,\
             \"width\":1920,\"height\":1080,\"is_primary\":true},\
             {\"id\":7,\"stable_id\":\"Right_1920x1080_1920,0\",\"name\":\"Right\",\"x\":1920,\
             \"y\":0,\"width\":1920,\"height\":1080,\"is_primary\":false}]"
        );
        // Enumeration order does not change the canonical form.
        assert_eq!(json, canonical_display_layout_json(&[left, right]));
    }

    #[test]
    fn canonical_json_of_no_displays_is_an_empty_array() {
        assert_eq!(canonical_display_layout_json(&[]), "[]");
    }

    #[test]
    fn parse_round_trips_and_accepts_fractional_numbers() {
        let displays = vec![display(1, "A", -1280, false), display(2, "B", 0, true)];
        let json = canonical_display_layout_json(&displays);
        assert_eq!(parse_display_layout_json(&json).unwrap(), displays);

        let foreign = r#"[{"id":1,"stable_id":"Built-in","name":"Built-in","x":0.0,"y":-0.4,
            "width":1512.0,"height":982,"is_primary":true}]"#;
        let parsed = parse_display_layout_json(foreign).unwrap();
        assert_eq!(parsed[0].x, 0);
        assert_eq!(parsed[0].y, 0);
        assert_eq!(parsed[0].width, 1512);
        assert_eq!(parsed[0].height, 982);
    }

    #[test]
    fn parse_rejects_negative_sizes() {
        let bad = r#"[{"id":1,"stable_id":"s","name":"n","x":0,"y":0,
            "width":-5,"height":10,"is_primary":true}]"#;
        assert!(parse_display_layout_json(bad).is_err());
    }

    #[tokio::test]
    async fn insert_and_read_latest() {
        let db = DatabaseManager::new("sqlite::memory:", Default::default())
            .await
            .unwrap();
        assert_eq!(db.latest_display_layout().await.unwrap(), None);

        let first = canonical_display_layout_json(&[display(1, "A", 0, true)]);
        let second = canonical_display_layout_json(&[
            display(1, "A", 0, true),
            display(2, "B", 1920, false),
        ]);
        let id1 = db
            .insert_display_layout(DisplayLayoutReason::Startup, &first)
            .await
            .unwrap();
        let id2 = db
            .insert_display_layout(DisplayLayoutReason::Change, &second)
            .await
            .unwrap();
        assert!(id2 > id1);
        assert_eq!(db.latest_display_layout().await.unwrap(), Some(second));

        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT reason, timestamp FROM display_layout ORDER BY id")
                .fetch_all(&db.pool)
                .await
                .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "startup");
        assert_eq!(rows[1].0, "change");
        // The column default writes RFC 3339 with a `Z` suffix.
        assert!(rows[0].1.ends_with('Z'), "timestamp {}", rows[0].1);
    }
}
