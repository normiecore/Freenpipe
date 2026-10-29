// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! `POST /v1/chat/completions` is a proxy to the user's own AI gateway.
//!
//! With no gateway configured it must answer 503 `cloud_token_missing` and
//! send nothing. With a gateway configured (here a mock OpenAI-compatible
//! server on loopback) it must forward the body unchanged to
//! `{gateway}/chat/completions` with the gateway credential and pass the
//! answer back.

use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{HeaderMap, Request, StatusCode},
    routing::post,
    Json, Router,
};
use screenpipe_audio::audio_manager::AudioManagerBuilder;
use screenpipe_db::DatabaseManager;
use screenpipe_engine::SCServer;
use serde_json::{json, Value};
use tower::ServiceExt;

#[derive(Debug, Clone)]
struct LoggedRequest {
    path: String,
    authorization: Option<String>,
    body: Value,
}

type Log = Arc<Mutex<Vec<LoggedRequest>>>;

async fn mock_chat_completions(
    State(log): State<Log>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    log.lock().unwrap().push(LoggedRequest {
        path: "/v1/chat/completions".to_string(),
        authorization: headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        body,
    });
    Json(json!({
        "id": "chatcmpl-mock",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "hello from the mock gateway"},
            "finish_reason": "stop"
        }]
    }))
}

/// Start a mock OpenAI-compatible server; returns its address and request log.
async fn start_mock_gateway() -> (SocketAddr, Log) {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/v1/chat/completions", post(mock_chat_completions))
        .with_state(log.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, log)
}

async fn engine_router() -> Router {
    let db = Arc::new(
        DatabaseManager::new("sqlite::memory:", Default::default())
            .await
            .unwrap(),
    );
    let audio_manager = Arc::new(
        AudioManagerBuilder::new()
            .output_path(std::env::temp_dir().join("handvel-cloud-proxy-test"))
            .is_disabled(true)
            .build(db.clone())
            .await
            .unwrap(),
    );
    let server = SCServer::new(
        db,
        SocketAddr::from(([127, 0, 0, 1], 23961)),
        PathBuf::from(""),
        false,
        false,
        audio_manager,
        false,
        "balanced".to_string(),
    )
    .with_cloud_token(Some("gateway-test-key".to_string()));
    server.create_router().await
}

async fn post_chat(router: &Router) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"model":"x","messages":[]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

// One test owns the process-wide gateway setting, so the two phases run in
// order instead of racing each other.
#[tokio::test]
async fn chat_completions_proxy_needs_and_uses_the_users_gateway() {
    std::env::remove_var(screenpipe_core::ai_gateway::URL_ENV);
    assert!(screenpipe_core::ai_gateway::apply_setting(None));
    let (gateway_addr, log) = start_mock_gateway().await;
    let router = engine_router().await;

    // Phase 1: nothing configured -> 503 cloud_token_missing, nothing sent.
    let (status, body) = post_chat(&router).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"], "cloud_token_missing");
    assert!(log.lock().unwrap().is_empty(), "no request may leave");

    // Phase 2: the aiGatewayUrl setting points at the mock -> forwarded.
    assert!(screenpipe_core::ai_gateway::apply_setting(Some(&format!(
        "http://{gateway_addr}/v1"
    ))));
    let (status, body) = post_chat(&router).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(
        body["choices"][0]["message"]["content"],
        "hello from the mock gateway"
    );

    let requests = log.lock().unwrap().clone();
    assert_eq!(requests.len(), 1, "the mock logged exactly one request");
    let request = &requests[0];
    assert_eq!(request.path, "/v1/chat/completions");
    assert_eq!(
        request.authorization.as_deref(),
        Some("Bearer gateway-test-key")
    );
    assert_eq!(request.body, json!({"model": "x", "messages": []}));

    assert!(screenpipe_core::ai_gateway::apply_setting(None));
}
