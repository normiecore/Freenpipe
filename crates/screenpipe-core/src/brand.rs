// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Product identity shared by the engine and the desktop app.
//!
//! Interoperability identifiers (the `screenpipe` CLI binary, `~/.screenpipe`,
//! `SCREENPIPE_*` variables, routes and settings keys) keep their upstream
//! names and are not defined here.

/// Product name shown to people.
pub const PRODUCT_NAME: &str = "Handvel";

/// Lowercase markers that identify this product's own processes and windows
/// in captured app names. On Windows the captured app name is the
/// executable's file description, which the bundler sets to the product name
/// ("Handvel", "Handvel Dev", "Handvel Beta"); the bare CLI and the app
/// binary are still called `screenpipe` / `screenpipe-app`, and data captured
/// by screenpipe itself carries its name. Frames recorded before the rename
/// (decision 9) carry "Freenpipe". All three are matched.
pub const OWN_APP_NAME_MARKERS: &[&str] = &["handvel", "freenpipe", "screenpipe"];

/// True when `app_name` belongs to this product's own UI or processes, so
/// capture and search can leave it out. Case-insensitive substring match.
pub fn is_own_app_name(app_name: &str) -> bool {
    let lower = app_name.to_lowercase();
    OWN_APP_NAME_MARKERS.iter().any(|m| lower.contains(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_every_product_variant() {
        for name in [
            "Handvel",
            "Handvel Dev",
            "Handvel Beta",
            "handvel",
            "Freenpipe",
            "Freenpipe Dev",
            "screenpipe",
            "screenpipe-app",
            "screenpipe - Development",
        ] {
            assert!(is_own_app_name(name), "{name}");
        }
    }

    #[test]
    fn leaves_other_apps_alone() {
        for name in ["Google Chrome", "WezTerm", "Pipedream", "Free Download Manager", ""] {
            assert!(!is_own_app_name(name), "{name}");
        }
    }
}
