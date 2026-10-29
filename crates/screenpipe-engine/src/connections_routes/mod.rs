// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Route groups for individual integrations, nested under `/connections`.
//!
//! Each integration that needs its own endpoints owns one module here with a
//! `routes()` function returning a router on [`ConnectionsState`]. [`router`]
//! merges them, and `connections_api::router` merges the result at a single
//! point, so adding an integration's routes never touches
//! `connections_api.rs`. Paths are relative to `/connections`.

use axum::Router;

use crate::connections_api::ConnectionsState;

pub mod bee;
pub mod browser_act;
pub mod imap;
pub mod slack;
pub mod telegram;
pub mod webhook_proxy;

/// Router type every integration module returns.
pub type ConnectionsRouter = Router<ConnectionsState>;

/// Names of the integration route groups, in merge order.
pub const ROUTE_GROUPS: &[&str] = &[
    "bee",
    "browser_act",
    "imap",
    "slack",
    "telegram",
    "webhook_proxy",
];

/// Every integration's routes in one router.
pub fn router() -> ConnectionsRouter {
    Router::new()
        .merge(bee::routes())
        .merge(browser_act::routes())
        .merge(imap::routes())
        .merge(slack::routes())
        .merge(telegram::routes())
        .merge(webhook_proxy::routes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_route_group_has_a_module() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/connections_routes");
        let mut files: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".rs") && n != "mod.rs")
            .map(|n| n.trim_end_matches(".rs").to_string())
            .collect();
        files.sort();
        assert_eq!(files, ROUTE_GROUPS);
    }

    #[test]
    fn merged_router_builds() {
        // Merging panics on overlapping routes, so building is the check.
        let _router: ConnectionsRouter = router();
    }
}
