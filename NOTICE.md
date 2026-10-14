# NOTICE

Freenpipe is a fork of [screenpipe](https://github.com/screenpipe/screenpipe).

## Upstream base

- Commit `892199f742e46d0c5d9e8c06687b35ca7c2b6547` (2026-06-10), the last commit published under the MIT License. On that same day upstream relicensed later work under the Screenpipe Commercial License (`81e412ff5`).
- Copyright (c) 2024-2026 louis030195. MIT License (see `LICENSE.md`).
- Upstream's `ee/` directory was licensed separately under the Screenpipe Enterprise License. It is **not** included. Freenpipe's replacements for the few places that referenced it are original code.

## Later work

Behaviour added upstream after the relicense (screenpipe v2.5.27 → v2.7.104) was reimplemented clean-room from static analysis of the shipped v2.7.104 Windows binary, without access to post-relicense source. Only interoperability identifiers (API routes, JSON fields, SQL schema names, CLI flags, settings keys) intentionally match upstream.

"screenpipe" is a name used by its respective owners. Freenpipe is not affiliated with or endorsed by them.
