# Re-sourced specifications

This folder holds only spec text written under [SOURCING.md](../SOURCING.md): interoperability identifiers redacted from the pre-freeze specs, and behaviour written from public material, the MIT base or Freenpipe's own design. Implementers read this folder and nothing else about screenpipe's later behaviour.

Each file keeps the name of its pre-freeze counterpart in `../legacy-specs/` (for example `api.md`), and each delta item keeps its id (for example `api-031`), so `PLAN.md` references stay valid. A work package can be scheduled once every item it lists is here with a source tag.

`scripts/ci/clean-room-guard.test.ts` scans this folder strictly: no evidence pointers, no hex offsets, and every delta item must carry a source tag.
