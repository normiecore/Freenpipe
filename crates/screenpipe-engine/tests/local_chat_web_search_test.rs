// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! W3-25 acceptance harness, in-process: the full engine router served over
//! loopback TCP with API auth on, a temporary data dir, a mock
//! OpenAI-compatible server and a mock SearXNG instance.
//!
//! - `POST /v1/local/chat/completions` needs the local API key, answers 400
//!   with the missing-preset error when no default preset exists, and with a
//!   default preset pointing at the mock streams the mock's SSE back. The
//!   mock sees the preset's key, never the local one.
//! - `POST /v1/web-search` answers 501 `web_search_not_configured` when no
//!   provider is configured and returns `{content, sources}` from the mock
//!   SearXNG once `SCREENPIPE_WEB_SEARCH_URL` points at it.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::{
    body::{Body, Bytes},
    extract::{Query, State},
    http::{header, HeaderMap},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use screenpipe_audio::audio_manager::AudioManagerBuilder;
use screenpipe_db::DatabaseManager;
use screenpipe_engine::SCServer;
use serde_json::{json, Value};

const LOCAL_KEY: &str = "fp-test-local-key";
const PRESET_KEY: &str = "sk-preset-for-tests";

type Seen = Arc<Mutex<Vec<(Option<String>, Value)>>>;

async fn serve(router: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });
    addr
}

/// Mock OpenAI server: streams three deltas and `[DONE]` as SSE.
async fn mock_openai() -> (SocketAddr, Seen) {
    let seen: Seen = Arc::default();
    let router = Router::new()
        .route(
            "/v1/chat/completions",
            post(
                |State(seen): State<Seen>, headers: HeaderMap, Json(body): Json<Value>| async move {
                    let auth = headers
                        .get(header::AUTHORIZATION)
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string);
                    seen.lock().unwrap().push((auth, body));
                    let events = ["Hel", "lo", "."]
                        .into_iter()
                        .map(|t| {
                            format!(
                                "data: {}\n\n",
                                json!({"object": "chat.completion.chunk",
                                       "choices": [{"index": 0, "delta": {"content": t}}]})
                            )
                        })
                        .chain(["data: [DONE]\n\n".to_string()])
                        .map(|e| Ok::<_, std::io::Error>(Bytes::from(e)));
                    (
                        [(header::CONTENT_TYPE, "text/event-stream")],
                        Body::from_stream(futures::stream::iter(events.collect::<Vec<_>>())),
                    )
                        .into_response()
                },
            ),
        )
        .with_state(seen.clone());
    (serve(router).await, seen)
}

/// Mock SearXNG instance.
async fn mock_searxng() -> SocketAddr {
    let router = Router::new().route(
        "/search",
        get(
            |Query(q): Query<std::collections::HashMap<String, String>>| async move {
                assert_eq!(q.get("format").map(String::as_str), Some("json"));
                Json(json!({
                    "query": q.get("q"),
                    "results": [
                        {"title": "Example result", "url": "https://example.test/one",
                         "content": "First snippet."},
                        {"title": "Second", "url": "https://example.test/two",
                         "content": "Second snippet."}
                    ]
                }))
            },
        ),
    );
    serve(router).await
}

async fn engine(data_dir: &Path) -> SocketAddr {
    let db = Arc::new(
        DatabaseManager::new("sqlite::memory:", Default::default())
            .await
            .unwrap(),
    );
    let audio_manager = Arc::new(
        AudioManagerBuilder::new()
            .output_path(data_dir.join("audio"))
            .is_disabled(true)
            .build(db.clone())
            .await
            .unwrap(),
    );
    let mut server = SCServer::new(
        db,
        SocketAddr::from(([127, 0, 0, 1], 0)),
        data_dir.to_path_buf(),
        false,
        false,
        audio_manager,
        false,
        "balanced".to_string(),
    );
    server.api_auth = true;
    server.api_auth_key = Some(LOCAL_KEY.to_string());
    serve(server.create_router().await).await
}

fn write_store(data_dir: &Path, presets: Value) {
    let store = json!({ "settings": { "aiPresets": presets } });
    std::fs::write(
        data_dir.join("store.bin"),
        serde_json::to_vec_pretty(&store).unwrap(),
    )
    .unwrap();
}

async fn post_json(
    client: &reqwest::Client,
    url: &str,
    key: Option<&str>,
    body: &str,
) -> reqwest::Response {
    let mut request = client
        .post(url)
        .header("Content-Type", "application/json")
        .body(body.to_string());
    if let Some(key) = key {
        request = request.bearer_auth(key);
    }
    request.send().await.unwrap()
}

