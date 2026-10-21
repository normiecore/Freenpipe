// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Mount point for feature route modules.
//!
//! Each planned feature that serves HTTP routes owns one module under
//! `routes/` and exposes two functions:
//!
//! - `documented() -> DocumentedRoutes`: handlers that appear in
//!   `/openapi.json`. Build them with an oasgen server and wrap it with
//!   [`documented_routes!`]:
//!
//!   ```ignore
//!   pub fn documented() -> DocumentedRoutes {
//!       documented_routes!(oasgen::Server::axum().get("/starred-sessions", list_sessions))
//!   }
//!   ```
//!
//! - `router() -> ExtRouter`: routes oasgen cannot describe (websockets,
//!   streams, opaque or multipart bodies).
//!
//! [`feature_routes`] collects every module once and `server.rs` mounts the
//! result in one place, so a feature fills its own module and leaves
//! `server.rs` alone. All modules start empty; until a feature lands it adds
//! no route and no OpenAPI path.

use std::fmt;
use std::sync::Arc;

use axum::Router;
use oasgen::{OpenAPI, Operation, PathItem, RefOr};

use crate::server::AppState;

use super::{
    activity_ledger, agent, ai_feedback, artifacts, cloud_agents, connect_broker, data_sync_proxy,
    frames, internal_telemetry, live_views, local_chat, meeting_overlay, pipe_stream,
    request_origin, response_format, search_only, semantic, starred, structured_outputs, vision,
    web_search, workflows,
};

/// Router type every feature module returns. State is the shared [`AppState`].
pub type ExtRouter = Router<Arc<AppState>>;

/// Routes from one feature that are described in the OpenAPI spec.
pub struct DocumentedRoutes {
    spec: Option<Arc<OpenAPI>>,
    router: ExtRouter,
}

impl DocumentedRoutes {
    /// No documented routes.
    pub fn empty() -> Self {
        Self {
            spec: None,
            router: Router::new(),
        }
    }

    /// Pair a frozen oasgen spec with the router it produced. Use
    /// [`documented_routes!`] rather than calling this directly.
    pub fn from_parts(spec: Arc<OpenAPI>, router: ExtRouter) -> Self {
        Self {
            spec: Some(spec),
            router,
        }
    }

    /// Number of OpenAPI paths this feature adds.
    pub fn path_count(&self) -> usize {
        self.spec.as_ref().map_or(0, |s| s.paths.paths.len())
    }

    /// The OpenAPI paths this feature adds, in registration order.
    pub fn paths(&self) -> Vec<String> {
        self.spec
            .as_ref()
            .map(|s| s.paths.paths.keys().cloned().collect())
            .unwrap_or_default()
    }
}

/// Freeze an oasgen axum server built by a feature module into
/// [`DocumentedRoutes`]. The server's state type must be `Arc<AppState>`.
#[allow(unused_macros)]
macro_rules! documented_routes {
    ($server:expr) => {{
        let frozen = $server.freeze();
        let spec = frozen.openapi.clone();
        $crate::routes::ext::DocumentedRoutes::from_parts(spec, frozen.into_router())
    }};
}
#[allow(unused_imports)]
pub(crate) use documented_routes;

/// Everything one feature module contributes.
pub struct FeatureRoutes {
    pub name: &'static str,
    pub documented: DocumentedRoutes,
    pub router: ExtRouter,
}

/// Names of the feature modules, in mount order.
pub const FEATURE_MODULES: &[&str] = &[
    "activity_ledger",
    "agent",
    "ai_feedback",
    "artifacts",
    "cloud_agents",
    "connect_broker",
    "data_sync_proxy",
    "frames",
    "internal_telemetry",
    "live_views",
    "local_chat",
    "meeting_overlay",
    "pipe_stream",
    "request_origin",
    "response_format",
    "search_only",
    "semantic",
    "starred",
    "structured_outputs",
    "vision",
    "web_search",
    "workflows",
];

