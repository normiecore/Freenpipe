# NOTICE

Freenpipe is a fork of [screenpipe](https://github.com/screenpipe/screenpipe).

## Upstream base

- Commit `892199f742e46d0c5d9e8c06687b35ca7c2b6547` (2026-06-10), the last commit published under the MIT License. On that same day upstream relicensed later work under the Screenpipe Commercial License (`81e412ff5`).
- Copyright (c) 2024-2026 louis030195. MIT License (see `LICENSE.md`).
- Upstream's `ee/` directory was licensed separately under the Screenpipe Enterprise License. It is **not** included. Freenpipe's replacements for the few places that referenced it are original code.

## Later work

Until 2026-10-29, specifications for behaviour added upstream after the relicense (screenpipe v2.5.27 → v2.7.104) were written from static analysis of the shipped v2.7.104 Windows binary, without access to post-relicense source, and work packages were implemented from those specifications. That analysis stopped on 2026-10-29. The earlier specifications are kept unchanged in `docs/clean-room/legacy-specs/`, and code merged before that date is unchanged. Since then, implementers work only from specifications written from public material, the MIT base and Freenpipe's own design, with only interoperability identifiers carried over from the earlier ones (`docs/clean-room/SOURCING.md`). Each work package started since then is required to have a provenance record in `docs/clean-room/provenance/`. Only interoperability identifiers (API routes, JSON fields, SQL schema names, CLI flags, settings keys) intentionally match upstream.

"screenpipe" is a name used by its respective owners. Freenpipe is not affiliated with or endorsed by them.

## Third-party model weights

The Windows installer redistributes two speaker-diarization models (pyannote segmentation-3.0, MIT; a WeSpeaker CAM++ VoxCeleb embedding model, CC BY 4.0). Their sources, licences and credits are in `crates/screenpipe-audio/models/pyannote/NOTICE.md`, which ships next to them in `<install>/models/pyannote/`.
