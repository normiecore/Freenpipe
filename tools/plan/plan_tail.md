## 11. Risks

**Build and toolchain**
- The base only builds inside the vcvars environment, and the debug profile does not link (CRT mismatch between knf-rs-sys and whisper-rs-sys). Until W0-01 fixes it, every acceptance check uses `--profile release-dev`, and each release-dev build takes about 7 minutes.
- The Tauri app has never been built on this machine. Expect sidecar, NSIS and WebView2 surprises in W0-01.
- W0-11 (sqlx 0.7.2 to 0.9, libsqlite3-sys 0.26 to 0.37) is a breaking API migration across db, engine, secrets and app. It is the prerequisite for the 3.51.3 WAL-reset guard, `sqlite3_recover`, FTS5 `contentless_delete` and the rusqlite vtab. sqlx's out-of-order and ignore-missing behaviour must be re-verified, because target migration 20260601000000 sorts before the base's last two.
- W2-19 needs the Vulkan SDK (glslc) to compile ggml-vulkan, plus DirectML / ORT rc.12. The target's audiopipe revision (b099a2e) postdates the relicense, so implement the DML recycle and CPU retry ourselves unless its licence is cleared.
- W2-25 moves to React 19 and Next 15.5 underneath about 200 components. Expect breakage in Radix, tiptap and the test setup. Pin stable releases at least two weeks old, not the target's canary.
- Several third-party crates carry unknown or vendor-adjacent licences: the AEC3 port (sonora), the forks under github.com/screenpipe/* (whisper-rs, rusty-tesseract) and divanshu-go/* (audiopipe, tauri-helper). W0-01 must confirm that pinned revisions predate 2026-06-10, and mirror them.

**Coexistence and data safety (owner machine)**
- The installed screenpipe v2.7.104 shares port 3030, the :11435 control server, `~/.screenpipe`, `~/.screenpipe/auth.json` and the `screenpipe://` scheme. The base app's blind port-kill would terminate it, and base auth code can delete its auth.json. W0-01 and the section-2 rules must land before any app run.
- W1-01 runs an irreversible migration that drops ocr_text. Opening a real screenpipe DB with a Freenpipe build is one-way:
  - A v2.7.104 DB is already migrated, so versions match.
  - A W0-state build (89 migrations) opening a v2.7 DB fails at runtime, because ocr_text is gone.
  Neither build may touch real data during development.
- Freenpipe's migration files differ in text from the target's: upstream header comments are inside the checksummed bodies. Interop relies on the checksum-rewrite path on both sides. If a DB alternates between the two apps, each one rewrites checksums on open.

**Scale and overlap**
- Huge single items: the semantic parser crate (W2-06), workflows core (W4-01), the ACP runtime (W4-03), the audio-process detector (W4-12), the retained UIA tree (W4-13), the SOP renderer (W4-21), hybrid storage (W5-07, W6-06, W7-01, W7-02), meeting piggyback (W6-05), and the Live Views, Workflows and chat-blocks UIs. Each is XL and may need internal phasing within one agent's session.
- Hotspot files (section 6): `db.rs`, `standalone-chat.tsx`, `pipes/mod.rs`, `meeting_detector.rs`, app `pi.rs`, `connections_api.rs` and `event_driven_capture.rs`. Region discipline and the W1-03 seams are what keep the waves mergeable.

**Testing gaps**
- Windows-only behaviour needs real hardware and manual smoke tests: WASAPI process loopback (build 20348 or later), native WGC, UIA retained trees, the LL-hook watchdog, WER, PowerRegisterSuspendResumeNotification, SAPI, DirectML and Vulkan. CI cannot cover these, so manual checks are listed explicitly.

**Spec gaps (inferred behaviour)**
- Wedge-watchdog thresholds, Bluetooth-gate and identification-gate numbers, the text-region detector algorithm, compaction thresholds, and the app reaction to a stale renderer heartbeat are all inferred.
- The search-only gated route set (api-034) is unresolved.
- The retention ui_events strip is inferred-strong.
- The hybrid payload byte layout is only partially observed.
Implementers must choose sensible values and document them in code comments.

**Clean-room hazard**
- Several WPs are prompt- and prose-heavy: 21 built-in pipes, 20 starter skills, meeting summary, first-run summary, activity history, workflow agents and the video-sop skill. Reviewers must check that all of this text is newly written.

**Scope creep from vendor features**
- Do not port vendor-only features, even as local stubs, unless the parity frontend calls them: Composio, data-sync upload, team, trials, quotas, remote controls, announcements and workflow sharing.

## 12. Open questions for the owner

1. **Default data dir and coexistence.** Option A: keep `~/.screenpipe` and port 3030/11435 for drop-in interop with existing data, pipes and MCP clients, at the cost of never running beside screenpipe. Option B: default to a Freenpipe dir and port, with an "import from screenpipe" path. Which one?
2. **Deep-link scheme.** Register only `freenpipe://`, or also `screenpipe://` (and `screenpipe-enterprise:`) as an opt-in alias for external links (frontend-M01)? An alias would collide with an installed screenpipe.
3. **CLI binary name.** Keep `screenpipe` (pipes, skills and MCP configs invoke it), rename to `freenpipe`, or ship both?
4. **Hybrid Parquet storage** (db-021..024, retention-008, parts of frontend-039/044). It is opt-in in the target, and it is five XL/L WPs with real data-loss risk. Is it required for parity, or should it be deferred and the waves after W5 trimmed?
5. **SQLite toolchain jump** (W0-11). Is the move to sqlx 0.9 / libsqlite3-sys 0.37 acceptable? The alternative is to stay on 0.7 and give up the 3.51.3 guard, embedded recovery and the vtab.
6. **Git dependencies under the vendor's orgs.** Should they be mirrored under normiecore after the date and licence check? May the target's newer audiopipe revision be used if its licence is clearly MIT or Apache?
7. **Updater and releases.** Who creates the minisign key and the GitHub release secrets for normiecore/Freenpipe? Is `autoUpdate` defaulting to true (as in the target) acceptable for Freenpipe?
8. **Localization.** Ship English only with the ja catalogue scaffolded, or commission a Japanese translation?
9. **Third-party, user-initiated network features.** Are these acceptable to keep: the MCP registry search (registry.modelcontextprotocol.io), ACP agent npm installs, Cursor Cloud (api.cursor.com), the GitHub skill install and the Bee pairing service? The Bee pairing app id would have to be supplied by the user.
10. **Default narration provider** for SOP videos. Captions-only (silent) unless the user configures TTS, or local Windows SAPI?
11. **React version.** Follow the target to React 19 now (W2-25), or stay on React 18 until the feature UIs land?
12. **Built-in pipe and skill content.** 21 pipes and 20 starter skills need fresh prose. Should all of them be written for parity, or a curated subset?
13. **CI.** Does normiecore/Freenpipe have Windows GitHub Actions runners for the build, test and installer pipeline that W0-03 produces?

## 13. Regenerating this plan

`python3 tools/plan/plan_build.py` validates the following and rewrites this file and `docs/clean-room/plan_wps.json`:
- every live item is in exactly one WP;
- every dropped item is mapped in section 9;
- dependencies point to known WPs in the same or an earlier wave.

`python3 tools/plan/plan_build.py --check` only validates and fails when either output is out of date. Work-package status (merged, in review, claimed, ready, blocked) is not part of this plan; `python3 tools/plan/status.py` derives it from git and GitHub and the merger publishes it as `docs/clean-room/STATUS.md`.
