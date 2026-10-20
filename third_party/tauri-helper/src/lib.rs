use rayon::prelude::*;
use std::collections::HashSet;
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use syn::parse_file;
use tauri_helper_core::{find_workspace_dir, get_member_pkg_name, get_workspace_members};

pub use tauri_helper_core::types::TauriHelperOptions;
pub use tauri_helper_macros::*;

#[derive(Debug, Clone)]
struct SourceFile {
    path: PathBuf,
    module_path: String,
}

#[derive(Debug, Clone)]
struct CollectedCommand {
    path: String,
    specta: bool,
}

fn module_path_for_source(source_dir: &Path, source_path: &Path) -> Option<String> {
    let relative = source_path.strip_prefix(source_dir).ok()?;
    if relative == Path::new("main.rs") || relative == Path::new("lib.rs") {
        return Some(String::new());
    }

    let mut parts: Vec<String> = relative
        .iter()
        .map(|part| part.to_string_lossy().to_string())
        .collect();
    let last = parts.last_mut()?;
    if !last.ends_with(".rs") {
        return None;
    }
    last.truncate(last.len() - ".rs".len());

    if last == "mod" {
        parts.pop();
    }

    Some(parts.join("::"))
}

fn collect_rs_files(source_dir: &Path, dir: &Path, files: &mut Vec<SourceFile>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(source_dir, &path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs")
            && let Some(module_path) = module_path_for_source(source_dir, &path)
        {
            files.push(SourceFile { path, module_path });
        }
    }
}

