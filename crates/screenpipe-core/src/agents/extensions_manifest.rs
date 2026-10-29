// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Data-driven list of the Pi extensions bundled with screenpipe-core.
//!
//! A Pi project directory gets its extensions as TypeScript files under
//! `.pi/extensions/`. Each bundled extension is one asset file plus one entry
//! in [`BUNDLED_EXTENSIONS`]; the install code walks the manifest instead of
//! carrying a hand-written function per extension. Adding an extension means
//! adding its asset under `assets/extensions/` and one entry here.
//!
//! Every entry has a [`ExtensionGate`]. `Always` entries are written on every
//! sync. A gated entry is written when its condition holds and removed when it
//! does not, but only by a caller that decided that condition: the conditions
//! a caller leaves undecided keep whatever is on disk. That mirrors how the
//! pipe setup and the executor each manage their own subset of extensions.
//!
//! The desktop app keeps its own manifest (`pi_extensions_manifest.rs` in the
//! app crate) built from the same types.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::{debug, info};

/// When a bundled extension belongs in a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionGate {
    /// Written on every sync.
    Always,
    /// Only when a web search provider is configured
    /// ([`crate::web_search`]). The extension's `sp_web_search` tool calls
    /// the engine's `/v1/web-search` route, which talks to that provider.
    WebSearch,
    /// Only for pipes that declare data-permission restrictions.
    PipePermissions,
    /// Only for pipes that enable `subagent` in their front matter.
    SubAgent,
}

/// One extension shipped inside the binary.
#[derive(Debug, Clone, Copy)]
pub struct BundledExtension {
    /// Stable identifier, also the file stem.
    pub id: &'static str,
    /// File name inside `.pi/extensions/`.
    pub file_name: &'static str,
    /// The TypeScript source written to disk.
    pub source: &'static str,
    /// When the extension is present.
    pub gate: ExtensionGate,
}

/// Extensions bundled with screenpipe-core, in install order.
pub const BUNDLED_EXTENSIONS: &[BundledExtension] = &[
    BundledExtension {
        id: "web-search",
        file_name: "web-search.ts",
        source: include_str!("../../assets/extensions/web-search.ts"),
        gate: ExtensionGate::WebSearch,
    },
    BundledExtension {
        id: "context-pruning",
        file_name: "context-pruning.ts",
        source: include_str!("../../assets/extensions/context-pruning.ts"),
        gate: ExtensionGate::Always,
    },
    BundledExtension {
        id: "orphan-guard",
        file_name: "orphan-guard.ts",
        source: include_str!("../../assets/extensions/orphan-guard.ts"),
        gate: ExtensionGate::Always,
    },
    BundledExtension {
        id: "mcp-bridge",
        file_name: "mcp-bridge.ts",
        source: include_str!("../../assets/extensions/mcp-bridge.ts"),
        gate: ExtensionGate::Always,
    },
    BundledExtension {
        id: "screenpipe-permissions",
        file_name: "screenpipe-permissions.ts",
        source: include_str!("../../assets/extensions/screenpipe-permissions.ts"),
        gate: ExtensionGate::PipePermissions,
    },
    BundledExtension {
        id: "sub-agent",
        file_name: "sub-agent.ts",
        source: include_str!("../../assets/extensions/sub-agent.ts"),
        gate: ExtensionGate::SubAgent,
    },
];

/// The gate conditions one caller has decided. `None` means "not mine to
/// decide": extensions behind that gate are left as they are.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExtensionConditions {
    pub web_search: Option<bool>,
    pub pipe_permissions: Option<bool>,
    pub sub_agent: Option<bool>,
}

/// What a sync should do with one extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionAction {
    Install,
    Remove,
    Leave,
}

impl ExtensionConditions {
    fn decided(&self, gate: ExtensionGate) -> Option<bool> {
        match gate {
            ExtensionGate::Always => Some(true),
            ExtensionGate::WebSearch => self.web_search,
            ExtensionGate::PipePermissions => self.pipe_permissions,
            ExtensionGate::SubAgent => self.sub_agent,
        }
    }

    /// The action a sync takes for an extension behind `gate`.
    pub fn action_for(&self, gate: ExtensionGate) -> ExtensionAction {
        match self.decided(gate) {
            Some(true) => ExtensionAction::Install,
            Some(false) => ExtensionAction::Remove,
            None => ExtensionAction::Leave,
        }
    }
}

/// `<project>/.pi/extensions`.
pub fn extensions_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(".pi").join("extensions")
}

