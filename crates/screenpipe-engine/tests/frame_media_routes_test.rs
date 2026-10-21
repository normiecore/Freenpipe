// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Frame media routes end to end, through the real engine router with API
//! auth on: `/frames/:id` typed errors, `/frames/:id/thumbnail`,
//! `/frames/preview-samples` and `/frames/preview-media/:video_chunk_id`.
//!
//! Every fixture is synthetic (generated JPEGs and MP4s in a temp dir, an
//! in-memory database); nothing touches a real data dir or binds a port.

use std::{net::SocketAddr, path::Path, sync::Arc};

use axum::{
    body::{to_bytes, Body, Bytes},
    http::{HeaderMap, Request, StatusCode},
    Router,
};
use chrono::{DateTime, Duration, SecondsFormat, TimeZone, Utc};
use image::{codecs::jpeg::JpegEncoder, GenericImageView, ImageBuffer, Rgb};
use screenpipe_audio::audio_manager::AudioManagerBuilder;
use screenpipe_db::DatabaseManager;
use screenpipe_engine::SCServer;
use serde_json::Value;
use tower::ServiceExt;

const KEY: &str = "fp-frame-media-test-key";

struct Fixture {
    router: Router,
    db: Arc<DatabaseManager>,
    dir: tempfile::TempDir,
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
    Fixture { router, db, dir }
}

fn at(secs: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap() + Duration::seconds(secs)
}

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn write_jpeg(path: &Path, width: u32, height: u32) {
    let img = ImageBuffer::from_fn(width, height, |x, y| {
        Rgb([(x % 251) as u8, (y % 241) as u8, ((x * y) % 239) as u8])
    });
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 85)
        .encode_image(&img)
        .unwrap();
    std::fs::write(path, out).unwrap();
}

impl Fixture {
    fn path(&self, name: &str) -> String {
        self.dir.path().join(name).to_string_lossy().into_owned()
    }

    /// A snapshot frame with a real 640x360 JPEG on disk.
    async fn snapshot_frame(&self, name: &str, device: &str, secs: i64, app: &str) -> i64 {
        let path = self.path(name);
        write_jpeg(Path::new(&path), 640, 360);
        self.frame_row(&path, device, secs, app, None).await
    }

    async fn frame_row(
        &self,
        path: &str,
        device: &str,
        secs: i64,
        app: &str,
        url: Option<&str>,
    ) -> i64 {
        self.db
            .insert_snapshot_frame(
                device,
                at(secs),
                path,
                Some(app),
                Some("window"),
                url,
                true,
                Some("visual_change"),
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap()
    }

    async fn sql(&self, sql: &'static str, id: i64) {
        sqlx::query(sql)
            .bind(id)
            .execute(&self.db.pool)
            .await
            .unwrap();
    }

    async fn get(&self, uri: &str, headers: &[(&str, &str)]) -> (StatusCode, HeaderMap, Bytes) {
        let mut request = Request::builder().method("GET").uri(uri);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = self
            .router
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, headers, body)
    }

    /// GET with the bearer token.
    async fn api(&self, uri: &str, extra: &[(&str, &str)]) -> (StatusCode, HeaderMap, Bytes) {
        let bearer = format!("Bearer {KEY}");
        let mut headers = vec![("authorization", bearer.as_str())];
        headers.extend_from_slice(extra);
        self.get(uri, &headers).await
    }
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
}

fn json(body: &Bytes) -> Value {
    serde_json::from_slice(body).unwrap_or(Value::Null)
}

