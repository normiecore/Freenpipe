# permission-flow-iced

[![CI](https://github.com/veecore/permission-flow/actions/workflows/ci.yml/badge.svg)](https://github.com/veecore/permission-flow/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/permission-flow-iced.svg)](https://crates.io/crates/permission-flow-iced)
[![docs.rs](https://img.shields.io/docsrs/permission-flow-iced)](https://docs.rs/permission-flow-iced)
[![npm](https://img.shields.io/npm/v/%40veecore%2Ftauri-plugin-permission-flow-api)](https://www.npmjs.com/package/@veecore/tauri-plugin-permission-flow-api)

Headless `iced` helpers for [`permission-flow`](../flow).

![permission-flow demo](https://raw.githubusercontent.com/veecore/permission-flow/main/assets/demo-readme.gif)

`permission-flow-iced` is for `iced` apps that want help with two things:

- starting the macOS permission flow from a normal `iced` button
- keeping host-app permission status refreshed without reinventing the wiring

`PermissionFlowButton` eagerly owns its own `PermissionFlowController`, so each
button can manage its own permission-flow lifecycle without relying on a global
controller.

It also provides a built-in `subscription()` and `update(...)` pair for
host-app status refreshes, so apps do not need to hand-roll timer or
window-focus refresh logic themselves.

## What it is

This crate is a headless integration helper, not a custom `iced` widget.

You still render your own UI, but the crate owns:

- controller lifetime
- `start_flow()` / `press()`
- focus/timer refresh policy
- current host-app authorization state

## Platform behavior

`permission-flow-iced` is intended for macOS, but it compiles on other
platforms too because the underlying `permission-flow` crate now exposes a
no-op controller there.

Outside macOS, button presses do nothing and status resolves to `Unknown`, but
your `iced` app can still build cleanly in a cross-platform workspace.

## Important status warning

`Permission::authorization_state()` and `PermissionFlowButton::button_state()`
reflect what the current host process or host app can determine about its own
permission state.

They do **not** verify whether the arbitrary `.app` bundle you pass in
`StartFlowOptions` or `PermissionFlowButton::new(...)` already has that
permission.

In practice:

- If the suggested app bundle is the current host app, the status is meaningful.
- If the suggested app bundle is some other app, treat the status as host-app
  information only, not as an authoritative target-app check.

## Quick start

```rust
use iced::{Task, Subscription};
use permission_flow::{AppPath, Permission, StartFlowOptions};
use permission_flow_iced::{PermissionFlowButton, PermissionFlowButtonMessage};

struct App {
    button: PermissionFlowButton,
}

enum Message {
    PermissionFlow(PermissionFlowButtonMessage),
    Pressed,
}

impl App {
    fn new() -> Self {
        let app_path = AppPath::suggested_host_app()
            .expect("expected a host app bundle in this launch context");

        Self {
            button: PermissionFlowButton::with_options(
                StartFlowOptions::new(Permission::ACCESSIBILITY, app_path),
            )
            .expect("button should initialize"),
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Pressed => {
                let _ = self.button.press();
            }
            Message::PermissionFlow(inner) => {
                let _ = self.button.update(inner);
            }
        }

        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        self.button.subscription().map(Message::PermissionFlow)
    }
}
```

## Example app

Run the included example with:

```bash
cargo run -p permission-flow-iced --example button
```

The example infers a host app bundle from the current launch context, starts
the permission flow for that app, and refreshes the displayed status when the
window regains focus.
