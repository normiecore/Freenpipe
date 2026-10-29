// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Feature-specific database code, one module per feature.
//!
//! Each module holds an `impl DatabaseManager` block (plus any row types the
//! feature needs), so new queries go into the feature's own file instead of
//! `db.rs`. Existing functions in `db.rs` are changed in place. The modules
//! start empty; they add methods as their features land.

pub mod activity;
pub mod display_layout;
pub mod feedback;
pub mod frames_preview;
pub mod meetings_v2;
pub mod outputs;
pub mod pipe_events;
pub mod retention_strip;
pub mod search_parsed;
pub mod semantic;
pub mod starred;
pub mod tags;

/// Names of the modules above.
pub const MODULES: &[&str] = &[
    "activity",
    "display_layout",
    "feedback",
    "frames_preview",
    "meetings_v2",
    "outputs",
    "pipe_events",
    "retention_strip",
    "search_parsed",
    "semantic",
    "starred",
    "tags",
];

#[cfg(test)]
mod tests {
    #[test]
    fn module_list_matches_the_files() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ext");
        let mut files: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".rs") && n != "mod.rs")
            .map(|n| n.trim_end_matches(".rs").to_string())
            .collect();
        files.sort();
        assert_eq!(files, super::MODULES);
    }
}
