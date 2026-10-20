// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! User-visible identity of the desktop app and its deep-link schemes.
//!
//! Only names a person reads live here (window titles, tray text, menu
//! labels). Interoperability identifiers keep their upstream spelling and are
//! defined where they are used: the `screenpipe` CLI binary, `~/.screenpipe`,
//! `SCREENPIPE_*` variables, settings keys, routes and the tray id.
//!
//! Deep links: the installer and `register_all()` claim only the schemes in
//! the `deep-link` section of the Tauri config (`freenpipe`, or
//! `freenpipe-beta` for the beta channel). URLs that use screenpipe's
//! `screenpipe://` scheme are still accepted once they reach the app, for
//! example from a pipe notification, a skill-generated chat link or a command
//! line argument, so existing pipes keep working. The app never registers
//! `screenpipe://` with the OS: an installed screenpipe owns it.

/// Product name shown to people.
pub const PRODUCT_NAME: &str = "Freenpipe";

/// Scheme of the links this app builds (copy-link actions and the like).
pub const DEEP_LINK_SCHEME: &str = "freenpipe";

/// Every scheme the in-app deep-link parser accepts: this app's own schemes
/// first, then the screenpipe compatibility alias. Lowercase.
pub const ACCEPTED_DEEP_LINK_SCHEMES: &[&str] = &["freenpipe", "freenpipe-beta", "screenpipe"];

/// Scheme part of `url` (before `://`), lowercased, when it is one this app
/// accepts.
pub fn deep_link_scheme(url: &str) -> Option<String> {
    let (scheme, _) = url.trim().split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    ACCEPTED_DEEP_LINK_SCHEMES
        .contains(&scheme.as_str())
        .then_some(scheme)
}

/// True when `url` is an in-app deep link (`freenpipe://…` or the
/// `screenpipe://…` alias), as opposed to a web or file URL.
pub fn is_app_deep_link(url: &str) -> bool {
    deep_link_scheme(url).is_some()
}

/// The part of an in-app deep link after `scheme://`, e.g. `meeting/12?live=1`.
/// `None` for anything that is not an in-app deep link.
// Outside tests only the macOS notification router needs it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn deep_link_route(url: &str) -> Option<&str> {
    deep_link_scheme(url)?;
    url.trim().split_once("://").map(|(_, rest)| rest)
}

/// First command-line argument that is an in-app deep link. On Windows and
/// Linux the OS hands a clicked link to the app as an argument.
pub fn find_deep_link_arg(args: &[String]) -> Option<String> {
    args.iter().find(|a| is_app_deep_link(a)).cloned()
}

/// Build a link into the app with the primary scheme, e.g.
/// `app_deep_link("frame/42")` gives `freenpipe://frame/42`.
pub fn app_deep_link(route: &str) -> String {
    format!(
        "{DEEP_LINK_SCHEME}://{}",
        route.trim_start_matches('/')
    )
}

/// Title for a window that carries the product's name.
pub fn window_title(suffix: Option<&str>) -> String {
    match suffix.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => format!("{PRODUCT_NAME} — {s}"),
        None => PRODUCT_NAME.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_primary_and_alias_schemes() {
        assert!(is_app_deep_link("freenpipe://frame/1"));
        assert!(is_app_deep_link("freenpipe-beta://timeline"));
        assert!(is_app_deep_link("screenpipe://meeting/12"));
        assert!(is_app_deep_link("  FreenPipe://settings"));
        assert_eq!(
            deep_link_scheme("ScreenPipe://view?path=x").as_deref(),
            Some("screenpipe")
        );
    }

    #[test]
    fn rejects_other_urls() {
        for url in [
            "https://example.test/freenpipe://x",
            "file:///C:/freenpipe",
            "freenpipe:frame/1",
            "screenpipe-beta://frame/1",
            "freenpipe",
            "",
        ] {
            assert!(!is_app_deep_link(url), "{url}");
            assert_eq!(deep_link_route(url), None, "{url}");
        }
    }

    #[test]
    fn route_is_scheme_independent() {
        assert_eq!(
            deep_link_route("freenpipe://meeting/12?live=1"),
            Some("meeting/12?live=1")
        );
        assert_eq!(
            deep_link_route("screenpipe://meeting/12?live=1"),
            Some("meeting/12?live=1")
        );
    }

    #[test]
    fn finds_a_deep_link_in_argv() {
        let args: Vec<String> = ["C:\\Freenpipe\\screenpipe-app.exe", "--autostart", "screenpipe://frame/9"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(find_deep_link_arg(&args).as_deref(), Some("screenpipe://frame/9"));
        let none: Vec<String> = vec!["app.exe".into(), "https://example.test".into()];
        assert_eq!(find_deep_link_arg(&none), None);
    }

    #[test]
    fn builds_links_with_the_primary_scheme() {
        assert_eq!(app_deep_link("frame/42"), "freenpipe://frame/42");
        assert_eq!(app_deep_link("/timeline"), "freenpipe://timeline");
        assert!(is_app_deep_link(&app_deep_link("view?path=a")));
    }

    #[test]
    fn primary_scheme_is_registered_and_alias_is_not() {
        // The schemes the OS is asked to route here come from the Tauri
        // config. They must be this app's own, never screenpipe's.
        for file in [
            "tauri.conf.json",
            "tauri.prod.conf.json",
            "tauri.beta.conf.json",
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
            let text = std::fs::read_to_string(&path).expect("tauri config readable");
            let conf: serde_json::Value = serde_json::from_str(&text).expect("tauri config is json");
            let schemes: Vec<&str> = conf["plugins"]["deep-link"]["desktop"]["schemes"]
                .as_array()
                .expect("desktop schemes listed")
                .iter()
                .filter_map(|v| v.as_str())
                .collect();
            assert!(!schemes.is_empty(), "{file}: no desktop scheme");
            for scheme in &schemes {
                assert!(scheme.starts_with("freenpipe"), "{file}: registers {scheme}");
                assert!(ACCEPTED_DEEP_LINK_SCHEMES.contains(scheme), "{file}: {scheme}");
            }
        }
    }

    #[test]
    fn window_titles_carry_the_product_name() {
        assert_eq!(window_title(None), "Freenpipe");
        assert_eq!(window_title(Some("  ")), "Freenpipe");
        assert_eq!(window_title(Some("search")), "Freenpipe — search");
    }
}