/// Look up a bundled extension by id.
pub fn find(id: &str) -> Option<&'static BundledExtension> {
    find_in(BUNDLED_EXTENSIONS, id)
}

/// Look up an extension by id in any manifest.
pub fn find_in<'a>(manifest: &'a [BundledExtension], id: &str) -> Option<&'a BundledExtension> {
    manifest.iter().find(|ext| ext.id == id)
}

impl BundledExtension {
    /// Write the extension into the project.
    pub fn install(&self, project_dir: &Path) -> Result<()> {
        let dir = extensions_dir(project_dir);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating extensions dir {}", dir.display()))?;
        let path = dir.join(self.file_name);
        std::fs::write(&path, self.source)
            .with_context(|| format!("writing {} extension", self.id))?;
        debug!("{} extension installed at {:?}", self.id, path);
        Ok(())
    }

    /// Delete the extension from the project if it is there.
    pub fn remove(&self, project_dir: &Path) -> Result<()> {
        let path = extensions_dir(project_dir).join(self.file_name);
        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("removing {} extension", self.id))?;
            info!("{} extension removed from {:?}", self.id, project_dir);
        }
        Ok(())
    }

    /// Install or remove the extension according to `conditions`.
    pub fn apply(&self, project_dir: &Path, conditions: &ExtensionConditions) -> Result<()> {
        match conditions.action_for(self.gate) {
            ExtensionAction::Install => self.install(project_dir),
            ExtensionAction::Remove => self.remove(project_dir),
            ExtensionAction::Leave => Ok(()),
        }
    }
}

/// An extension that could not be installed or removed.
#[derive(Debug)]
pub struct ExtensionSyncError {
    pub id: &'static str,
    pub error: anyhow::Error,
}

impl std::fmt::Display for ExtensionSyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} extension: {:#}", self.id, self.error)
    }
}

/// Bring a project's extensions in line with `manifest` for the conditions
/// the caller decided. Every entry is attempted; the failures are returned
/// in manifest order so the caller can choose to warn or to fail.
pub fn sync_manifest(
    manifest: &[BundledExtension],
    project_dir: &Path,
    conditions: &ExtensionConditions,
) -> Vec<ExtensionSyncError> {
    manifest
        .iter()
        .filter_map(|ext| {
            ext.apply(project_dir, conditions)
                .err()
                .map(|error| ExtensionSyncError { id: ext.id, error })
        })
        .collect()
}

/// [`sync_manifest`] over the extensions bundled with screenpipe-core.
pub fn sync_bundled_extensions(
    project_dir: &Path,
    conditions: &ExtensionConditions,
) -> Vec<ExtensionSyncError> {
    sync_manifest(BUNDLED_EXTENSIONS, project_dir, conditions)
}

/// Turn sync failures into a single error (the first one), for callers that
/// treat any failure as fatal.
pub fn first_error(errors: Vec<ExtensionSyncError>) -> Result<()> {
    match errors.into_iter().next() {
        Some(e) => Err(e.error.context(format!("{} extension", e.id))),
        None => Ok(()),
    }
}

