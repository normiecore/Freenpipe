// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! `format=` / `fields=` on `/search`, `/elements` and
//! `/frames/:id/elements`, calendar time literals on `/search`, and
//! `GET /tags/autocomplete`, through the real engine router with API auth on.
//!
//! Fixtures are synthetic rows in an in-memory database; nothing binds a
//! port or touches a real data dir.

use std::{net::SocketAddr, sync::Arc};

use axum::{
    body::{to_bytes, Body, Bytes},
    http::{HeaderMap, Request, StatusCode},
    Router,
};
use chrono::{Local, Utc};
use screenpipe_audio::audio_manager::AudioManagerBuilder;
use screenpipe_db::DatabaseManager;
use screenpipe_engine::SCServer;
use serde_json::{json, Value};
use tower::ServiceExt;

const KEY: &str = "fp-response-format-test-key";

/// OCR text with a comma, quotes, a tab and a line break, to exercise
/// CSV quoting and TSV replacement through the route.
const OCR_TEXT: &str = "Budget, \"Q3\" review\tsentinelword\nsecond line";

struct Fixture {
    router: Router,
    db: Arc<DatabaseManager>,
    _dir: tempfile::TempDir,
    frame_id: i64,
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(
        DatabaseManager::new("sqlite::memory:", Default::default())
            .await
            .unwrap(),
    );
    let audio_manager = Arc::new(
        AudioManagerBuilder::new()
            .output_path(dir.path().join("audio"))
            .is_disabled(true)
            .build(db.clone())
            .await
            .unwrap(),
    );
    let mut server = SCServer::new(
        db.clone(),
        SocketAddr::from(([127, 0, 0, 1], 3035)),
        dir.path().to_path_buf(),
        false,
        false,
        audio_manager,
        false,
        "balanced".to_string(),
    );
    server.api_auth = true;
    server.api_auth_key = Some(KEY.to_string());
    let router = server.create_router().await;

    let device = "monitor_1";
    let chunk = dir.path().join("chunk.mp4");
    db.insert_video_chunk(chunk.to_str().unwrap(), device)
        .await
        .unwrap();
    let frame_id = db
        .insert_frame(
            device,
            Some(Utc::now()),
            Some("https://example.test/doc"),
            Some("FormatFixtureApp"),
            Some("Quarterly, \"draft\""),
            true,
            Some(0),
        )
        .await
        .unwrap();
    assert_ne!(frame_id, 0);
    db.insert_ocr_text(frame_id, OCR_TEXT, "[]").await.unwrap();

    for (order, role, text, left) in [
        (0, "AXWindow", None, 0.0),
        (1, "AXButton", Some("Save, \"now\""), 0.25),
        (2, "AXStaticText", Some("tab\there\nnext"), 0.5),
    ] {
        sqlx::query(
            "INSERT INTO elements (frame_id, source, role, text, parent_id, depth, \
             left_bound, top_bound, width_bound, height_bound, confidence, sort_order) \
             VALUES (?1, 'accessibility', ?2, ?3, NULL, 0, ?4, 0.1, 0.2, 0.05, NULL, ?5)",
        )
        .bind(frame_id)
        .bind(role)
        .bind(text)
        .bind(left)
        .bind(order)
        .execute(db.write_pool())
        .await
        .unwrap();
    }

    Fixture {
        router,
        db,
        _dir: dir,
        frame_id,
    }
}

impl Fixture {
    async fn request(
        &self,
        method: &str,
        uri: &str,
        auth: bool,
        body: Option<Value>,
    ) -> (StatusCode, HeaderMap, Bytes) {
        let mut request = Request::builder().method(method).uri(uri);
        if auth {
            request = request.header("authorization", format!("Bearer {KEY}"));
        }
        let body = match body {
            Some(json) => {
                request = request.header("content-type", "application/json");
                Body::from(json.to_string())
            }
            None => Body::empty(),
        };
        let response = self
            .router
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, headers, bytes)
    }

    async fn api(&self, uri: &str) -> (StatusCode, HeaderMap, Bytes) {
        self.request("GET", uri, true, None).await
    }
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
}

