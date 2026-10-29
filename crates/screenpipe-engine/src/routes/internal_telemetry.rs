// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Internal telemetry routes that accept and discard reports.
//!
//! Mounted by [`super::ext`]; this module adds no routes yet.

use super::ext::{DocumentedRoutes, ExtRouter};

/// Routes described in `/openapi.json`. Empty until the feature lands.
pub fn documented() -> DocumentedRoutes {
    DocumentedRoutes::empty()
}

/// Routes outside the OpenAPI spec. Empty until the feature lands.
pub fn router() -> ExtRouter {
    ExtRouter::new()
}