// One test owns the process environment (web-search variables, gateway
// setting), so the phases run in order.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_chat_and_web_search_acceptance() {
    for name in [
        screenpipe_core::web_search::PROVIDER_ENV,
        screenpipe_core::web_search::URL_ENV,
        screenpipe_core::web_search::API_KEY_ENV,
        screenpipe_core::ai_gateway::URL_ENV,
        screenpipe_core::ai_gateway::KEY_ENV,
    ] {
        std::env::remove_var(name);
    }
    assert!(screenpipe_core::ai_gateway::apply_setting(None));

    let data_dir = tempfile::tempdir().unwrap();
    let engine = engine(data_dir.path()).await;
    let client = reqwest::Client::new();
    let chat_url = format!("http://{engine}/v1/local/chat/completions");
    let search_url = format!("http://{engine}/v1/web-search");
    let chat_body = r#"{"messages":[{"role":"user","content":"say hello"}],"stream":true}"#;

    // The local API key is required like on every other route.
    let resp = post_json(&client, &chat_url, None, chat_body).await;
    assert_eq!(resp.status().as_u16(), 403);
    let resp = post_json(&client, &search_url, Some("wrong-key"), r#"{"query":"x"}"#).await;
    assert_eq!(resp.status().as_u16(), 403);

    // No settings store at all: 400 with the missing-preset message.
    let resp = post_json(&client, &chat_url, Some(LOCAL_KEY), chat_body).await;
    assert_eq!(resp.status().as_u16(), 400);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], "no_default_preset");
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("no default AI preset"));

    // Presets but none marked default: the same error.
    write_store(
        data_dir.path(),
        json!([{"id": "a", "provider": "custom", "url": "http://127.0.0.1:1/v1",
                "model": "m", "defaultPreset": false}]),
    );
    let resp = post_json(&client, &chat_url, Some(LOCAL_KEY), chat_body).await;
    assert_eq!(resp.status().as_u16(), 400);

    // A default preset without a base URL.
    write_store(
        data_dir.path(),
        json!([{"id": "nourl", "provider": "custom", "url": "", "model": "m",
                "defaultPreset": true}]),
    );
    let resp = post_json(&client, &chat_url, Some(LOCAL_KEY), chat_body).await;
    assert_eq!(resp.status().as_u16(), 400);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], "preset_missing_base_url");

    // Default preset on the mock OpenAI server: streamed SSE comes back.
    let (openai, seen) = mock_openai().await;
    write_store(
        data_dir.path(),
        json!([{"id": "mock", "provider": "custom", "url": format!("http://{openai}/v1"),
                "model": "mock-model", "apiKey": PRESET_KEY, "defaultPreset": true,
                "maxContextChars": 200000, "prompt": ""}]),
    );
    let resp = post_json(&client, &chat_url, Some(LOCAL_KEY), chat_body).await;
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(
        resp.headers()[header::CONTENT_TYPE.as_str()],
        "text/event-stream"
    );
    assert_eq!(resp.headers()[header::CACHE_CONTROL.as_str()], "no-cache");
    let mut stream = resp.bytes_stream();
    let mut text = String::new();
    use futures::StreamExt;
    while let Some(chunk) = stream.next().await {
        text.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
    }
    let events: Vec<&str> = text.split("\n\n").filter(|e| !e.is_empty()).collect();
    assert_eq!(events.len(), 4, "{text}");
    assert!(events[0].contains("\"Hel\""));
    assert_eq!(events[3], "data: [DONE]");
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            seen[0].0.as_deref(),
            Some(format!("Bearer {PRESET_KEY}").as_str())
        );
        assert_eq!(seen[0].1["model"], "mock-model");
        assert_eq!(seen[0].1["stream"], true);
    }
    // The preset key never comes back to the caller, even in errors.
    write_store(
        data_dir.path(),
        json!([{"id": "down", "provider": "custom", "url": "http://127.0.0.1:9/v1",
                "model": "m", "apiKey": PRESET_KEY, "defaultPreset": true}]),
    );
    let resp = post_json(&client, &chat_url, Some(LOCAL_KEY), chat_body).await;
    assert_eq!(resp.status().as_u16(), 502);
    let text = resp.text().await.unwrap();
    assert!(text.contains("upstream_unreachable"), "{text}");
    assert!(!text.contains(PRESET_KEY), "{text}");

    // Web search with nothing configured: disabled, nothing sent.
    let resp = post_json(&client, &search_url, Some(LOCAL_KEY), r#"{"query":"rust"}"#).await;
    assert_eq!(resp.status().as_u16(), 501);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "web_search_not_configured");
    assert!(!body["message"].as_str().unwrap().is_empty());

    // Pointed at the mock SearXNG: sources come back. curl's default form
    // content type is accepted too.
    let searx = mock_searxng().await;
    std::env::set_var(
        screenpipe_core::web_search::URL_ENV,
        format!("http://{searx}"),
    );
    let resp = client
        .post(&search_url)
        .bearer_auth(LOCAL_KEY)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(r#"{"query":"rust"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = resp.json().await.unwrap();
    let sources = body["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 2);
    assert_eq!(sources[0]["url"], "https://example.test/one");
    assert_eq!(sources[0]["title"], "Example result");
    assert!(body["content"]
        .as_str()
        .unwrap()
        .contains("1. Example result"));

    for name in [
        screenpipe_core::web_search::URL_ENV,
        screenpipe_core::web_search::PROVIDER_ENV,
    ] {
        std::env::remove_var(name);
    }
}