/// Whether the web-search extension belongs in a run. The tool goes through
/// the local engine, so every model provider may use it; it is offered only
/// when a web search provider is configured.
pub fn web_search_wanted() -> bool {
    crate::web_search::is_configured()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(project: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(extensions_dir(project))
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    #[test]
    fn manifest_lists_exactly_the_base_extensions() {
        // The extensions screenpipe-core installed before the manifest
        // existed, with the condition each was installed under.
        let listed: Vec<(&str, &str, ExtensionGate)> = BUNDLED_EXTENSIONS
            .iter()
            .map(|e| (e.id, e.file_name, e.gate))
            .collect();
        assert_eq!(
            listed,
            vec![
                ("web-search", "web-search.ts", ExtensionGate::WebSearch),
                (
                    "context-pruning",
                    "context-pruning.ts",
                    ExtensionGate::Always
                ),
                ("orphan-guard", "orphan-guard.ts", ExtensionGate::Always),
                ("mcp-bridge", "mcp-bridge.ts", ExtensionGate::Always),
                (
                    "screenpipe-permissions",
                    "screenpipe-permissions.ts",
                    ExtensionGate::PipePermissions
                ),
                ("sub-agent", "sub-agent.ts", ExtensionGate::SubAgent),
            ]
        );
    }

    #[test]
    fn manifest_entries_are_well_formed() {
        let mut ids = std::collections::HashSet::new();
        for ext in BUNDLED_EXTENSIONS {
            assert!(ids.insert(ext.id), "{} listed twice", ext.id);
            assert_eq!(ext.file_name, format!("{}.ts", ext.id));
            assert!(!ext.source.trim().is_empty(), "{} has no source", ext.id);
        }
        // Every asset in the extensions folder is in the manifest.
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/extensions");
        for entry in std::fs::read_dir(assets).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            if name.ends_with(".ts") {
                assert!(
                    BUNDLED_EXTENSIONS.iter().any(|e| e.file_name == name),
                    "asset {name} is not in the manifest"
                );
            }
        }
    }

    #[test]
    fn executor_conditions_install_always_entries_and_decide_web_search() {
        let dir = tempfile::tempdir().unwrap();
        let conditions = ExtensionConditions {
            web_search: Some(false),
            ..Default::default()
        };
        assert!(sync_bundled_extensions(dir.path(), &conditions).is_empty());
        assert_eq!(
            installed(dir.path()),
            vec!["context-pruning.ts", "mcp-bridge.ts", "orphan-guard.ts"]
        );

        let conditions = ExtensionConditions {
            web_search: Some(true),
            ..Default::default()
        };
        assert!(sync_bundled_extensions(dir.path(), &conditions).is_empty());
        assert!(installed(dir.path()).contains(&"web-search.ts".to_string()));
        assert_eq!(
            std::fs::read_to_string(extensions_dir(dir.path()).join("web-search.ts")).unwrap(),
            find("web-search").unwrap().source
        );
    }

    #[test]
    fn undecided_gates_leave_files_alone() {
        let dir = tempfile::tempdir().unwrap();
        // Pipe setup installs the permissions extension...
        let pipe_setup = ExtensionConditions {
            pipe_permissions: Some(true),
            sub_agent: Some(false),
            ..Default::default()
        };
        assert!(sync_bundled_extensions(dir.path(), &pipe_setup).is_empty());
        assert!(installed(dir.path()).contains(&"screenpipe-permissions.ts".to_string()));

        // ...and the executor, which does not decide permissions, keeps it.
        let executor = ExtensionConditions {
            web_search: Some(false),
            ..Default::default()
        };
        assert!(sync_bundled_extensions(dir.path(), &executor).is_empty());
        assert!(installed(dir.path()).contains(&"screenpipe-permissions.ts".to_string()));

        // A decided `false` removes it again.
        let no_restrictions = ExtensionConditions {
            pipe_permissions: Some(false),
            sub_agent: Some(false),
            ..Default::default()
        };
        assert!(sync_bundled_extensions(dir.path(), &no_restrictions).is_empty());
        assert!(!installed(dir.path()).contains(&"screenpipe-permissions.ts".to_string()));
    }

    #[test]
    fn sync_reports_failures_per_extension() {
        let dir = tempfile::tempdir().unwrap();
        // A file where the extensions directory should be makes every write fail.
        std::fs::create_dir_all(dir.path().join(".pi")).unwrap();
        std::fs::write(dir.path().join(".pi").join("extensions"), b"not a dir").unwrap();
        let errors = sync_bundled_extensions(
            dir.path(),
            &ExtensionConditions {
                web_search: Some(true),
                ..Default::default()
            },
        );
        let ids: Vec<&str> = errors.iter().map(|e| e.id).collect();
        assert_eq!(
            ids,
            vec![
                "web-search",
                "context-pruning",
                "orphan-guard",
                "mcp-bridge"
            ]
        );
        assert!(first_error(errors).is_err());
        assert!(first_error(Vec::new()).is_ok());
    }

    #[test]
    fn web_search_extension_calls_the_local_route_only() {
        let source = find("web-search").unwrap().source;
        assert!(source.contains("\"sp_web_search\""));
        assert!(source.contains("/v1/web-search"));
        assert!(source.contains("SCREENPIPE_LOCAL_API_KEY"));
        // The search provider's credentials stay in the engine.
        assert!(!source.contains("SCREENPIPE_AI_GATEWAY_URL"));
        assert!(!source.contains("SCREENPIPE_WEB_SEARCH_API_KEY"));
        assert!(!source.contains("process.env.SCREENPIPE_API_KEY"));
    }

    #[test]
    fn app_copy_of_the_web_search_extension_matches() {
        // The desktop app ships its own copy for chat sessions
        // (src-tauri/assets/extensions). Both copies are the same file so
        // pipes and chat get the same tool.
        let app_copy = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/screenpipe-app-tauri/src-tauri/assets/extensions/web-search.ts");
        if let Ok(app_source) = std::fs::read_to_string(&app_copy) {
            assert_eq!(app_source, find("web-search").unwrap().source);
        }
    }
}