/// Collect the routes of every feature module.
pub fn feature_routes() -> Vec<FeatureRoutes> {
    macro_rules! feature {
        ($module:ident) => {
            FeatureRoutes {
                name: stringify!($module),
                documented: $module::documented(),
                router: $module::router(),
            }
        };
    }
    vec![
        feature!(activity_ledger),
        feature!(agent),
        feature!(ai_feedback),
        feature!(artifacts),
        feature!(cloud_agents),
        feature!(connect_broker),
        feature!(data_sync_proxy),
        feature!(frames),
        feature!(internal_telemetry),
        feature!(live_views),
        feature!(local_chat),
        feature!(meeting_overlay),
        feature!(pipe_stream),
        feature!(request_origin),
        feature!(response_format),
        feature!(search_only),
        feature!(semantic),
        feature!(starred),
        feature!(structured_outputs),
        feature!(vision),
        feature!(web_search),
        feature!(workflows),
    ]
}

/// Two sources that both define the same method on the same path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteCollision {
    pub feature: &'static str,
    pub path: String,
    pub method: &'static str,
}

impl fmt::Display for RouteCollision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "route {} {} from feature module '{}' is already registered",
            self.method.to_uppercase(),
            self.path,
            self.feature
        )
    }
}

impl std::error::Error for RouteCollision {}

/// Merge every feature's OpenAPI paths into `spec` (the core spec built in
/// `server.rs`). A path both sides define is merged method by method, the
/// same way axum merges the routers; defining the same method twice is an
/// error. Returns how many new paths were added.
pub fn extend_spec(
    spec: &mut OpenAPI,
    features: &[FeatureRoutes],
) -> Result<usize, RouteCollision> {
    let mut added = 0;
    for feature in features {
        let Some(feature_spec) = feature.documented.spec.as_ref() else {
            continue;
        };
        for (path, incoming) in &feature_spec.paths.paths {
            let RefOr::Item(incoming) = incoming else {
                continue;
            };
            match spec.paths.paths.get_mut(path) {
                None => {
                    spec.paths
                        .paths
                        .insert(path.clone(), RefOr::Item(incoming.clone()));
                    added += 1;
                }
                Some(RefOr::Item(existing)) => {
                    merge_path_item(existing, incoming, feature.name, path)?;
                }
                Some(RefOr::Reference { .. }) => {
                    return Err(RouteCollision {
                        feature: feature.name,
                        path: path.clone(),
                        method: "any",
                    });
                }
            }
        }
    }
    Ok(added)
}

fn merge_path_item(
    existing: &mut PathItem,
    incoming: &PathItem,
    feature: &'static str,
    path: &str,
) -> Result<(), RouteCollision> {
    fn slot(
        target: &mut Option<Operation>,
        add: &Option<Operation>,
        method: &'static str,
        feature: &'static str,
        path: &str,
    ) -> Result<(), RouteCollision> {
        match (target.as_ref(), add) {
            (_, None) => Ok(()),
            (None, Some(op)) => {
                *target = Some(op.clone());
                Ok(())
            }
            (Some(_), Some(_)) => Err(RouteCollision {
                feature,
                path: path.to_string(),
                method,
            }),
        }
    }
    slot(&mut existing.get, &incoming.get, "get", feature, path)?;
    slot(&mut existing.put, &incoming.put, "put", feature, path)?;
    slot(&mut existing.post, &incoming.post, "post", feature, path)?;
    slot(
        &mut existing.delete,
        &incoming.delete,
        "delete",
        feature,
        path,
    )?;
    slot(
        &mut existing.options,
        &incoming.options,
        "options",
        feature,
        path,
    )?;
    slot(&mut existing.head, &incoming.head, "head", feature, path)?;
    slot(&mut existing.patch, &incoming.patch, "patch", feature, path)?;
    slot(&mut existing.trace, &incoming.trace, "trace", feature, path)?;
    Ok(())
}

