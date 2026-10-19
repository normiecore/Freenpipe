# permission-flow

[![CI](https://github.com/veecore/permission-flow/actions/workflows/ci.yml/badge.svg)](https://github.com/veecore/permission-flow/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/permission-flow.svg)](https://crates.io/crates/permission-flow)
[![docs.rs](https://img.shields.io/docsrs/permission-flow)](https://docs.rs/permission-flow)
[![npm](https://img.shields.io/npm/v/%40veecore%2Ftauri-plugin-permission-flow-api)](https://www.npmjs.com/package/@veecore/tauri-plugin-permission-flow-api)

`permission-flow` provides a Rust-friendly API for presenting the macOS
permission guidance flow backed by the Swift/AppKit implementation in this
workspace.

![permission-flow demo](https://raw.githubusercontent.com/veecore/permission-flow/main/assets/demo-readme.gif)

## What it does

- Creates a `PermissionFlowController` on the macOS main thread
- Starts guided flows for supported privacy panes
- Infers a likely host app bundle with `AppPath::suggested_host_app()`
- Exposes host-app permission status checks through
  `Permission::authorization_state()`

Supported permissions:

- Accessibility
- Input Monitoring
- Screen Recording
- App Management
- Bluetooth
- Developer Tools
- Full Disk Access
- Media & Apple Music

## Quick start

```rust
use permission_flow::{AppPath, Permission, PermissionFlowController, StartFlowOptions};

let controller = PermissionFlowController::new()?;
let app_path = AppPath::suggested_host_app()
    .expect("expected a host app bundle in this launch context");

controller.start_flow(StartFlowOptions::new(Permission::ACCESSIBILITY, app_path))?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Platform behavior

This crate is designed for macOS, but it now compiles on other operating
systems too.

Outside macOS:

- `PermissionFlowController::new()` succeeds with a no-op controller
- `start_flow()` and `stop_current_flow()` become no-ops
- `Permission::authorization_state()` returns `PermissionAuthorizationState::Unknown`
- `AppPath::suggested_host_app()` returns `None`

That keeps cross-platform Rust workspaces buildable without pretending the
actual macOS permission UI exists on those platforms.

## Important status warning

`Permission::authorization_state()` reports what the current host process or
host app can determine about its own permission state.

It does **not** authoritatively answer whether an arbitrary target `.app`
bundle already has the requested permission.

This means:

- if the target app is the current host app, the status is meaningful
- if the target app is some other app bundle, treat the status as host-app
  information only

## Runtime note

Because the final macOS executable links Swift runtime libraries, downstream
binary crates currently need this in their `build.rs`:

```rust
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
}
```

## Acknowledgements

The Swift side of this crate builds on top of the excellent
[`PermissionFlow`](https://github.com/jaywcjlove/PermissionFlow) project and
uses [`swift-rs`](https://github.com/Brendonovich/swift-rs) for the bridge into
Rust.
