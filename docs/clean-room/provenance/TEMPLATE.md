# <WP-ID> provenance

- **Work package:** <WP-ID>, <title>
- **Branch:** `wp/<WP-ID>` (find the merge with `git log --grep "Merge <WP-ID>"`)
- **Implemented by:** <person, or "Claude Code session" with its date>
- **Dates:** <started> to <finished>

## Spec items implemented

Every item must be in `docs/clean-room/specs/` with a source tag (SOURCING.md).

| Item | Spec | Source tag | Redacted by | Behaviour by |
|---|---|---|---|---|
| <e.g. audio-012> | `specs/audio.md` | <own / mit / public / interop> | <name> | <name> |

The implementer is neither the redactor nor the behaviour author of any item above.

## What the implementer was given and read

- Brief or prompt: <who wrote it, and confirm it contains no description taken from legacy-specs/>
- Specs: <files and items>
- Freenpipe and MIT-base code: <areas>
- Third-party documentation: <libraries, URLs>
- Anything else: <or "nothing">

## Checks

- [ ] The implementer did not read `docs/clean-room/legacy-specs/`, the analysis workspace, any screenpipe binary, or post-relicense screenpipe source.
- [ ] `python3 -I tools/clean-room/verbatim_gate.py` passes on the new or rewritten prose (or each hit is listed under Exceptions with its reason).
- [ ] `bun test scripts/ci/clean-room-guard.test.ts` passes.

## Exceptions

<Anything that did not follow SOURCING.md, and what was done about it. "None" if nothing.>
