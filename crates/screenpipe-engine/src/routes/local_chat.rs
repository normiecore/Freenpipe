// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! `POST /v1/local/chat/completions`: OpenAI-compatible chat completions on
//! the user's default AI preset. The logic lives in [`crate::local_chat`].
//!
//! The route is not in `/openapi.json`: its body and answer are OpenAI's
//! chat-completions format, streamed when the caller asks for it. Like every
//! other route it sits behind the engine's API-key check.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::response::Response;
use axum::routing::post;

use super::ext::{DocumentedRoutes, ExtRouter};
use crate::local_chat::{self, GatewayAccess, Timeouts, LOCAL_CHAT_PATH, MAX_REQUEST_BYTES};
use crate::server::AppState;

/// No documented routes.
pub fn documented() -> DocumentedRoutes {
    DocumentedRoutes::empty()
}

/// The proxy route.
pub fn router() -> ExtRouter {
    ExtRouter::new().route(
        LOCAL_CHAT_PATH,
        post(local_chat_completions).layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES)),
    )
}

async fn local_chat_completions(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let pushed = (**state.cloud_token.load()).clone();
    let gateway = GatewayAccess::current(pushed.as_deref());
    // The store can be encrypted (keychain lookup) and lives on disk.
    let data_dir = state.screenpipe_dir.clone();
    let store = tokio::task::spawn_blocking(move || {
        crate::cli::store_file::read_store_for(&data_dir).map_err(|e| format!("{e:#}"))
    })
    .await
    .unwrap_or_else(|e| Err(format!("settings reader stopped: {e}")));
    local_chat::proxy(store, &gateway, &body, Timeouts::default()).await
}