fn json_of(body: &Bytes) -> Value {
    serde_json::from_slice(body).unwrap_or(Value::Null)
}

fn text_of(body: &Bytes) -> String {
    String::from_utf8(body.to_vec()).unwrap()
}

/// RFC 4180 quoting, written independently of the engine's writer.
fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

const SEARCH: &str = "/search?q=sentinelword&content_type=ocr&limit=3";

#[tokio::test]
async fn search_json_is_unchanged_without_format_or_fields() {
    let fx = fixture().await;
    let (status, headers, plain) = fx.api(SEARCH).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header(&headers, "content-type"), "application/json");
    let (_, _, explicit) = fx.api(&format!("{SEARCH}&format=json")).await;
    let (_, _, empty) = fx.api(&format!("{SEARCH}&format=&fields=")).await;
    assert_eq!(plain, explicit);
    assert_eq!(plain, empty);

    let body = json_of(&plain);
    let keys: Vec<&String> = body.as_object().unwrap().keys().collect();
    assert_eq!(keys, vec!["data", "pagination"]);
    assert_eq!(body["pagination"]["total"], 1);
    let row = &body["data"][0];
    assert_eq!(row["type"], "OCR");
    assert_eq!(row["content"]["frame_id"], fx.frame_id);
    assert_eq!(row["content"]["text"], OCR_TEXT);
    assert_eq!(row["content"]["app_name"], "FormatFixtureApp");
    // Still the typed shape the CLI, MCP server and app deserialize.
    let typed: screenpipe_engine::routes::search::SearchResponse =
        serde_json::from_slice(&plain).unwrap();
    assert_eq!(typed.data.len(), 1);
}

