# permission-flow

[![CI](https://github.com/veecore/permission-flow/actions/workflows/ci.yml/badge.svg)](https://github.com/veecore/permission-flow/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/permission-flow.svg)](https://crates.io/crates/permission-flow)
[![docs.rs](https://img.shields.io/docsrs/permission-flow)](https://docs.rs/permission-flow)
[![npm](https://img.shields.io/npm/v/%40veecore%2Ftauri-plugin-permission-flow-api)](https://www.npmjs.com/package/@veecore/tauri-plugin-permission-flow-api)

`permission-flow` is a macOS-first workspace for guiding users through system
permission onboarding from Rust apps.

![permission-flow demo](https://raw.githubusercontent.com/veecore/permission-flow/main/assets/demo-readme.gif)

## Pick a package

The workspace currently contains three packages:

- `permission-flow`: the core Rust crate backed by the Swift/AppKit flow
- `permission-flow-iced`: headless `iced` helpers built on top of the core crate
- `tauri-plugin-permission-flow`: a Tauri plugin plus guest-side JS bindings

Use `permission-flow` if you want direct Rust control over controller lifetime.

Use `permission-flow-iced` if you are building an `iced` app and want the
permission-flow and host-status wiring handled for you.

Use `tauri-plugin-permission-flow` if your UI lives in a Tauri frontend and you
want Rust plus JS bindings.

## Install

Add the core crate:

```toml
[dependencies]
permission-flow = "0.1.40"
```

Or use one of the higher-level packages:

```toml
[dependencies]
permission-flow-iced = "0.1.40"
tauri-plugin-permission-flow = "0.1.40"
```

The Tauri guest package is published separately on npm as:

```json
{
  "dependencies": {
    "@veecore/tauri-plugin-permission-flow-api": "0.1.40"
  }
}
```

## What you get

The workspace supports guided flows for these permission areas:

- Accessibility
- Input Monitoring
- Screen Recording
- App Management
- Bluetooth
- Developer Tools
- Full Disk Access
- Media & Apple Music

The macOS flow is focused on:

- opening the right Settings area
- showing the target app bundle clearly
- guiding the user through the drag/drop step where needed
- giving you a host-app permission status signal you can reflect in your UI

## Important status note

The status helpers in this workspace describe what the current host app or host
process can determine about its own permission state.

They do **not** authoritatively answer whether some arbitrary target `.app`
bundle already has that permission.

## Runtime note

Because the final macOS executable links against Swift runtime libraries,
downstream binary crates currently need this in their `build.rs`:

```rust
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
}
```

## Package docs

- Core crate: [`crates/flow/README.md`](./crates/flow/README.md)
- iced helpers: [`crates/flow-iced/README.md`](./crates/flow-iced/README.md)
- Tauri plugin: [`crates/tauri-plugin-permission-flow/README.md`](./crates/tauri-plugin-permission-flow/README.md)

## Acknowledgements

This workspace builds on top of the excellent
[`PermissionFlow`](https://github.com/jaywcjlove/PermissionFlow) Swift package
and uses [`swift-rs`](https://github.com/Brendonovich/swift-rs) for the Rust to
Swift bridge.

## Development

The CI pipeline checks:

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --lib`
- `cargo check -p permission-flow-iced --examples`
- `cargo publish -p permission-flow --dry-run`
