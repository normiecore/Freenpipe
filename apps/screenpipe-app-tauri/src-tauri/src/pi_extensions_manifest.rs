// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Pi extensions bundled with the desktop app for chat sessions.
//!
//! Same shape as screenpipe-core's manifest
//! (`screenpipe_core::agents::extensions_manifest`): each extension is one
//! asset under `src-tauri/assets/extensions/` plus one entry here, and the
//! chat setup in `pi.rs` installs the whole list with one call.

use std::path::Path;

use screenpipe_core::agents::extensions_manifest::{
    self, BundledExtension, ExtensionConditions, ExtensionGate,
};

/// Extensions the app writes into a chat project's `.pi/extensions/`, in
/// install order.
pub const APP_EXTENSIONS: &[BundledExtension] = &[
    BundledExtension {
        id: "web-search",
        file_name: "web-search.ts",
        source: include_str!("../assets/extensions/web-search.ts"),
        gate: ExtensionGate::WebSearch,
    },
    BundledExtension {
        id: "mcp-bridge",
        file_name: "mcp-bridge.ts",
        source: include_str!("../assets/extensions/mcp-bridge.ts"),
        gate: ExtensionGate::Always,
    },
];

/// Install the app's extensions into a chat project. `web_search` says
/// whether the web-search extension belongs there for this session.
pub fn sync_chat_extensions(project_dir: &Path, web_search: bool) -> Result<(), String> {
    let conditions = ExtensionConditions {
        web_search: Some(web_search),
        ..Default::default()
    };
    match extensions_manifest::sync_manifest(APP_EXTENSIONS, project_dir, &conditions)
        .into_iter()
        .next()
    {
        Some(failure) => Err(format!("Failed to install {}", failure)),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_lists_exactly_the_base_chat_extensions() {
        // The extensions the app installed for chat before the manifest
        // existed, with the condition each was installed under.
        let listed: Vec<(&str, &str, ExtensionGate)> = APP_EXTENSIONS
            .iter()
            .map(|e| (e.id, e.file_name, e.gate))
            .collect();
        assert_eq!(
            listed,
            vec![
                ("web-search", "web-search.ts", ExtensionGate::WebSearch),
                ("mcp-bridge", "mcp-bridge.ts", ExtensionGate::Always),
            ]
        );
        for ext in APP_EXTENSIONS {
            assert!(!ext.source.trim().is_empty(), "{} has no source", ext.id);
        }
    }

    #[test]
    fn every_app_asset_is_listed() {
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/extensions");
        for entry in std::fs::read_dir(assets).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            if name.ends_with(".ts") {
                assert!(
                    APP_EXTENSIONS.iter().any(|e| e.file_name == name),
                    "asset {name} is not in the app manifest"
                );
            }
        }
    }

    #[test]
    fn web_search_matches_the_pipe_copy() {
        // Chat and pipes offer the same `sp_web_search` tool.
        let app = extensions_manifest::find_in(APP_EXTENSIONS, "web-search").unwrap();
        let core = extensions_manifest::find("web-search").unwrap();
        assert_eq!(app.source, core.source);
    }

    #[test]
    fn chat_sync_writes_mcp_bridge_and_gates_web_search() {
        let dir = tempfile::tempdir().unwrap();
        let ext_dir = dir.path().join(".pi").join("extensions");

        sync_chat_extensions(dir.path(), false).unwrap();
        assert!(ext_dir.join("mcp-bridge.ts").exists());
        assert!(!ext_dir.join("web-search.ts").exists());

        sync_chat_extensions(dir.path(), true).unwrap();
        assert!(ext_dir.join("web-search.ts").exists());

        sync_chat_extensions(dir.path(), false).unwrap();
        assert!(!ext_dir.join("web-search.ts").exists());
        assert!(ext_dir.join("mcp-bridge.ts").exists());
    }
}