#[tokio::test]
async fn search_fields_make_flat_json_rows() {
    let fx = fixture().await;
    let (status, _, body) = fx
        .api(&format!(
            "{SEARCH}&fields=type,content.app_name,content.window_name,content.nope"
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    let body = json_of(&body);
    assert_eq!(
        body["data"],
        json!([{
            "type": "OCR",
            "content.app_name": "FormatFixtureApp",
            "content.window_name": "Quarterly, \"draft\"",
        }])
    );
    assert_eq!(body["pagination"]["total"], 1);
}

#[tokio::test]
async fn search_csv_quotes_and_reports_the_total() {
    let fx = fixture().await;
    let (status, headers, body) = fx
        .api(&format!(
            "{SEARCH}&format=csv&fields=content.text,type,content.frame_id"
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header(&headers, "content-type"), "text/csv; charset=utf-8");
    assert_eq!(header(&headers, "x-total-count"), "1");
    assert_eq!(
        text_of(&body),
        format!(
            "content.text,type,content.frame_id\r\n{},OCR,{}\r\n",
            csv_cell(OCR_TEXT),
            fx.frame_id
        )
    );

    // The acceptance example: a single column, limit 3.
    let (status, headers, body) = fx
        .api("/search?format=csv&fields=content.text&limit=3&content_type=ocr")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header(&headers, "content-type"), "text/csv; charset=utf-8");
    assert!(text_of(&body).starts_with("content.text\r\n"));
}

#[tokio::test]
async fn search_tsv_without_fields_flattens_every_column() {
    let fx = fixture().await;
    let (status, headers, body) = fx.api(&format!("{SEARCH}&format=tsv")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        header(&headers, "content-type"),
        "text/tab-separated-values; charset=utf-8"
    );
    let text = text_of(&body);
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(
        lines.len(),
        3,
        "header, one row, trailing newline: {text:?}"
    );
    assert_eq!(lines[2], "");
    let columns: Vec<&str> = lines[0].split('\t').collect();
    let cells: Vec<&str> = lines[1].split('\t').collect();
    assert_eq!(columns.len(), cells.len());
    assert_eq!(columns[0], "type");
    assert!(columns.contains(&"content.frame_id"));
    let at = |name: &str| cells[columns.iter().position(|c| *c == name).unwrap()];
    assert_eq!(at("type"), "OCR");
    assert_eq!(
        at("content.text"),
        "Budget, \"Q3\" review sentinelword second line"
    );
    assert_eq!(at("content.frame_id"), fx.frame_id.to_string());
}

#[tokio::test]
async fn unknown_formats_are_rejected_on_every_list_route() {
    let fx = fixture().await;
    let expected = json!({"error": "unknown format `xml` (expected json|csv|tsv)"});
    for uri in [
        format!("{SEARCH}&format=xml"),
        "/elements?format=xml".to_string(),
        format!("/frames/{}/elements?format=xml", fx.frame_id),
    ] {
        let (status, _, body) = fx.api(&uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(json_of(&body), expected, "{uri}");
    }
}

#[tokio::test]
async fn element_text_formats_are_answered_as_json_until_implemented() {
    let fx = fixture().await;
    let frame = format!("/frames/{}/elements", fx.frame_id);
    let list = format!("/elements?frame_id={}&source=accessibility", fx.frame_id);
    for base in [frame.as_str(), list.as_str()] {
        let join = if base.contains('?') { '&' } else { '?' };
        let (status, headers, plain) = fx.api(base).await;
        assert_eq!(status, StatusCode::OK, "{base}");
        assert_eq!(header(&headers, "content-type"), "application/json");
        // What the screenpipe-era MCP element tool sends: outline for
        // reading, automation for computer use, preferred otherwise.
        for format in ["outline", "automation", "preferred", " Outline "] {
            let uri = format!("{base}{join}format={}", format.replace(' ', "%20"));
            let (status, headers, body) = fx.api(&uri).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            assert_eq!(
                header(&headers, "content-type"),
                "application/json",
                "{uri}"
            );
            assert_eq!(body, plain, "{uri}");
        }
        // `fields` applies exactly as it does with format=json.
        let (_, _, json_fields) = fx
            .api(&format!("{base}{join}format=json&fields=id,role"))
            .await;
        let (status, _, outline_fields) = fx
            .api(&format!("{base}{join}format=outline&fields=id,role"))
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(outline_fields, json_fields);
        assert_eq!(
            json_of(&outline_fields)["data"][0]
                .as_object()
                .unwrap()
                .len(),
            2
        );
    }
    // They are element formats only: /search still rejects them.
    let (status, _, body) = fx.api(&format!("{SEARCH}&format=outline")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        json_of(&body),
        json!({"error": "unknown format `outline` (expected json|csv|tsv)"})
    );
}

#[tokio::test]
async fn tabular_timestamps_follow_the_json_timezone() {
    let fx = fixture().await;
    let cell = |body: &Bytes| -> String {
        let text = text_of(body);
        let mut lines = text.split("\r\n");
        assert_eq!(lines.next(), Some("content.timestamp"), "{text:?}");
        lines.next().unwrap().to_string()
    };
    let json_timestamp = |body: &Bytes| -> String {
        json_of(body)["data"][0]["content"]["timestamp"]
            .as_str()
            .unwrap()
            .to_string()
    };

    // Default: the middleware gives JSON local time with an offset, and the
    // CSV cell carries the same text.
    let (_, _, json_local) = fx.api(SEARCH).await;
    let (status, _, csv_local) = fx
        .api(&format!("{SEARCH}&format=csv&fields=content.timestamp"))
        .await;
    assert_eq!(status, StatusCode::OK);
    let local = json_timestamp(&json_local);
    assert!(!local.ends_with('Z'), "{local}");
    assert_eq!(cell(&csv_local), local);

    // timezone=utc keeps UTC in both.
    let (_, _, json_utc) = fx.api(&format!("{SEARCH}&timezone=utc")).await;
    let (_, _, tsv_utc) = fx
        .api(&format!(
            "{SEARCH}&timezone=utc&format=tsv&fields=content.timestamp"
        ))
        .await;
    let utc = json_timestamp(&json_utc);
    assert!(utc.ends_with('Z'), "{utc}");
    assert_eq!(text_of(&tsv_utc), format!("content.timestamp\n{utc}\n"));
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(&local).unwrap(),
        chrono::DateTime::parse_from_rfc3339(&utc).unwrap(),
        "same instant"
    );
}

#[tokio::test]
async fn calendar_literals_bound_search_at_local_midnight() {
    let fx = fixture().await;
    let local_today = Local::now().date_naive().format("%Y-%m-%d").to_string();
    for start in ["today", "TODAY", "yesterday", local_today.as_str()] {
        let (status, _, body) = fx
            .api(&format!(
                "/search?start_time={start}&content_type=ocr&limit=1"
            ))
            .await;
        assert_eq!(status, StatusCode::OK, "start_time={start}");
        assert_eq!(
            json_of(&body)["pagination"]["total"],
            1,
            "start_time={start}"
        );
    }
    // `end_time=today` is the start of today, so a frame from just now is out.
    let (status, _, body) = fx
        .api("/search?start_time=yesterday&end_time=today&content_type=ocr&limit=1")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_of(&body)["pagination"]["total"], 0);
    // RFC 3339 and relative input keep working.
    for start in ["2000-01-01T00:00:00Z", "1h%20ago"] {
        let (status, _, body) = fx
            .api(&format!(
                "/search?start_time={start}&content_type=ocr&limit=1"
            ))
            .await;
        assert_eq!(status, StatusCode::OK, "start_time={start}");
        assert_eq!(
            json_of(&body)["pagination"]["total"],
            1,
            "start_time={start}"
        );
    }
}

#[tokio::test]
async fn bad_time_input_names_the_three_accepted_families() {
    let fx = fixture().await;
    let (status, _, body) = fx.api("/search?start_time=nonsense&limit=1").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let message = text_of(&body);
    for family in ["RFC 3339", "today", "yesterday", "YYYY-MM-DD", "ago"] {
        assert!(message.contains(family), "{message:?} lacks {family}");
    }
    let (status, _, _) = fx.api("/search?start_time=2026-02-30&limit=1").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn elements_formats_and_fields() {
    let fx = fixture().await;
    let base = format!("/elements?frame_id={}&source=accessibility", fx.frame_id);

    let (status, headers, plain) = fx.api(&base).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header(&headers, "content-type"), "application/json");
    let plain = json_of(&plain);
    assert_eq!(plain["pagination"]["total"], 3);
    assert!(plain["data"][0].get("bounds").is_some());

    let (status, headers, body) = fx
        .api(&format!("{base}&format=csv&fields=role,text,bounds.left"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header(&headers, "content-type"), "text/csv; charset=utf-8");
    assert_eq!(header(&headers, "x-total-count"), "3");
    let text = text_of(&body);
    assert!(text.starts_with("role,text,bounds.left\r\n"), "{text:?}");
    for line in [
        "AXWindow,,0.0\r\n",
        "AXButton,\"Save, \"\"now\"\"\",0.25\r\n",
        "AXStaticText,\"tab\there\nnext\",0.5\r\n",
    ] {
        assert!(text.contains(line), "{text:?} lacks {line:?}");
    }

    let (_, _, body) = fx.api(&format!("{base}&fields=id,bounds.left")).await;
    let rows = json_of(&body)["data"].clone();
    assert_eq!(rows.as_array().unwrap().len(), 3);
    for row in rows.as_array().unwrap() {
        let keys: Vec<&String> = row.as_object().unwrap().keys().collect();
        assert_eq!(keys.len(), 2);
        assert!(row.get("bounds.left").is_some() && row.get("id").is_some());
    }
}

#[tokio::test]
async fn frame_elements_need_the_key_and_honour_the_format() {
    let fx = fixture().await;
    let uri = format!(
        "/frames/{}/elements?format=tsv&fields=sort_order,role,text",
        fx.frame_id
    );
    let (status, _, _) = fx.request("GET", &uri, false, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, headers, body) = fx.api(&uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        header(&headers, "content-type"),
        "text/tab-separated-values; charset=utf-8"
    );
    assert_eq!(
        text_of(&body),
        "sort_order\trole\ttext\n0\tAXWindow\t\n1\tAXButton\tSave, \"now\"\n2\tAXStaticText\ttab here next\n"
    );

    // Without format/fields the JSON envelope is the base one.
    let (status, _, body) = fx.api(&format!("/frames/{}/elements", fx.frame_id)).await;
    assert_eq!(status, StatusCode::OK);
    let body = json_of(&body);
    assert_eq!(
        body["pagination"],
        json!({"limit": 3, "offset": 0, "total": 3})
    );
}

#[tokio::test]
async fn tag_autocomplete_lists_counts_from_every_store() {
    let fx = fixture().await;
    let (status, _, _) = fx
        .request(
            "POST",
            &format!("/tags/vision/{}", fx.frame_id),
            true,
            Some(json!({"tags": ["work", "project:atlas", "homework"]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    fx.db
        .insert_memory(
            "fact",
            "user",
            None,
            Some(r#"["work","Workshop"]"#),
            0.5,
            None,
        )
        .await
        .unwrap();

    let (status, headers, body) = fx.api("/tags/autocomplete?q=wo").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header(&headers, "content-type"), "application/json");
    assert_eq!(
        json_of(&body),
        json!([
            {"name": "work", "frame_count": 1, "audio_count": 0, "memory_count": 1},
            {"name": "Workshop", "frame_count": 0, "audio_count": 0, "memory_count": 1},
            {"name": "homework", "frame_count": 1, "audio_count": 0, "memory_count": 0},
        ])
    );

    let (_, _, body) = fx.api("/tags/autocomplete?q=%23atlas&limit=5").await;
    assert_eq!(
        json_of(&body),
        json!([{"name": "project:atlas", "frame_count": 1, "audio_count": 0, "memory_count": 0}])
    );
    let (_, _, body) = fx.api("/tags/autocomplete?limit=1").await;
    assert_eq!(json_of(&body).as_array().unwrap().len(), 1);
    let (status, _, _) = fx.api("/tags/autocomplete?limit=lots").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = fx
        .request("GET", "/tags/autocomplete?q=wo", false, None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn openapi_describes_the_new_parameters_and_route() {
    let fx = fixture().await;
    let (status, _, body) = fx.api("/openapi.json").await;
    assert_eq!(status, StatusCode::OK);
    let spec = json_of(&body);
    let params = |path: &str| -> Vec<String> {
        spec["paths"][path]["get"]["parameters"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|p| p["name"].as_str().map(str::to_string))
            .collect()
    };
    // The frame route's key depends on how oasgen spells path parameters.
    let frame_path = ["/frames/{frame_id}/elements", "/frames/:frame_id/elements"]
        .into_iter()
        .find(|p| spec["paths"][*p]["get"].is_object())
        .expect("GET /frames/:frame_id/elements is documented");
    for path in ["/search", "/elements", frame_path] {
        let names = params(path);
        assert!(names.contains(&"format".to_string()), "{path}: {names:?}");
        assert!(names.contains(&"fields".to_string()), "{path}: {names:?}");
    }
    let autocomplete = &spec["paths"]["/tags/autocomplete"]["get"];
    assert!(
        autocomplete.is_object(),
        "GET /tags/autocomplete is documented"
    );
    let names = params("/tags/autocomplete");
    assert!(names.contains(&"q".to_string()) && names.contains(&"limit".to_string()));
}
