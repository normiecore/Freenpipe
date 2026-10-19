use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct CargoToml {
    #[allow(dead_code)]
    pub package: Package,
    #[serde(default)]
    pub workspace: Workspace,
}

#[derive(Debug, Deserialize, Default)]
pub struct Workspace {
    #[serde(default)]
    pub members: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Package {
    #[allow(dead_code)]
    pub name: String,
    #[allow(dead_code)]
    pub version: String,
    #[allow(dead_code)]
    pub edition: String,
}

/// Configuration options for `generate_command_file`.
#[derive(Deserialize, Default)]
pub struct TauriHelperOptions {
    /// Workspace members to scan. When `None`, uses `[workspace].members` from the nearest
    /// `Cargo.toml`. If that list is empty (common for single-crate apps with a bare
    /// `[workspace]` table), the current crate (`"."`) is scanned automatically.
    pub members: Option<Vec<String>>,
}

impl TauriHelperOptions {
    pub fn new(members: Option<Vec<String>>) -> Self {
        Self { members }
    }
}
