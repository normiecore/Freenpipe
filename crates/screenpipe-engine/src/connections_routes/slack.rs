// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Slack user-token routes (`/connections/slack/*`).
//!
//! Merged into `/connections` by [`super::router`]; adds no routes yet.

use super::ConnectionsRouter;

/// This integration's routes, relative to `/connections`.
pub fn routes() -> ConnectionsRouter {
    ConnectionsRouter::new()
}