/// One router holding every feature's documented and undocumented routes.
pub fn into_router(features: Vec<FeatureRoutes>) -> ExtRouter {
    features.into_iter().fold(Router::new(), |acc, feature| {
        acc.merge(feature.documented.router).merge(feature.router)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use oasgen::oasgen;
    use serde_json::{json, Value};

    #[oasgen]
    async fn seam_probe(State(_state): State<Arc<AppState>>) -> crate::JsonResponse<Value> {
        crate::JsonResponse(json!({ "ok": true }))
    }

    #[oasgen]
    async fn seam_probe_post(State(_state): State<Arc<AppState>>) -> crate::JsonResponse<Value> {
        crate::JsonResponse(json!({ "ok": true }))
    }

    fn probe_feature(name: &'static str, documented: DocumentedRoutes) -> FeatureRoutes {
        FeatureRoutes {
            name,
            documented,
            router: Router::new(),
        }
    }

    #[test]
    fn every_planned_module_is_mounted_once() {
        let names: Vec<&str> = feature_routes().iter().map(|f| f.name).collect();
        assert_eq!(names, FEATURE_MODULES);
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len());
        // PLAN.md section 5 lists these stubs.
        for planned in [
            "starred",
            "semantic",
            "artifacts",
            "structured_outputs",
            "live_views",
            "ai_feedback",
            "activity_ledger",
            "vision",
            "workflows",
            "agent",
            "connect_broker",
            "local_chat",
            "web_search",
            "data_sync_proxy",
            "cloud_agents",
            "internal_telemetry",
            "request_origin",
            "response_format",
            "search_only",
            "pipe_stream",
            "meeting_overlay",
        ] {
            assert!(names.contains(&planned), "{planned} is not mounted");
        }
    }

    #[test]
    fn feature_modules_do_not_collide_with_each_other() {
        let features = feature_routes();
        let mut spec = OpenAPI::default();
        let added = extend_spec(&mut spec, &features).expect("feature routes collide");
        // Every documented feature path lands in the spec exactly once.
        let distinct: std::collections::HashSet<String> =
            features.iter().flat_map(|f| f.documented.paths()).collect();
        assert_eq!(added, distinct.len());
        assert_eq!(spec.paths.paths.len(), distinct.len());
        // The merged router builds (axum panics on overlapping routes).
        let _router = into_router(features);
    }

    #[test]
    fn documented_routes_wraps_an_oasgen_server() {
        let routes = documented_routes!(oasgen::Server::axum()
            .get("/__seam/probe", seam_probe)
            .post("/__seam/probe", seam_probe_post));
        assert_eq!(routes.path_count(), 1);
        assert_eq!(routes.paths(), vec!["/__seam/probe".to_string()]);
        assert_eq!(DocumentedRoutes::empty().path_count(), 0);
    }

    #[test]
    fn extend_spec_adds_new_paths_and_merges_methods() {
        let mut core = OpenAPI::default();
        core.paths.paths.insert(
            "/__seam/probe".into(),
            RefOr::Item(PathItem::get(Operation::default())),
        );
        let features = vec![probe_feature(
            "probe",
            documented_routes!(oasgen::Server::axum()
                .post("/__seam/probe", seam_probe_post)
                .get("/__seam/other", seam_probe)),
        )];
        let added = extend_spec(&mut core, &features).unwrap();
        assert_eq!(added, 1, "only /__seam/other is a new path");
        assert_eq!(core.paths.paths.len(), 2);
        let RefOr::Item(item) = &core.paths.paths["/__seam/probe"] else {
            panic!("path item expected");
        };
        assert!(item.get.is_some() && item.post.is_some());
    }

    #[test]
    fn extend_spec_rejects_a_method_defined_twice() {
        let mut core = OpenAPI::default();
        core.paths.paths.insert(
            "/__seam/probe".into(),
            RefOr::Item(PathItem::get(Operation::default())),
        );
        let features = vec![probe_feature(
            "probe",
            documented_routes!(oasgen::Server::axum().get("/__seam/probe", seam_probe)),
        )];
        let err = extend_spec(&mut core, &features).unwrap_err();
        assert_eq!(
            err,
            RouteCollision {
                feature: "probe",
                path: "/__seam/probe".into(),
                method: "get",
            }
        );
        assert!(err.to_string().contains("GET /__seam/probe"));
    }

    #[test]
    fn empty_features_leave_the_spec_untouched() {
        let mut core = OpenAPI::default();
        core.paths.paths.insert(
            "/health".into(),
            RefOr::Item(PathItem::get(Operation::default())),
        );
        let before = serde_json::to_value(&core).unwrap();
        let empty = vec![
            probe_feature("a", DocumentedRoutes::empty()),
            probe_feature("b", DocumentedRoutes::empty()),
        ];
        let added = extend_spec(&mut core, &empty).unwrap();
        assert_eq!(added, 0);
        assert_eq!(serde_json::to_value(&core).unwrap(), before);
    }
}