fn collect_path_module_files(files: &mut Vec<SourceFile>) {
    let snapshot = files.clone();
    for source_file in &snapshot {
        let Ok(content) = fs::read_to_string(&source_file.path) else {
            continue;
        };
        let Ok(ast) = parse_file(&content) else {
            continue;
        };
        for item in &ast.items {
            let syn::Item::Mod(mod_item) = item else {
                continue;
            };
            if mod_item.content.is_some() {
                continue;
            }
            for attr in &mod_item.attrs {
                if !attr.path().is_ident("path") {
                    continue;
                }
                let path_str = match &attr.meta {
                    syn::Meta::NameValue(meta) => {
                        if let syn::Expr::Lit(syn::ExprLit {
                            lit: syn::Lit::Str(lit_str),
                            ..
                        }) = &meta.value
                        {
                            Some(lit_str.clone())
                        } else {
                            None
                        }
                    }
                    syn::Meta::List(_) => {
                        if let Ok(syn::Expr::Lit(syn::ExprLit {
                            lit: syn::Lit::Str(lit_str),
                            ..
                        })) = attr.parse_args::<syn::Expr>()
                        {
                            Some(lit_str)
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                let Some(path_str) = path_str else {
                    continue;
                };
                let Some(parent) = source_file.path.parent() else {
                    continue;
                };
                let path = parent.join(path_str.value());
                if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                    continue;
                }
                let mod_name = mod_item.ident.to_string();
                let module_path = if source_file.module_path.is_empty() {
                    mod_name
                } else {
                    format!("{}::{}", source_file.module_path, mod_name)
                };
                files.push(SourceFile { path, module_path });
            }
        }
    }
}

fn is_tauri_command_attr(path: &syn::Path) -> bool {
    path.segments.len() == 2
        && path.segments[0].ident == "tauri"
        && path.segments[1].ident == "command"
}

fn is_specta_attr(path: &syn::Path) -> bool {
    path.segments.len() == 2
        && path.segments[0].ident == "specta"
        && path.segments[1].ident == "specta"
}

fn collect_commands_from_source(source_file: &SourceFile) -> Vec<CollectedCommand> {
    let Ok(content) = fs::read_to_string(&source_file.path) else {
        return Vec::new();
    };
    if !content.contains("tauri::command") {
        return Vec::new();
    }

    let Ok(ast) = parse_file(&content) else {
        return Vec::new();
    };

    let mut commands = Vec::new();

    for item in ast.items {
        let syn::Item::Fn(func) = item else {
            continue;
        };

        if !func
            .attrs
            .iter()
            .any(|attr| is_tauri_command_attr(attr.path()))
        {
            continue;
        }

        let specta = func.attrs.iter().any(|attr| is_specta_attr(attr.path()));

        let fn_name = func.sig.ident.to_string();
        let path = if source_file.module_path.is_empty() {
            fn_name
        } else {
            format!("{}::{}", source_file.module_path, fn_name)
        };

        commands.push(CollectedCommand { path, specta });
    }

    commands
}

fn full_command_path(package_name: &str, command_path: &str) -> String {
    format!("{}::{command_path}", package_name.replace('-', "_"))
}

fn write_command_list(path: &Path, commands: &[String]) {
    let mut file = File::create(path)
        .unwrap_or_else(|e| panic!("failed to create {}: {e}", path.display()));
    for command in commands {
        writeln!(file, "{command}")
            .unwrap_or_else(|e| panic!("failed to write to {}: {e}", path.display()));
    }
}

fn sort_by_fn_name(commands: &mut [String]) {
    commands.sort_by(|a, b| {
        let a_fn = a.rsplit("::").next().unwrap_or(a.as_str());
        let b_fn = b.rsplit("::").next().unwrap_or(b.as_str());
        a_fn.cmp(b_fn)
    });
}

#[allow(clippy::needless_doctest_main)]
/// Scans each workspace member's `src/` tree for functions annotated with
/// `#[tauri::command]` and writes two files per crate under
/// `target/tauri_commands_list/`:
///
/// - `{crate}.txt` — all collected commands (consumed by [`tauri_collect_commands!`])
/// - `{crate}_specta.txt` — commands that also carry `#[specta::specta]` (consumed by
///   [`specta_collect_commands!`])
///
/// Call this from `build.rs` **before** `tauri_build::build()`:
///
/// ```rust,ignore
/// fn main() {
///     tauri_helper::generate_command_file(tauri_helper::TauriHelperOptions::default());
///     tauri_build::build();
/// }
/// ```
///
/// Every `#[tauri::command]` function is collected automatically — no extra
/// annotation is required. Add `#[specta::specta]` to functions that should also
/// appear in the TypeScript bindings.
///
/// # Options
///
/// `TauriHelperOptions::members` overrides which workspace members are scanned.
/// When `None`, `[workspace].members` from the nearest `Cargo.toml` is used; an
/// empty list (bare `[workspace]` table, common in `src-tauri`) falls back to `"."`.
///
/// # Notes
///
/// `#[cfg(...)]`-gated commands are collected on all platforms because source
/// scanning runs outside the compiler's cfg resolution. Platform-specific commands
/// will produce linker errors on unsupported platforms if they reference
/// platform-only APIs in their bodies — guard those bodies with `#[cfg]` as usual.
///
/// # Panics
///
/// - `target/tauri_commands_list/` cannot be created or written to.
/// - A member crate's `Cargo.toml` cannot be read or parsed.
pub fn generate_command_file(options: TauriHelperOptions) {
    let workspace_root = find_workspace_dir(
        Path::new(&env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set")),
    );
    let commands_dir = workspace_root.join("target").join("tauri_commands_list");
    fs::create_dir_all(&commands_dir)
        .unwrap_or_else(|e| panic!("failed to create {}: {e}", commands_dir.display()));

    let workspace_members = options.members.clone().unwrap_or_else(|| {
        let members = get_workspace_members(&workspace_root);
        if members.is_empty() {
            vec![".".to_string()]
        } else {
            members
        }
    });

    workspace_members.par_iter().for_each(|member| {
        let manifest_dir = workspace_root.join(member);
        let package_name = get_member_pkg_name(&manifest_dir);

        let src_dir = manifest_dir.join("src");
        // Watch the src dir so cargo rebuilds when files are added or removed.
        println!("cargo:rerun-if-changed={}", src_dir.display());

        let mut source_files = Vec::new();
        collect_rs_files(&src_dir, &src_dir, &mut source_files);
        collect_path_module_files(&mut source_files);

        // Deduplicate: a file can appear from both the filesystem walk and a #[path] reference.
        let mut seen_paths = HashSet::new();
        source_files.retain(|f| seen_paths.insert(f.path.clone()));

        source_files.sort_by(|left, right| left.path.cmp(&right.path));

        let mut commands = Vec::new();
        for source_file in &source_files {
            println!("cargo:rerun-if-changed={}", source_file.path.display());
            commands.extend(collect_commands_from_source(source_file));
        }

        if commands.is_empty() {
            return;
        }

        let mut tauri_commands: Vec<String> = commands
            .iter()
            .map(|command| full_command_path(&package_name, &command.path))
            .collect();
        sort_by_fn_name(&mut tauri_commands);
        tauri_commands.dedup();

        let mut specta_commands: Vec<String> = commands
            .iter()
            .filter(|command| command.specta)
            .map(|command| full_command_path(&package_name, &command.path))
            .collect();
        sort_by_fn_name(&mut specta_commands);
        specta_commands.dedup();

        let crate_name = package_name.replace('-', "_");
        write_command_list(
            &commands_dir.join(format!("{crate_name}.txt")),
            &tauri_commands,
        );

        if !specta_commands.is_empty() {
            write_command_list(
                &commands_dir.join(format!("{crate_name}_specta.txt")),
                &specta_commands,
            );
        }
    });
}
