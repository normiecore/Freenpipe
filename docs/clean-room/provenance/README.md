# Provenance records

Owner decision 8 requires a provenance record for every work package started from 2026-10-29 on. The record is a dated account of where each part of the work came from. If anyone ever asks how Freenpipe was built, these records answer for each work package.

## Rules

- One file per work package: `docs/clean-room/provenance/<WP>.md`. Copy [TEMPLATE.md](TEMPLATE.md).
- The implementer writes it on the work package's branch, and it is merged with the code. CI fails a `wp/<WP>` branch that has no record (`scripts/ci/clean-room-guard.test.ts`).
- Write what happened, not what should have happened. If a rule was broken, say so and say what was done about it.
- Never put extracted material or evidence pointers in a record. Name sources by URL, base commit or spec item id.

## Work merged before the freeze

Work packages merged before 2026-10-29 were built from specs written from binary evidence, under the rules in force at the time. They have no individual records; their history is the `Merge W…` commits in the git log up to `bc03932`. W4-02 and W4-05 were opened before the freeze and are grandfathered the same way.