#[tokio::test]
async fn frames_require_the_bearer_token_while_health_stays_open() {
    let fx = fixture().await;
    let id = fx.snapshot_frame("a.jpg", "monitor_1", 0, "Code").await;
    let preview = format!(
        "/frames/preview-samples?app_name=Code&start_time={}&end_time={}",
        rfc3339(at(-60)),
        rfc3339(at(60))
    );

    for uri in [
        format!("/frames/{id}"),
        format!("/frames/{id}/thumbnail?width=320"),
        format!("/frames/{id}/context"),
        preview.clone(),
        "/frames/preview-media/1".to_string(),
    ] {
        let (status, _, body) = fx.get(&uri, &[]).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri} must need auth");
        assert!(json(&body)["error"]
            .as_str()
            .unwrap_or_default()
            .starts_with("unauthorized"));
    }
    let (status, _, _) = fx.get("/health", &[]).await;
    assert_eq!(status, StatusCode::OK, "/health stays open");

    let (status, headers, body) = fx.api(&format!("/frames/{id}"), &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header(&headers, "content-type"), "image/jpeg");
    assert_eq!(
        image::load_from_memory(&body).unwrap().dimensions(),
        (640, 360)
    );

    // <img src> paths: the cookie and the ?token= query both work.
    let cookie = format!("screenpipe_auth={KEY}");
    let (status, _, _) = fx
        .get(
            &format!("/frames/{id}/thumbnail"),
            &[("cookie", cookie.as_str())],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = fx
        .get(
            &format!("/frames/{id}/thumbnail?width=320&token={KEY}"),
            &[],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = fx.api(&preview, &[]).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn thumbnail_carries_size_headers_and_revalidates_with_etag() {
    let fx = fixture().await;
    let id = fx.snapshot_frame("a.jpg", "monitor_1", 0, "Code").await;
    let uri = format!("/frames/{id}/thumbnail?width=320");

    let (status, headers, body) = fx.api(&uri, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header(&headers, "content-type"), "image/jpeg");
    assert_eq!(header(&headers, "cache-control"), "private, no-cache");
    assert_eq!(header(&headers, "x-screenpipe-thumbnail-width"), "320");
    assert_eq!(header(&headers, "x-screenpipe-thumbnail-height"), "180");
    assert_eq!(header(&headers, "x-screenpipe-thumbnail-cache"), "miss");
    assert_eq!(header(&headers, "content-length"), body.len().to_string());
    let etag = header(&headers, "etag").to_string();
    assert!(
        etag.starts_with(&format!("\"frame-thumbnail-{id}-320-75-")) && etag.ends_with('"'),
        "{etag}"
    );
    assert_eq!(
        image::load_from_memory(&body).unwrap().dimensions(),
        (320, 180)
    );

    let (status, headers, _) = fx.api(&uri, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header(&headers, "x-screenpipe-thumbnail-cache"), "hit");
    assert_eq!(header(&headers, "etag"), etag);

    let (status, headers, body) = fx.api(&uri, &[("if-none-match", etag.as_str())]).await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert_eq!(header(&headers, "etag"), etag);
    assert!(body.is_empty());

    // Never wider than the frame; quality is part of the tag.
    let (status, headers, _) = fx
        .api(
            &format!("/frames/{id}/thumbnail?width=1000&quality=60&retry=2"),
            &[],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header(&headers, "x-screenpipe-thumbnail-width"), "640");
    assert!(header(&headers, "etag").contains("-1000-60-"));

    for bad in [
        "width=10",
        "width=4000",
        "quality=0",
        "quality=101",
        "fallback=maybe",
    ] {
        let (status, _, body) = fx.api(&format!("/frames/{id}/thumbnail?{bad}"), &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        if !bad.starts_with("fallback") {
            assert_eq!(json(&body)["error_type"], "invalid_request", "{bad}");
        }
    }

    let (status, _, body) = fx.api("/frames/987654/thumbnail", &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json(&body)["error_type"], "frame_not_found");
}

#[tokio::test]
async fn a_deleted_frame_is_never_served_from_the_cache() {
    let fx = fixture().await;
    let id = fx.snapshot_frame("a.jpg", "monitor_1", 0, "Code").await;
    let uri = format!("/frames/{id}/thumbnail?width=320");
    assert_eq!(fx.api(&uri, &[]).await.0, StatusCode::OK);
    let (_, headers, _) = fx.api(&uri, &[]).await;
    assert_eq!(header(&headers, "x-screenpipe-thumbnail-cache"), "hit");

    fx.sql("DELETE FROM frames WHERE id = ?1", id).await;

    let (status, _, body) = fx.api(&uri, &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json(&body)["error_type"], "frame_not_found");
    assert_eq!(
        fx.api(&format!("/frames/{id}"), &[]).await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn missing_snapshot_uses_a_nearby_frame_unless_fallback_is_off() {
    let fx = fixture().await;
    let near = fx.snapshot_frame("near.jpg", "monitor_1", 0, "Code").await;
    let _other_monitor = fx.snapshot_frame("other.jpg", "monitor_2", 1, "Code").await;
    let gone = fx.snapshot_frame("gone.jpg", "monitor_1", 2, "Code").await;
    std::fs::remove_file(fx.path("gone.jpg")).unwrap();

    let (status, _, body) = fx
        .api(&format!("/frames/{gone}/thumbnail?fallback=false"), &[])
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json(&body)["error_type"], "snapshot_missing");

    let (status, headers, _) = fx.api(&format!("/frames/{gone}/thumbnail"), &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        header(&headers, "etag").starts_with(&format!("\"frame-thumbnail-{near}-")),
        "served from the same monitor's neighbour: {}",
        header(&headers, "etag")
    );

    // With no neighbour left either, the failure is reported.
    std::fs::remove_file(fx.path("near.jpg")).unwrap();
    std::fs::remove_file(fx.path("other.jpg")).unwrap();
    let (status, _, body) = fx.api(&format!("/frames/{gone}/thumbnail"), &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json(&body)["error_type"], "snapshot_missing");
    let (status, _, body) = fx.api(&format!("/frames/{gone}"), &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json(&body)["error_type"], "snapshot_missing");
}

#[tokio::test]
async fn frame_route_reports_typed_media_errors() {
    let fx = fixture().await;

    // No snapshot and no video chunk: nothing to serve.
    let bare = fx.snapshot_frame("bare.jpg", "monitor_1", 0, "Code").await;
    fx.sql("UPDATE frames SET snapshot_path = NULL WHERE id = ?1", bare)
        .await;
    let (status, _, body) = fx.api(&format!("/frames/{bare}"), &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json(&body)["error_type"], "frame_media_unavailable");
    let (status, _, body) = fx
        .api(&format!("/frames/{bare}/thumbnail?fallback=false"), &[])
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json(&body)["error_type"], "frame_unavailable");

    // The snapshot path exists but cannot be read as a file.
    let dir_path = fx.path("not-a-file.jpg");
    std::fs::create_dir(&dir_path).unwrap();
    let unreadable = fx
        .frame_row(&dir_path, "monitor_9", 100, "Code", None)
        .await;
    let (status, _, body) = fx.api(&format!("/frames/{unreadable}"), &[]).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(json(&body)["error_type"], "snapshot_read_failed");

    // Synced from another device.
    let remote = fx
        .frame_row("cloud://frames/1", "monitor_1", 200, "Code", None)
        .await;
    let (status, _, body) = fx.api(&format!("/frames/{remote}"), &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json(&body)["error_type"], "remote_device");
    let (status, _, body) = fx.api(&format!("/frames/{remote}/thumbnail"), &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json(&body)["error_type"], "remote_device");

    let (status, _, _) = fx.api("/frames/424242", &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn preview_samples_list_spread_references_and_validate_queries() {
    let fx = fixture().await;
    let mut code = Vec::new();
    for i in 0..12 {
        code.push(
            fx.snapshot_frame(&format!("c{i}.jpg"), "monitor_1", i * 60, "Code")
                .await,
        );
    }
    fx.snapshot_frame("slack.jpg", "monitor_1", 30, "Slack")
        .await;
    // The first frame's file is gone: its slice moves on to the next frame.
    std::fs::remove_file(fx.path("c0.jpg")).unwrap();
    // Frame 5 was compacted into an MP4 (the file only has to exist here).
    let mp4 = fx.path("chunk.mp4");
    std::fs::write(&mp4, vec![0u8; 4096]).unwrap();
    let chunk = fx
        .db
        .insert_video_chunk_with_fps(&mp4, "monitor_1", 0.5)
        .await
        .unwrap();
    fx.db
        .compact_snapshots_queued(chunk, vec![(code[6], 3)])
        .await
        .unwrap();

    let range = format!(
        "start_time={}&end_time={}",
        rfc3339(at(-600)),
        rfc3339(at(3600))
    );
    let (status, _, body) = fx
        .api(
            &format!("/frames/preview-samples?app_name=code&{range}&limit=6"),
            &[],
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let frames = json(&body)["frames"].as_array().cloned().unwrap();
    assert_eq!(frames.len(), 6, "{frames:?}");
    let ids: Vec<i64> = frames
        .iter()
        .map(|f| f["frame_id"].as_i64().unwrap())
        .collect();
    assert!(
        !ids.contains(&code[0]),
        "a frame without its file is skipped: samples {ids:?}, frames {code:?}, body {frames:?}"
    );
    assert_eq!(ids[0], code[1]);
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "samples come in time order");
    for f in &frames {
        assert!(f["timestamp"]
            .as_str()
            .unwrap()
            .parse::<DateTime<Utc>>()
            .is_ok());
        match f["source"].as_str().unwrap() {
            "snapshot" => {
                assert!(f.get("video_chunk_id").is_none());
                assert!(f.get("video_offset_seconds").is_none());
            }
            "video" => {
                assert_eq!(f["frame_id"].as_i64(), Some(code[6]));
                assert_eq!(f["video_chunk_id"].as_i64(), Some(chunk));
                assert_eq!(f["video_offset_seconds"].as_f64(), Some(6.0));
            }
            other => panic!("unexpected source {other}"),
        }
    }
    assert!(
        frames.iter().any(|f| f["source"] == "video"),
        "the compacted frame is offered as a video sample: {frames:?}"
    );

    let start = at(0);
    for (query, why) in [
        (
            format!(
                "app_name=Code&start_time={}&end_time={}",
                rfc3339(start),
                rfc3339(start + Duration::hours(25))
            ),
            "25 h range",
        ),
        (
            format!(
                "app_name=Code&start_time={}&end_time={}",
                rfc3339(start),
                rfc3339(start)
            ),
            "empty range",
        ),
        (
            format!("app_name=Code&end_time={}", rfc3339(start)),
            "no start",
        ),
        (format!("{range}&app_name="), "empty app"),
        (format!("{range}&app_name={}", "a".repeat(257)), "long app"),
        (format!("{range}&app_name=Code&limit=0"), "limit 0"),
        (format!("{range}&app_name=Code&limit=13"), "limit 13"),
        (
            format!("{range}&app_name=Code&browser_domain=https%3A%2F%2Fgithub.com"),
            "domain with scheme",
        ),
    ] {
        let (status, _, body) = fx
            .api(&format!("/frames/preview-samples?{query}"), &[])
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{why}");
        assert_eq!(json(&body)["error_type"], "invalid_request", "{why}");
    }

    let (status, _, body) = fx
        .api(
            &format!("/frames/preview-samples?app_name=Code&{range}&browser_domain=github.com"),
            &[],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json(&body)["frames"].as_array().map(Vec::len), Some(0));
}

#[tokio::test]
async fn preview_media_serves_existing_mp4_byte_ranges() {
    let fx = fixture().await;
    let mp4 = fx.path("chunk.mp4");
    let content: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&mp4, &content).unwrap();
    let chunk = fx
        .db
        .insert_video_chunk_with_fps(&mp4, "monitor_1", 1.0)
        .await
        .unwrap();
    let uri = format!("/frames/preview-media/{chunk}");

    let (status, headers, body) = fx.api(&uri, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header(&headers, "content-type"), "video/mp4");
    assert_eq!(header(&headers, "accept-ranges"), "bytes");
    assert_eq!(header(&headers, "content-length"), "1000");
    assert_eq!(header(&headers, "cache-control"), "private");
    assert_eq!(body.as_ref(), content.as_slice());

    for (range, start, end) in [
        ("bytes=0-99", 0usize, 99usize),
        ("bytes=990-", 990, 999),
        ("bytes=-5", 995, 999),
        ("bytes=900-5000", 900, 999),
        ("bytes=-5000", 0, 999),
    ] {
        let (status, headers, body) = fx.api(&uri, &[("range", range)]).await;
        assert_eq!(status, StatusCode::PARTIAL_CONTENT, "{range}");
        assert_eq!(
            header(&headers, "content-range"),
            format!("bytes {start}-{end}/1000"),
            "{range}"
        );
        assert_eq!(
            header(&headers, "content-length"),
            (end - start + 1).to_string()
        );
        assert_eq!(body.as_ref(), &content[start..=end], "{range}");
    }

    for range in [
        "bytes=1000-",
        "bytes=abc",
        "bytes=5-1",
        "bytes=0-1,5-6",
        "bytes=-0",
        "0-99",
        "bytes=99999999999999999999999-",
    ] {
        let (status, headers, _) = fx.api(&uri, &[("range", range)]).await;
        assert_eq!(status, StatusCode::RANGE_NOT_SATISFIABLE, "{range}");
        assert_eq!(header(&headers, "content-range"), "bytes */1000", "{range}");
    }

    // A unit this route does not know is ignored; so is a conditional range.
    let (status, _, body) = fx.api(&uri, &[("range", "items=0-5")]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.len(), 1000);
    let (status, _, _) = fx
        .api(&uri, &[("range", "bytes=0-9"), ("if-range", "\"x\"")])
        .await;
    assert_eq!(status, StatusCode::OK);

    // Unknown, evicted, non-MP4 and missing media are all 404.
    let (status, _, body) = fx.api("/frames/preview-media/99999", &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json(&body)["error_type"], "preview_media_not_found");

    let other = fx.path("chunk.mkv");
    std::fs::write(&other, &content).unwrap();
    let mkv = fx
        .db
        .insert_video_chunk_with_fps(&other, "monitor_1", 1.0)
        .await
        .unwrap();
    assert_eq!(
        fx.api(&format!("/frames/preview-media/{mkv}"), &[]).await.0,
        StatusCode::NOT_FOUND
    );

    let missing = fx
        .db
        .insert_video_chunk_with_fps(&fx.path("missing.mp4"), "monitor_1", 1.0)
        .await
        .unwrap();
    assert_eq!(
        fx.api(&format!("/frames/preview-media/{missing}"), &[])
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    fx.sql(
        "UPDATE video_chunks SET file_path = '', evicted_at = CURRENT_TIMESTAMP WHERE id = ?1",
        chunk,
    )
    .await;
    assert_eq!(fx.api(&uri, &[]).await.0, StatusCode::NOT_FOUND);
}

/// The thumbnail of a compacted frame comes out of its MP4 through ffmpeg.
/// Skipped when no ffmpeg/ffprobe pair is installed.
#[tokio::test]
async fn compacted_frame_thumbnail_is_extracted_from_the_mp4() {
    if screenpipe_core::find_ffmpeg_path().is_none() {
        eprintln!("ffmpeg not found; skipping the video thumbnail test");
        return;
    }
    let fx = fixture().await;
    let mp4 = fx.path("compacted.mp4");
    let made = std::process::Command::new(screenpipe_core::find_ffmpeg_path().unwrap())
        .args([
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=640x360:rate=1",
            "-frames:v",
            "5",
            "-pix_fmt",
            "yuv420p",
            &mp4,
        ])
        .status()
        .unwrap();
    assert!(made.success());

    let id = fx
        .snapshot_frame("will-compact.jpg", "monitor_1", 0, "Code")
        .await;
    let chunk = fx
        .db
        .insert_video_chunk_with_fps(&mp4, "monitor_1", 1.0)
        .await
        .unwrap();
    fx.db
        .compact_snapshots_queued(chunk, vec![(id, 2)])
        .await
        .unwrap();
    std::fs::remove_file(fx.path("will-compact.jpg")).unwrap();

    let (status, headers, body) = fx
        .api(
            &format!("/frames/{id}/thumbnail?width=320&fallback=false"),
            &[],
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(header(&headers, "x-screenpipe-thumbnail-width"), "320");
    assert_eq!(header(&headers, "x-screenpipe-thumbnail-height"), "180");
    assert_eq!(
        image::load_from_memory(&body).unwrap().dimensions(),
        (320, 180)
    );

    let (status, _, body) = fx
        .api(
            &format!(
                "/frames/preview-samples?app_name=Code&start_time={}&end_time={}",
                rfc3339(at(-60)),
                rfc3339(at(60))
            ),
            &[],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let frames = json(&body)["frames"].as_array().cloned().unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0]["source"], "video");
    assert_eq!(frames[0]["video_offset_seconds"].as_f64(), Some(2.0));

    let (status, _, _) = fx
        .api(
            &format!("/frames/preview-media/{chunk}"),
            &[("range", "bytes=0-99")],
        )
        .await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
}
