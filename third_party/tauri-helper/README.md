# `tauri_helper`

![Crates.io License](https://img.shields.io/crates/l/tauri_helper)
[![Crates.io](https://img.shields.io/crates/v/tauri_helper)](https://crates.io/crates/tauri_helper)
[![Docs.rs](https://docs.rs/tauri-helper/badge.svg)](https://docs.rs/tauri_helper)
![Crates.io Size](https://img.shields.io/crates/size/tauri_helper)


`tauri_helper` is a collection of tools and utilities designed to simplify the development of Tauri applications. It provides macros and automation to streamline command registration by auto-collecting every `#[tauri::command]` function across your workspace — no extra annotation required.

This workspace includes the following crates:
- **`tauri_helper_core`**: Core utilities for workspace management and command collection.
- **`tauri_helper_macros`**: Procedural macros for automating Tauri command registration and error handling.

---

## Features

- **Auto command collection**: Every `#[tauri::command]` function is picked up automatically.
- **Specta support**: Functions that also carry `#[specta::specta]` are emitted separately for TypeScript binding generation.
- **Workspace-aware**: Scans all workspace members; single-crate apps work without any configuration.
- **Stable ordering**: Commands are sorted alphabetically by function name in all generated output.

### Macros

- **`specta_collect_commands!`**: Generates a `tauri_specta::collect_commands!` invocation for all `#[tauri::command]` + `#[specta::specta]` functions.
- **`tauri_collect_commands!`**: Generates a `tauri::generate_handler!` invocation for all `#[tauri::command]` functions.
- **`array_collect_commands!`**: Generates a `[&str; N]` array of collected command paths.
- **`WithLogging`** *(requires `tracing` feature, experimental)*: Implements `From` for enum variants with automatic error logging.

---

## Installation

Add `tauri-helper` to both `[dependencies]` and `[build-dependencies]` in your `Cargo.toml`:

```toml
[dependencies]
tauri-helper = "0.2.1"

[build-dependencies]
tauri-helper = "0.2.1"
```

To use the `WithLogging` derive macro, enable the `tracing` feature:

```toml
[dependencies]
tauri-helper = { version = "0.2.1", features = ["tracing"] }
```

---

## Setup

Call `generate_command_file` in `build.rs` **before** `tauri_build::build()`:

```rust
fn main() {
    tauri_helper::generate_command_file(tauri_helper::TauriHelperOptions::default());
    tauri_build::build();
}
```

For multi-crate workspaces, list all members in your root `Cargo.toml`:

```toml
[workspace]
members = [
    ".",
    "local-crates/some-commands",
]
```

**Single-crate apps** with a bare `[workspace]` table (common in `src-tauri` layouts) are detected automatically — no `members` override needed.

---

## Usage

Annotate your commands with `#[tauri::command]`. Add `#[specta::specta]` to any command that should also appear in the TypeScript bindings. Nothing else is required.

```rust
#[tauri::command]
#[specta::specta]
fn greet(name: String) -> String {
    format!("Hello, {}!", name)
}

#[tauri::command]
#[specta::specta]
fn calculate_sum(a: i32, b: i32) -> i32 {
    a + b
}
```

Then wire up the handler and specta builder:

```rust
fn main() {
    let builder = tauri_specta::Builder::<tauri::Wry>::new()
        .commands(specta_collect_commands!());

    #[cfg(debug_assertions)]
    builder
        .export(
            specta_typescript::Typescript::default()
                .bigint(specta_typescript::BigIntExportBehavior::Number),
            "../src/bindings.ts",
        )
        .expect("failed to export TypeScript bindings");

    tauri::Builder::default()
        .invoke_handler(tauri_collect_commands!())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

---

## Feature Flags

- **`tracing`**: Enables the `WithLogging` derive macro. Optional; must be explicitly enabled.

---

## Notes

- **`WithLogging` stability**: Experimental — may have breaking changes. Not recommended for production.
- **`#[cfg(...)]` and platform-specific commands**: Source scanning runs outside the compiler's cfg resolution, so platform-gated commands are collected on all platforms. Their bodies are still compiled only on the target platform, so this only becomes an issue if the function signature itself references a platform-only type.

---

## Contributing

Contributions are welcome! Please open an issue or submit a pull request.

---

## License

This project is licensed under the [MIT License](LICENSE).
