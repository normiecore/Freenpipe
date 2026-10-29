# Clean-room verbatim gate

Handvel's earlier specs were written from the shipped screenpipe v2.7.104 binary and implemented in
new words. Binary analysis stopped on 2026-10-29 (owner decision 8, `docs/clean-room/SOURCING.md`),
but text written before then, and text written since, can still echo that binary by accident. This directory holds the tripwire that catches text copied from that binary.

| File | What it is |
|---|---|
| `verbatim_gate.py` | The checker. Python 3.8+, standard library only. |
| `corpus-shingles-8.bin` | The hashed index of the target corpus (about 135,000 entries, 1 MB). |

## What it checks

The gate flags any line of a checked file that shares a run of **8 consecutive plain words** with the
text extracted from the shipped screenpipe v2.7.104 Windows build: its string table, the bundled
frontend, embedded prompts and similar prose.

How a line becomes words:

1. The line is lowercased and split into tokens (letters, digits and the characters ``_ ' . / : @ # $ % & = + -``).
2. Leading and trailing `.`, `:`, `-` and `'` are stripped from each token.
3. A token made only of letters and apostrophes is a **word**. Anything else (identifiers with `_`,
   paths, routes, numbers, dotted names, URLs) is a **break**, and no run crosses it.
4. Every window of 8 consecutive words is an **8-gram**.

Lines that look like SQL statements (`CREATE`, `ALTER`, `SELECT`, `INSERT`, `PRAGMA` and so on) and
lines inside ```` ```sql ```` fences are skipped, because schema text is an interop identifier that
must match exactly.

So identifiers, routes, settings keys, JSON field names and SQL never trip the gate. Prose does: UI
copy, comments, documentation, prompts and error sentences.

## Why the corpus is hashed and never in the repository

The extracted strings and frontend of the shipped binary belong to the vendor and are licensed under
the Screenpipe Commercial License. They are reverse-engineering evidence, not material Handvel may
redistribute. Owner decision 4 (`docs/clean-room/DECISIONS.md`) keeps all raw extracted material (string
dumps, the extracted frontend, REA graph dumps) out of the repository for good.

The gate still needs to know which 8-grams occur in that corpus. So the index stores only a keyed
hash of each one:

- Each 8-gram (the 8 words joined by single spaces) is hashed with BLAKE2b, 8-byte digest, keyed with
  the constant `freenpipe-clean-room-v1`, and read as a little-endian unsigned 64-bit integer.
- `corpus-shingles-8.bin` is the 5-byte magic `FPSH1`, one byte for N (8), then the sorted, de-duplicated
  hashes as little-endian `uint64`. The gate binary-searches it.

The hash is one-way. The file holds no words, no word order beyond each 8-gram, and nothing that
rebuilds the corpus. The key is public (it is in `verbatim_gate.py`), so anyone can test whether a
**given** 8-word phrase occurs in the corpus, which is exactly what the gate does. Enumerating the
corpus would require guessing whole 8-word phrases, which is not practical. False positives from
64-bit hash collisions are negligible at this size.

## Running it

```bash
# whole files or directories (exit 1 when any line matches, 0 otherwise)
python3 -I tools/clean-room/verbatim_gate.py <file-or-dir> [...]
python3 -I tools/clean-room/verbatim_gate.py --json hits.json docs/clean-room

# only the lines a branch adds: what the CI "gates" job runs (report-only)
bash scripts/ci/verbatim-diff.sh origin/main HEAD
```

Directories are walked for `.md .txt .json .rs .ts .tsx .js .jsx .mjs .py .toml .yml .yaml .html .css
.swift` files, skipping `.git`, `node_modules`, `target`, `out`, `.next`, `dist` and `third_party`.
Each hit prints `file:line: text`.

## What to do with a hit

- **Prose** (UI copy, comments, docs, prompts, error sentences): rewrite it in your own words. This is
  the case the gate exists for.
- **An interop identifier the gate could not tell apart from prose**, for example a client-visible
  error string that clients match on: keep it, and justify it in the PR description ("exact string,
  client-visible", as the specs mark them).
- **Moved or unchanged MIT base code**: text that existed in the MIT base (`892199f`) can also be in
  the later binary. Keep it, and justify it in the PR description with the base file it came from.

Reviewers check every justification (`docs/dev/cloud-reviewers.md`, compliance brief). The gate
is a tripwire, not proof: shorter shared runs pass it, so writing in your own words still matters.

## Rebuilding the index

The index is frozen as built in commit `c71c132` (2026-10-15) and will not be rebuilt: owner
decision 8 stopped all use of the analysis workspace. Its SHA-256 is
`5721def0dbf28a392bb1fa2a7425f1caf5d6dc9a24e5bb601bd304010c8a78ca`. The builder is not published.
