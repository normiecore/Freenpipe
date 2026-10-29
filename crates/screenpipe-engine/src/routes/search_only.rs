// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Search-only mode: the engine keeps serving saved history after capture stops.
//!
//! Mounted by [`super::ext`]; this module adds no routes yet, and its
//! middleware lets every request through.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;

use super::ext::{DocumentedRoutes, ExtRouter};
use crate::server::AppState;

/// Routes described in `/openapi.json`. Empty until the feature lands.
pub fn documented() -> DocumentedRoutes {
    DocumentedRoutes::empty()
}

/// Routes outside the OpenAPI spec. Empty until the feature lands.
pub fn router() -> ExtRouter {
    ExtRouter::new()
}

/// Runs on every authenticated request (wired in `server.rs`). Passes
/// requests through unchanged for now.
pub async fn search_only_layer(
    State(_state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    next.run(request).await
}
