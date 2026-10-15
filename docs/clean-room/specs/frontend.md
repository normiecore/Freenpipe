# Freenpipe spec — Desktop UI (pages, settings, windows, Tauri commands)

Dimension key: `frontend`. Target: installed screenpipe **v2.7.104** (Windows, Tauri 2.11). Base: Freenpipe @ upstream `892199f` (`apps/screenpipe-app-tauri`).

Clean-room note: everything below is a paraphrase of observed behaviour. Interop identifiers (route paths, Tauri command and argument names, event names, settings keys, localStorage keys, JSON field names, file/dir names, test ids) are quoted exactly. UI copy is summarised, never transcribed; no JS bodies are reproduced.

## 0. Evidence conventions

| Tag | Meaning |
|---|---|
| `str@0xNNNN` | hex file offset of a line in `<re-workspace>/app_strings.tsv` |
| `fe:<path>` | file under `<re-workspace>/frontend/` (Next.js static export extracted from the exe) |
| `fe@<chunk>:<char offset>` | character offset inside a minified chunk (found with `tools\ctx.py` / `tools\slice.py`) |
| REA | `ev_57f23857f31e9207a9f71e1d612412e78f9c5dc0d29af8c68f97372786b6216d` = analyze_javascript_application over `frontend/` (parent). `ev_d205fc1fda693bff953c410e8912bcbd3a6e1ff1e195914f6446ed80da215f84` = summary view (2112 modules, 394 parsed JS files, partial semantic coverage). `ev_b3dc1ba68f99f4bdad91cd2958e6b08c8bb349390b55a76643217add23ddb66c` = module page view. `ev_3e6569e0f9782a4eb4e8a7b233eb9b785413a9bd0652156c82ef343bab6d6d92` = module trace seeded on `starred-sessions` (completed; too large for transport). `ev_ed178f1b60f7fc0f59c9e2ea51190f0f827c24217d71c2818b1662183129cc97`, `ev_ac3d9dbfc93a6f8a3af1f2389dd669fa5a22c2a456d424d92be94ca3d6bff3ad` = string/route seeds with no graph match (the REA JS graph does not index literal command/route strings in these minified chunks, so most findings below come from direct static reading of the chunks). |

Analyst tools written for this pass (all in `<re-workspace>/tools/`): `tauri_cmds.py` (binding diff), `fe_scan.py` (events/invokes/localStorage), `ui_outline.py` + `ol_filter.py` (per-chunk UI outline), `settings_keys.py` (settings-key → label map), `base_has.py` (base presence check), `sgrep.py`, `ctx.py`, `slice.py`, `libsig.py`, `srcmods.py`.

Key chunk map (target):

| Chunk | Contents |
|---|---|
| `fe:_next/static/chunks/8001-00ed120106af2449.js` | tauri-specta `commands` bindings (244 methods / 310 command literals) |
| `fe:_next/static/chunks/app/layout-3ec03e167c097e07.js` | root layout: providers, deep links, storage-migration overlay, DB recovery, announcements, browser-pairing prompt, pipe-install review, renderer heartbeat, browser-dev Tauri mock |
| `fe:_next/static/chunks/app/(main)/layout-fc8e211a4c4ce103.js` | shared shell for `/home` + `/settings` (resizable sidebar, first-run summary, calendar nudge, trial/card-ask surfaces) |
| `fe:_next/static/chunks/app/(main)/home/page-c18385b042b3382e.js` | Home: nav, chat sidebar, Library/Live Views, Activity, Meetings, Help, command palette, recording status, first-run guide |
| `fe:_next/static/chunks/app/(main)/settings/page-d9c69007346aaf6a.js` | Settings v2 (all sections) |
| `3696-…` chat engine/UI, `5977-…` connections + automations + AI preset picker, `1491-…` timeline, `4656-…`/`755-…`/`3768-…`/`9783-…` Workflows workspace, `7451-…` ACP presets, keybindings, settings zod schema, `9665-…` AI-tool connect + skill catalog, `961-…` enterprise, `378-…` settings store defaults/auth/sidebar layout, `7737-…` feedback, `3308-…` search UI, `9092-…` entitlement gate, `9668-…` goal categories + settings section list, `6139-…` i18n runtime + `ja` dictionary, `3188-…` General Translation runtime, `3822-…` feedback module, `9089-…` typed event channels, `1a258343…` @xyflow/react, `7185…` mermaid loader | |

---

## 1. UI stack (base → target)

| Item | Base (package.json) | Target (observed) | Evidence |
|---|---|---|---|
| Next.js | 15.1.4 | 15.5.21 (static export, app router + `(main)` route group) | `"15.5.21"` in `fe:_next/static/chunks/main-439ea742d4c438c2.js` |
| React (app router) | 18.3.1 | 19.2.0-canary-0bdb9206-20250818 (Next-vendored); pages-router framework chunk still 18.3.1 | `fe:_next/static/chunks/4bd1b696-9a184e74945ebe81.js`, `framework-b9fd9bcc3ecde907.js` |
| Tailwind | 3.4.x | 3.x (v3 `--tw-*` preflight variables; CSS modules for Workflows) | `fe:_next/static/css/f2f100b14d1afc16.css` |
| UI kit | shadcn/ui on Radix + cmdk 0.2.1 | same family (Radix `data-radix`, cmdk used by command palette/model picker) | libsig: radix 148 hits/21 files, cmdk in `5436`, `7451` |
| Fonts | Inter (next/font) + JetBrains Mono stack for sans/mono | Inter + **Space Grotesk** (Workflows headings via `--heading-font` / `--font-workflow-heading`) + JetBrains Mono stack | `fe:_next/static/css/4fcea525c8c4ca77.css`, `bb23ca13e30faefa.css` |
| Mermaid | 11.12.3 via dynamic `import("mermaid")` | 11.12.3 shipped as a static asset `static/media/mermaid.min.86edda15.js` and script-loaded | `fe:_next/static/media/mermaid.min.86edda15.js` (`version:"11.12.3"`), loader `7185.7d0e78cc3fef1f46.js` |
| i18n | none | **General Translation** runtime (`gt-next`/`gt-react` style `<T _hash>` + `useGT` with `$_hash`), bundled dictionaries, locales `en` (default) + `ja` | `3188-…` (`generaltranslation.locale`, `generaltranslation.enable-i18n`), `6139-…` |
| Flow canvas | none | **@xyflow/react** (React Flow) for Live View canvas/process map | `1a258343.0855f999d33dd4e5.js` |
| Rich text | tiptap 3.22 | tiptap/ProseMirror (meeting note editor, slash menu, format toolbar) | `54a60aa6-…`, `ed038253-…` |
| Drag & drop | @dnd-kit 6/10 | @dnd-kit (sidebar nav reorder, Live View block editor, workflow steps) | home chunk sidebar `V()` sensors |
| Other kept | framer-motion, fuse.js (settings search), nuqs, react-markdown + remark-gfm, react-syntax-highlighter (refractor), react-day-picker, xlsx, mammoth, pdf extraction, posthog-js, Sentry | same | libsig output |
| Telemetry | posthog-js + Sentry in deps | posthog (feature flags drive rollouts: `desktop-localization`, trial experiments, ACP agent flags) + Sentry | `9da6db1e-…`, `57-…` |

Freenpipe guidance: upgrading to Next 15.5/React 19 is optional; none of the delta features requires React 19. Keep Inter + JetBrains Mono; add Space Grotesk (OFL) only if the Workflows workspace is built. Keep mermaid dynamic import (bundled) — the static-asset loader is a packaging detail. PostHog/Sentry must stay disabled (CLAUDE.md "No phoning home"); every flag-gated rollout below must become a local default or plain setting.

---

## 2. Base vs target inventory

### 2.1 Routes / windows

| Route (html) | Window / label | Base | Target | Status |
|---|---|---|---|---|
| `/` (index) | Main/boot redirect | yes | yes | base-present |
| `/home` | Home (`ShowRewindWindow` `{Home:{page}}`) | `app/home/page.tsx` | `app/(main)/home` inside shared `(main)` layout | **changed** (frontend-002, -006…) |
| `/settings` | Home window, settings view | `app/settings/page.tsx` | `app/(main)/settings`, same shell as home, `?section=` + `?from=` | **changed** (frontend-035) |
| `/chat` | `Chat` window | yes (thin) | yes (thin wrapper around shared chat) | base-present |
| `/search` | `{Search:{query}}` | yes | yes, new tabs/keys + `timelineOrigin` | **changed** (frontend-027) |
| `/overlay` | `Main` (timeline overlay) | yes | yes | changed (timeline items) |
| `/viewer` | viewer window | yes | yes + sandboxed HTML preview | **changed** (frontend-028) |
| `/onboarding` | `Onboarding` | 5 slides | 8 steps | **changed** (frontend-034) |
| `/notification-panel` | `notification-panel` | yes | yes + feedback/open-source | **changed** (frontend-031) |
| `/notification-inbox` | `notification-inbox` | — (popover only) | **new window** | **new** (frontend-030) |
| `/shortcut-reminder` | `shortcut-reminder` | yes | v2 (health incidents, live transcript, anchors) | **changed** (frontend-032) |
| `/starred-sessions` | `starred-sessions` | — | **new window** | **new** (frontend-029) |
| `/permission-recovery` | `PermissionRecovery` | yes | yes (+secure storage row, restart flow) | **changed** (frontend-033) |
| `/dev/workflows-ux` | dev-only preview harness | — | present (fictional-data UX preview) | new (dev; skip) |

`showWindow`/`showWindowActivated` targets observed: `"Main"`, `"Chat"`, `"Onboarding"`, `"PermissionRecovery"`, `{Search:{query:null}}`, `{Home:{page:X}}` with X ∈ `home, brain, activity, meetings, pipes, account, display, recording, null`.

### 2.2 Home sections

| Section id | Base | Target label | Status |
|---|---|---|---|
| `home` | Chat home | "Chat" | changed (tabs, ACP, rich blocks) |
| `timeline` | yes | "Timeline" | changed |
| `activity` | — | "Activity" | **new** (frontend-023) |
| `memories` | yes | folded into `brain` | **removed** (alias) |
| `brain` | — | "Library" (Live Views / Memories / Artifacts) | **new** (frontend-021/022) — hidden by default in nav |
| `pipes` | yes | "Automations" | changed (frontend-049) |
| `connections` | yes | "Connections" | changed (frontend-050) |
| `meetings` | yes | "Meetings" | changed (frontend-024) |
| `help` | yes | (bottom nav "Help & feedback") | changed |
| `history` | yes | chat history list | base-present |
| `workflows` | — | separate **Workflows workspace** reachable from a workspace switcher | **new** (frontend-047) |

### 2.3 Settings sections

| Base id → label | Target id → label (group) | Status |
|---|---|---|
| `display` Display | `display` "Appearance" (App) | changed |
| `general` General | `general` (App) | changed |
| `ai` AI models | `ai` "Models & keys" (AI) | changed |
| — | `ai-settings` "AI features" (AI) | **new** |
| — | `activities` "Activities" (AI) | **new** |
| `recording` Recording | `recording` "Screen" (Capture & data) | changed (split) |
| — | `audio` "Audio & meetings" (Capture & data) — renders the recording component with `section:"audio"` | **new** |
| `shortcuts` | `shortcuts` (App) | changed |
| `notifications` | `notifications` (App) | changed |
| `usage` Usage | `usage` "AI credits" (AI) | changed (cloud) |
| `privacy` | `privacy` (Capture & data) | changed |
| — | `permissions` "Permissions" (macOS only) | **new** |
| `storage` | `storage` "Disk & retention" | changed |
| `speakers` | `speakers` | base-present (minor) |
| `team` | `team` | changed (cloud) |
| `account` | `account` | changed (cloud) |
| `referral` "Get free month" | `referral` "Refer a friend" | changed (cloud) |

### 2.4 Onboarding steps

Base: `login → permissions → engine → connect-apps → pipe`. Target: `login → acquisition → permissions → timeline → engine → plan → recommended-setup → first-task` (conditional filtering, see frontend-034).

### 2.5 Tauri command surface

Base `lib/utils/tauri.ts`: 206 commands. Target bindings + ACL: 310 command literals (`str@0x5177a68` sorted ACL list; registration arms `str@0x4993348`, `str@0x4993580`, `str@0x4993f18`). **112 new, 8 removed, 7 argument changes** — full table in Appendix A.

### 2.6 Events

Appendix B lists every listened/emitted event with base/new status (≈25 new Tauri events + typed channels `job:event`, `export:event`, `engine:event`, `notification:action`).

### 2.7 Settings keys

Appendix C lists the new and changed keys of the settings store (`store.bin` → `settings`), with defaults and the UI control that binds each one.

---

## Appendix A — Tauri commands invoked from JS

Signature notation: `name(argA, argB) → shape` where shape is paraphrased from usage or the target's browser-dev mock (`fe@app/layout-3ec03e167c097e07.js:34200-57800`). `R` = wrapped in the tauri-specta `{status:"ok"|"error"}` Result; `P` = plain promise.

### A.1 New commands (112)

Shell / windows / overlay
| Command | Args | Kind | Used by | Behaviour |
|---|---|---|---|---|
| `show_notification_inbox` | — | R | notification bell | open/focus the `notification-inbox` window (frontend-030) |
| `toggle_starred_sessions` | — | R | shortcut-reminder | show/hide `starred-sessions` window (frontend-029) |
| `hide_starred_sessions` | — | R | starred-sessions | hide own window |
| `snooze_shortcut_reminder_for_hour` | — | R | shortcut-reminder context menu | hide the shortcut bar for 1 h; persists `shortcutOverlaySnoozedUntil` |
| `set_shortcut_overlay_anchor` | `anchor, display` | R | shortcut-reminder drag | persist `shortcutOverlayAnchor` (`top-center`/`bottom-center`/`left-center`/`right-center`) + `shortcutOverlayDisplay` and reposition |
| `get_recording_health_state` | — | P → string `state|reason|component|recovery` | shortcut-reminder | see frontend-032 |
| `overlay_restart_recording` | — | R | shortcut-reminder | user-requested capture restart from an incident |
| `overlay_dismiss_incident` | — | R | shortcut-reminder | dismiss current incident |
| `is_capture_paused` | — | P → bool | home, root layout | global pause state for recording-status UI |
| `set_chat_always_on_top` | `onTop` | R | settings Appearance | apply `chatAlwaysOnTop` to the Chat window |
| `set_app_screen_capture_protection` | `hidden` | R | settings Appearance | exclude overlay window from screen capture (`hideOverlayInScreenRecording`) |
| `get_app_screen_capture_protection` | — | P → bool | settings | read current state |
| `set_keep_awake` | `enabled` | R | settings Screen → Power | prevent idle sleep (`keepComputerAwake`) |
| `set_history_swipe_navigation_enabled` | `enabled` | R | root layout (always `false`) | disable two-finger back/forward swipe in webviews |
| `webview_renderer_heartbeat` | — | P | root layout every 1 s | renderer liveness watchdog |
| `restart_for_update` | `timeoutSecs` | R | update banner | safe-restart into a downloaded update (replaces direct `await_safe_restart` + relaunch path) |
| `retry_screenpipe` | — | R | root layout (`request-server-restart`), permission recovery | restart the embedded engine on request |
| `restart_after_screen_recording_permission` | — | P | onboarding/permission recovery/overlay | relaunch app after macOS screen-recording grant |
| `check_accessibility_permission_live_cmd` | — | P | onboarding, settings Permissions | non-cached accessibility check |
| `activate_app_after_oauth` | — | P | onboarding / connections | bring app to front after browser OAuth |
| `native_timeline_show` | `port, apiKey, embedded` | P | timeline | macOS native timeline host (frontend-026) |
| `native_timeline_hide` / `native_timeline_close` | — | P | timeline | — |
| `native_timeline_navigate` | `timestamp, frameId` | P | timeline | — |
| `native_timeline_is_available` | — | P → bool | timeline | false on Windows |
| `copy_rich_text_to_clipboard` | `html, text` | R | meeting share | put HTML + plain text on clipboard |
| `read_clipboard_text` | — | R | connections (paste token) | read clipboard text |
| `read_log_tail` | `path, maxBytes` | R | feedback | tail a log file |
| `get_low_disk_guard_config` | — | P → `{thresholdBytes}` | home recording status, settings storage | low-disk guard threshold |
| `get_active_data_dir` | — | R → string | many | resolved data dir (honours fallback) |
| `get_screenpipe_base_dir` | — | R → string | settings store, workflows | `~/.screenpipe` |
| `get_chats_dir` | — | R → string | chat storage | `~/.screenpipe/chats` |
| `list_chat_entries_by_mtime` | `dir` | R → entries | chat storage | fast directory listing sorted by mtime |
| `is_telemetry_disabled_by_env` | — | P → bool | analytics init | honour an env opt-out |
| `get_onboarding_attribution` | — | P → `{utmSource,utmMedium,utmCampaign,utmContent,utmTerm}`/null | onboarding acquisition | website attribution (decloud) |

Storage & recovery
| Command | Args | Kind | Behaviour |
|---|---|---|---|
| `get_storage_migration_status` | — | R → `{app_session_id, root, busy, message, error, pending, in_place, bytes_saved, available_bytes, completed, using_new_storage, generation, source_bytes, migrated_bytes, can_migrate, can_cancel, can_delete_source, blocked_reason}` | frontend-039/044 |
| `get_storage_migration_activity` | — | P → `{root, busy, message, error, elapsed_seconds, completed_records, total_records, bytes_saved, available_bytes, completed}` | progress (also pushed via event `storage-migration-activity`) |
| `start_storage_migration` | `root` | R | start in-place conversion |
| `cancel_storage_migration` | `root` | R | stop and use original DB |
| `delete_original_storage_database` | `root, generation, confirmPermanentDeletion` | R → bytes freed | only when `can_delete_source` |
| `start_database_recovery` | — | R | deep link `screenpipe://database-recovery` |
| `restart_database_verification` | — | R | deep link `screenpipe://database-restart-verification` |

Activity
| `get_activity_history` | `start, end` | R → `{entries:[{id, kind:"work", meeting_id, start_at, end_at, title, summary, evidence:[{kind:"screen"|…, at, frame_id, meeting_id, app_name, label}]}], coverage:[{start,end}]}` | frontend-023 |
|---|---|---|---|
| `generate_activity_history` | `start, end, idempotencyKey` | R → `{entries}`; error text `activity_no_data:<reason>` with reason ∈ `not_recording, no_capture_in_range, empty_but_recording` | frontend-023 |
| `set_activity_sync_enabled` | (enterprise) | — | ee-stub |

Library / Live Views (brain views)
| `list_brain_views` | — | R → `BrainView[]` | frontend-022 |
|---|---|---|---|
| `save_brain_view` | `request:{id,title,expectedRevision,timeRange,periodPolicy,slots}` | R → BrainView (revision+1) | optimistic concurrency on `expectedRevision` |
| `delete_brain_view` | `id` | R | |
| `load_brain_view_canvas` | `viewId` | R → Canvas/null | |
| `save_brain_view_canvas` | `request:{…canvas, expectedRevision}` | R → Canvas | |
| `list_brain_view_template_kits` | — | R → TemplateKit[] | |
| `install_brain_view_template_kit` | `request` | R | installs a kit's pipes + view |

Workflows workspace
| `analyze_workflows` | `days, profile` | R | frontend-047 |
|---|---|---|---|
| `generate_workflow_skill` | `workflow, profile` | R | |
| `save_workflow_skill` | `draft` | R | |
| `load_workflow_recording` | `timestamp, appName` | R | local clip for a captured moment |
| `release_workflow_recording` | — | P | free clip resources |
| `open_workflow_captured_moment` | `frameId, timestamp` | R | open timeline at moment |
| `create_workflow_video` | `id, scenes` | R | render narrated SOP video (progress events `workflow-video-<id>`) |
| `cancel_workflow_video` / `discard_workflow_video` | `id` | R | |
| `ensure_workflows_runtime` | — | R | install/start the workflows agent runtime |
| `get_workflows_runtime` | — | P | runtime status |
| `redact_workflow_contribution` | `text` | R | PII-redact text before sharing (decloud consumer) |

Meetings
| `start_export_recording` | `meetingId, start, end, outputPath` | R → `{jobId}`; completion via `export:event` | frontend-024 |

Chat / agents
| Command | Args | Behaviour |
|---|---|---|
| `coding_workspace_create` | `conversationId, repositoryPath` | create isolated git worktree for a chat (frontend-017) |
| `coding_workspace_get` | `conversationId` | |
| `coding_workspace_prepare` | `conversationId, prompt, startingPath` | resolve repo from prompt/path |
| `pi_start_and_prompt` | `sessionId, projectDir, userToken, providerConfig, message` | start agent + first prompt atomically |
| `pi_request_state` | `sessionId` | refresh session state |
| `pi_stop_if_idle` | `sessionId` | |
| `pi_get_thinking_level` / `pi_set_thinking_level` | — / `sessionId, level` | reasoning effort |
| `pi_extension_ui_response` | `sessionId, requestId, response` | answer an extension UI request |
| `pi_list_extension_packages` / `pi_install_extension_package` / `pi_remove_extension_package` | — / `source` / `source` | Pi extension packages → `[{source, scope:"user", filtered, installed, acpCompatible}]` |
| `pi_acp_probe_agent` | `agent` | read models/modes of an ACP agent command |
| `pi_acp_agent_install` / `pi_acp_agent_install_status` / `pi_acp_agent_download_pending` | `agentId` | download/install catalog agents |
| `pi_acp_external_login` | `agentId` | run the agent's own login |
| `pi_acp_reauthenticate` | `sessionId` | |
| `pi_acp_set_mode` | `sessionId, modeId` | |
| `pi_acp_set_config_option` | `sessionId, optionId, value, isBoolean` | |
| `pi_acp_set_approval_mode` | `sessionId, approvalMode` | permission mode |
| `owned_browser_history` | `tabId, direction` | back/forward |
| `owned_browser_tab_navigate` | `tabId, url, owner` | multi-tab owned browser |
| `owned_browser_tab_set_bounds` | `tabId, parent, x, y, width, height` | |
| `owned_browser_tab_hide` / `owned_browser_tab_close` / `owned_browser_tab_clear_browsing_data` | `tabId` | |

Skills / integrations
| `fetch_skills_registry` | — | R → `[{name, description, repo, git_ref, path, source, featured?, imported}]` (bundled catalog + remote refresh, `str@0x4995d13`) |
|---|---|---|
| `install_registry_skill` | `repo, gitRef, path, name` | download a SKILL.md folder from GitHub into the skills store → `{name, description, path}` |
| `install_external_agent_skills` / `remove_external_agent_skills` | `target` | write/remove screenpipe skill files into an external agent's config (claude, codex, cursor, gemini, openclaw, hermes) |
| `set_ai_tool_auto_connect_opt_out` | `target, optOut` | |
| `resolve_ai_tool_config_path` | `path` | expand `~`/env in agent config paths |
| `grokbot_connection` | `action` (`connect`/`disconnect`/status) | → `{detected, connected, optedOut}` |
| `list_provider_automations` | — | → `[{key, provider, nativeId, name, schedule, scheduleLabel, status:"active"|"paused", executionScope:"local"|"session", manageability:"in_app"|"read_only", availableActions:["pause"|"resume"|"delete"], lifecycleNote, revision, updatedAtMs}]` |
| `manage_provider_automation` | `key, action` | pause/resume/delete an external agent's scheduled task |
| `remote_sync_exec_setup` | `config, target` | remote-sync helper setup |

Feedback / diagnostics
| `start_feedback_upload` | `request` | R; completion event `feedback-upload-completed` (decloud, frontend-046) |
|---|---|---|
| `redact_pii_for_feedback` | `text, settingsJson` | redact before attaching |

Transcription
| `test_openai_compatible_transcription` | `endpoint, apiKey, model, headers, rawAudio` | send a silent sample; must pass before the engine can be enabled |
|---|---|---|

Enterprise / vendor-cloud (stub)
`apply_enterprise_ui_visibility`, `get_enterprise_host_identity` (→ `{machine_id_hash, os_user_id_hash}`), `set_enterprise_recording_authorized(authorized, credentialType, credential)`, `is_history_access_restricted`, `list_managed_team_skills`, `sync_managed_team_skills(skills, pruneUnlisted)`, `get_screenpipe_ai_gateway_url`, `set_activity_sync_enabled` — see frontend-052.

### A.2 Removed (8)
`e2e_emit_agent_stream`, `e2e_emit_pipe_stream`, `e2e_main_overlay_visible`, `e2e_owned_browser_visible`, `e2e_shortcut_reminder_visible`, `get_e2e_seed_flags` (test hooks), `pipe_suggestions_get_settings`, `pipe_suggestions_update_settings` (pipe-suggestion scheduler UI gone). Freenpipe may keep its own e2e hooks.

### A.3 Argument changes (7)
| Command | Base args | Target args |
|---|---|---|
| `livetext_highlight` | `terms` | `terms, frameId` |
| `oauth_connect` | `integrationId, instance` | + `variant` |
| `open_login_window` | — | `freshSession, authMode` (decloud) |
| `open_search_window` | `query` | + `timelineOrigin` |
| `owned_browser_navigate` | `url` | + `owner, reveal` |
| `save_enterprise_team_config` | `isAdmin, licenseActive, teamApiToken` | + `gatewayUrl` |
| `search_navigate_to_timeline` | `timestamp, frameId, searchTerms, searchResultsJson, searchQuery` | + `timelineOrigin` |

### A.4 Unchanged (198)
All other base commands are still invoked with the same names/args (list in `tools\tauri_cmds.py` output; e.g. `show_window`, `pi_*` core, `livetext_*`, `oauth_*`, `ics_calendar_*`, `remote_sync_*`, `chatgpt_oauth_*`, `vault_*`, `update_global_shortcuts(showShortcut,startShortcut,stopShortcut,startAudioShortcut,stopAudioShortcut,profileShortcuts)`, `get_disk_usage(forceRefresh,dataDir)`).

---

## Appendix B — Events

Tauri events (Rust ⇄ JS). "Rust" = name present in the exe strings, so emitted/handled natively.

| Event | Dir | Base | Payload / meaning | Evidence |
|---|---|---|---|---|
| `starred-session-state` | JS emit | new | active session end ISO or null; drives overlay star indicator | starred-sessions chunk; `str@0x524e740` |
| `starred-sessions-changed` | Rust+JS | new | list changed (also a DOM `window` event) | `str@0x524e740` |
| `starred-sessions-visibility` | Rust→JS | new | window shown/hidden | `str@0x524e740` |
| `starred-session-toggle` | Rust→JS | new | star shortcut pressed | `str@0x524e740` |
| `recording-health-state` | Rust→JS | new | `state|reason|component|recovery` | `str@0x5a0c5e2` |
| `tray-recording-state-changed` | Rust→JS | new | tray start/stop routed to UI | `str@0x4994e80` |
| `storage-migration-activity` | Rust→JS | new | migration progress object (A.1) | root layout |
| `activity-history-updated` | Rust→JS | new | refetch Activity | home chunk |
| `audio-device-status-changed` | Rust→JS | new | refresh recording-status popover | home chunk |
| `native-shortcut-toggle-meeting` | Rust→JS | base-only listen → still used | meeting toggle from native shortcut | home chunk |
| `first-run-summary-state` | Rust→JS | new | first-run summary phase | `(main)` layout, chat |
| `first-run-guide-pending` | JS | new | start first-run guide | home |
| `feedback-upload-completed` | Rust→JS | new | feedback upload result | `7737-…` |
| `workflow-review-ready` | Rust→JS | new | a workflow review is ready (opens Workflows) | home |
| `workflow-video-<id>` | Rust→JS | new | video render progress | `4656-…` |
| `remote-support-log-status` | Rust→JS | new | remote log collection status (decloud) | settings |
| `native-timeline-attached` / `native-timeline-attach` / `native-timeline-detach` | both | new | macOS native timeline | `1491-…` |
| `search-hidden`, `search-reset` | Rust→JS | new | search window lifecycle | search page |
| `timeline-open-search`, `timeline-open-chat`, `timeline-toggle-sidebar`, `timeline-return-to-activity`, `timeline-open-daily-summary`, `timeline-ask-ai-selection`, `timeline-delete-range`, `timeline-export-video-selection` | →timeline | new | timeline commands (from native timeline/menus) | `1491-…` |
| `owned-browser:navigate`, `owned-browser:state`, `owned-browser:session-access-request`, `owned-browser:v20-cookie-blocked` | Rust→JS | new | multi-tab browser state, cookie access prompts, Chrome v20 app-bound cookie block | `3696-…` |
| `chat-sidebar-group-changed`, `chat-preset-restore` | JS⇄JS | new | chat groups / restore preset in another window | home, chat |
| `pipe-config-updated` | JS⇄JS | new | pipe config changed | `378-…`, settings, overlay |
| `store://change` | JS (browser-dev mock) | new | mock store change | root layout |
| `job:event`, `export:event`, `engine:event`, `notification:action` | Rust→JS | new | typed channels; `export:event` payload `{kind:"progress"|"completed"|"failed", jobId, label, progress, outputPath, request:{meetingId,start,end,outputPath}, summary, error}`; `notification:action` `{actionType, rawJson, payload}` | `9089-…` module 77177; `str@0x5179541` |
| `agent_event`, `agent_terminated`, `agent_session_evicted` | Rust→JS | base | unchanged | |
| Removed listeners | — | base | `chat-current-session` (now emit-only), `data-dir-fallback` (now via hook in settings), `open-pipe-suggestions`, `pi_terminated`, `update-now-clicked`, `navigate` (still emitted), `tray-upgrade` (now in settings) | |

All base events still used: `chat-*` (conversation-saved, deleted, load-conversation, ping, prefill, prefill-claim, ready, renamed, visibility-changed, focus-input), `cli-login`, `deep-link-received`, `escape-pressed`, `install-pipe`, `meeting-*` streaming events, `native-magnify`, `native-scroll`, `native-notification-action|shown`, `navigate-to-frame|timestamp`, `notification-panel-update`, `notification-requested`, `open-meeting-note`, `owned-browser:ready`, `permission-lost`, `permission_needed`, `pi-queue-changed`, `pi-reauth`, `pi_log`, `request-server-restart`, `screenpipe-auth-signout`, `search-navigate-to-timestamp`, `shortcut-reminder-update`, `shortcut-start|stop-recording|audio`, `sidecar_log`, `update-auth-required`, `update-available`, `watch_pipe`, `window-focused`, `google-calendar-auth-result`, `onboarding-read-complete`.

DOM CustomEvents (window-local, not Tauri): `starred-sessions-changed`, `sidebar-pipes-collapsed-changed`, `switch-pipes-tab`, `try-in-chat`, `connections-updated` (base).

---

## Appendix C — Settings store keys (JS defaults, `fe@378-9f286ebe7c0f0713.js:15705`; serde field order `str@0x52c8878`)

New keys (absent from base `use-settings.tsx` / `SettingsStore`) with target default and binding:

| Key | Default | Bound control |
|---|---|---|
| `uiLocale` | `"system"` | Language selector (onboarding, settings General) |
| `uiLocalizationEnabled` | `false` (PostHog flag `desktop-localization` flips it) | hidden rollout flag → make a visible toggle |
| `dataSyncEnabled` | `false` | Account → Data sync (cloud; decloud) |
| `activitiesEnabled` | `false` | Settings Activities toggle; auto-set true when the user generates activities |
| `activitiesIntervalMinutes` | `15` | Activities interval select: 5/15/30/60 |
| `userGoalCategory` | goal constant (`work_memory` family) | General → "Your goal" select (`work_memory`, `meeting_follow_through`, `work_patterns`, `process_automation`, `custom`, or none) |
| `acquisitionSource` | unset | onboarding acquisition step (analytics only) |
| `audioCaptureMode` | `"meetings-only"` | Audio → mode: `always` / `meetings-only` / `disabled` |
| `includedUrls` | `[]` | Privacy → Websites allowlist mode (objects `{domain, includeSubdomains, excludedSubdomains}`) |
| `remoteLogCollectionEnabled` | `true` | Privacy → Support access (decloud: default false) |
| `remoteLogCollectionUserId` | `null` | — |
| `user.enterprise_account` | `null` | — |
| `starSessionShortcut` | mac `Control+Super+B`, Windows `Alt+Shift+B` | Shortcuts → star-session row |
| `disableScreenshots` | `false` | Screen → Screenshots toggle; onboarding timeline step |
| `enableSemanticContext` | `true` | engine flag (remote-control gated) |
| `semanticContextMode` | `"memory"` | enum `memory` / `computerUse` / `both` |
| `remoteControlPreferences` | `{semanticContext,coreAudioSystemAudio,smartRecording,aecMode}` nulls | user overrides of remote policy |
| `remoteControlPolicy` | `{schemaVersion:1, boolean:{semanticContext,coreAudioSystemAudio,smartRecording:{defaultEnabled,forceDisabled}}, aecMode:{defaultValue,forceDisabled}}` | vendor remote kill-switch (decloud) |
| `hideOverlayInScreenRecording` | `false` | Appearance toggle (`set_app_screen_capture_protection`) |
| `hideAppInScreenShare` | `true` | engine/native |
| `firstRunGuideDone` | `false` | first-run guide |
| `recordingDetail` | `"auto"` | Screen → recording detail |
| `allowHidingShortcutOverlay` | `false` | shortcut overlay |
| `shortcutOverlaySnoozedUntil` | `null` | snooze 1 h |
| `shortcutOverlayAnchor`, `shortcutOverlayDisplay` | unset | overlay drag-to-anchor |
| `sidebarNavLayout` | `{order:["home","meetings","timeline","activity","brain","pipes","connections"], hidden:["brain"]}` | Home nav reorder/hide |
| `enhancedIncognitoDetection` | `false` | Privacy → private browsing "Enhance" (macOS) |
| `disableClickCapture` | `false` | Privacy → click events |
| `keepComputerAwake` | `false` | Screen → Power → Keep computer awake |
| `experimentalMeetingPiggyback` | policy default (`smartRecording.defaultEnabled`) | engine |
| `alwaysRecordBluetoothMic` | `false` | Audio → devices |
| `screenpipeAecEnabled`, `aecMode` | `false`, `"off"` | Audio → echo cancellation (`off`/`screenpipe`/`macos`/`windows`) |
| `headlessRecordOnly` | `false` | engine/headless mode (`headless` itself is base-present) |
| `keepSearchAvailableAfterQuit` | `true` | General toggle |
| `stopRecordingOnLowDisk` | (undefined → treated as `true`) | Storage → low-disk guard |
| `showChatSuggestions` | (undefined → on) | Appearance toggle |
| `hideDockIcon` | `false` | Appearance (macOS) |
| `redactAgentSessionSecrets`, `piiRedactionColumns`, `piiRedactionPseudonyms` | engine defaults | Privacy → agent logs / advanced extra places / consistent pseudonyms |
| `captureMaxWidth` | engine | Screen → recording quality (pixel cap) |
| `localRetentionMode` (changed) | `"media"`; enum now `off` / `media` / `lean` / `all` | Storage policy radio (base enum: media/all) |

Base-present keys that gain **new UI bindings** in the target (no schema change): `chatAlwaysOnTop` (now applied live via new `set_chat_always_on_top`), `prioritizeInputLatency`, `headless`, `deviceTier` (drives onboarding timeline step), `idleCaptureIntervalMs` (capture-frequency slider), `captureScroll` (scroll capture intensity), `maxSnapshotWidth`, `disableSnapshotCompaction`, `disableMeetingDetector` (auto-start meetings), `visualCheckIntervalMs`, `visualChangeThreshold`, `minCaptureIntervalMs`, `captureOnKeystroke`, `captureOnClipboard`, `extractionThreadPriority`, `pauseExtractionOnInputMs`, `asyncPiiRedaction`, `asyncImagePiiRedaction`, `piiBackend` (`local`/`tinfoil`; defaults now derived from `createDefaults(isPro)`: false/false/`"local"` when not pro), `piiRedactionLabels`, `shortcutOverlaySize`, `showShortcutOverlay`, `translucentSidebar`, `minimizeToTrayOnClose`, `uiTheme`, `enhancedAI`, `showRestartNotifications` (mirrored by notification pref `captureStalls`).

Changed defaults vs base: `usePiiRemoval` false→**true**; `autoUpdate` false→**true**; `filterMusic` false→**true**; `experimentalCoreaudioSystemAudio` false→**true**; `disableClipboardCapture`/`disableKeyboardCapture` now derived from a `isPro` argument (true for free/local); cloud-subscribed users get two default presets `chat` + `pipes` (base: one `screenpipe` preset); legacy section `cloud-archive`/`cloud-sync` removed from Storage (the binary carries a notice saying the cloud sync/archive options are gone from that settings page, `str@0x49943e0`).

Settings-export redaction: a key is scrubbed when its name contains, case-insensitively, any of these fragments — secrets: `token`, `apikey`/`api_key`, `secret`, `password`, `credential`, `auth`; identity: `email`, `user`, `account`; content: `prompt`; locations: `dir`, `path`, `url`, `host`; filter lists: `blocklist`, `ignored`, `included`. Exempt keys (never scrubbed): `port, fontSize, lastSyncedAt, onboardingStep, windowSize, windowPosition` (`fe@378-…:15705`).

Validation schema (zod, `fe@7451-47878874a9162526.js:56355`): provider enum adds `acp`; `ignoredUrls` items may be string **or** `{domain,includeSubdomains,excludedSubdomains}`; `audioCaptureMode` enum; `semanticContextMode` enum; `aecMode` enum `off|screenpipe|macos|windows`; `maxContextChars` 1 000–8 000 000; preset id regex letters/digits/space/`-`/`_`, may not end with "copy"; prompt ≥10 chars.

localStorage keys new in target: `screenpipe-ui-locale`, `screenpipe-ui-localization-enabled`, `screenpipe-announcement-preview`, `screenpipe-announcements-dismissed-v1`, `screenpipe:cloud-chat-display`, `screenpipe:group-expanded:<id>`, `screenpipe:recents-subsection-collapsed:<id>`, `screenpipe:settings:last-section`, `workflows:navigation-collapsed`, `workflows-preview-assistant`, `workflows-preview-feedback:<id>`, `workflow-guide-preview:<id>`, `workflow-skill-preview`, `recentChatSearchOrigin` (base). sessionStorage: `screenpipe:last-workflow-review-request`. Removed: `screenpipe-ui-theme`, `console_logs`, `screenpipe:upcoming-collapsed`, `screenpipe_claude_connected`.

---

## 3. Delta items

Each item: id, kind, size, confidence, evidence, base files to touch, behaviour spec, implementation notes. Base paths are relative to `apps/screenpipe-app-tauri/` unless prefixed with `crates/`. New Rust modules observed in the target app crate (from `src\…rs` panic paths, `tools\srcmods.py`) are named so the Rust side can mirror the split: `activity_history`, `brain_views`, `coding_workspace`, `commands\starred_sessions`, `commands\overlay_anchor`, `commands\native_actions`, `overlay_health`, `power_awake`, `storage_migration`, `db_recovery_notifications`, `db_relaunch`, `db_self_heal`, `disk_pressure_notifications`, `feedback_redact`, `feedback_upload`, `first_run_summary`, `localization`, `media_commands`, `update_restart`, `update_diagnostics`, `workflow_video`, `workflows_media`, `workflows_runtime`, `search_only`, `headless`, `remote_support_logs`, `data_sync`, `startup_auth`, `enterprise\*`.

---

### frontend-001 — UI stack refresh (libraries, fonts, loaders)
- Kind: changed · Size: M · Confidence: observed
- Evidence: §1 table; `fe:_next/static/chunks/main-439ea742d4c438c2.js` ("15.5.21"); `4bd1b696-…` (React 19.2 canary); `fe:_next/static/media/mermaid.min.86edda15.js`; `1a258343.0855f999d33dd4e5.js` (@xyflow); CSS `4fcea525c8c4ca77.css` (Space Grotesk).
- Base touchpoints: `package.json`, `next.config.mjs`, `app/layout.tsx` (fonts), `tailwind.config.ts`, `components/rewind/mermaid-diagram.tsx`.
- Spec: add `@xyflow/react` (Live View canvas, frontend-022) and keep `@dnd-kit` (nav reorder, Live View editor). Add Space Grotesk as an optional heading font token used only by the Workflows workspace. Next/React upgrades are optional; if upgrading, keep `output: "export"` and the static `.html`+`.txt` RSC output layout because Tauri serves the export as embedded assets.
- Notes: do **not** add PostHog feature-flag reads; replace every `posthog.getFeatureFlag(...)` gate seen in the target with a local constant or setting (listed per item).

### frontend-002 — `(main)` route group and shared Home/Settings shell
- Kind: changed · Size: M · Confidence: observed
- Evidence: `fe:_next/static/chunks/app/(main)/layout-fc8e211a4c4ce103.js` (test id `main-window-drag-surface`, "Resize sidebar"); settings page renders inside the same shell (`fe@(main)/settings/page-…:665179` uses the shared `AppSidebar` and `?from=` back navigation); `_manifest.json` routes.
- Base touchpoints: `app/home/page.tsx`, `app/settings/page.tsx`, `components/app-sidebar.tsx`, `lib/hooks/use-sidebar-width.tsx`.
- Spec:
  - Move `/home` and `/settings` under an `app/(main)/` group whose layout owns: the draggable title surface (`data-testid="main-window-drag-surface"`), the resizable left sidebar (drag handle labelled "Resize sidebar"), first-run summary state, the calendar join nudge (frontend-024), and plan/expiry notices (decloud → omit).
  - Settings back-to-app control: if `?from=` is present go back; a `from` value `home:<conversationId>` stores `<conversationId>` in localStorage `pending-chat-conversation` before navigating to `/home?section=home`; otherwise replace with `/home?section=<from>` or `/home`.
  - Section deep links: `/home?section=<id>` (ids in §2.2; `feedback`→`help`; `memories`→`brain`); `/settings?section=<id>` (aliases `disk-usage|cloud-archive|cloud-sync`→`storage`; `connections`→`/home?section=connections`).
- Notes: purely structural; URLs stay compatible.

### frontend-003 — UI localization (en + ja) with a language selector
- Kind: new · Size: L · Confidence: observed
- Evidence: `3188-59d54d99e994335e.js` (`generaltranslation.locale`, `generaltranslation.enable-i18n`); `6139-2b2ebb4a983b29b8.js` (bundled JSON: `defaultLocale:"en"`, locales `["ja"]`, coverage ja frontend 5332/5663, native 346/347, causes `insufficient_translation_credits`); `fe@6139-…:274381` provider logic; settings/onboarding test id `language-selector`; root layout `Q()` mirrors PostHog flag `desktop-localization` into `uiLocalizationEnabled`; Rust module `localization` (`str@0x51b3b60`).
- Base touchpoints: `app/layout.tsx`, `app/providers.tsx`, `lib/hooks/use-settings.tsx`, `components/settings/general-settings.tsx`, `components/onboarding/*`; Rust tray/menu strings (native side).
- Spec:
  - Settings keys `uiLocale` (default `"system"`) and `uiLocalizationEnabled` (bool). Effective locale = `en` unless localization is enabled and at least one non-default locale exists; with `"system"` resolve from `navigator.languages` + OS locale (`@tauri-apps/plugin-os` `locale()`), matching exact tag then base language, else `en`.
  - On change: set `document.documentElement.lang` and `dir` (from `Intl.Locale(...).textInfo.direction`), write localStorage `screenpipe-ui-localization-enabled` (`"true"/"false"`) and `screenpipe-ui-locale` (resolved tag) so non-React windows (overlay, reminder) can read it synchronously at boot; re-resolve on `languagechange`.
  - Language selector (`data-testid="language-selector"`): options "System default" + each locale shown by its native name (capitalised); saving failures show a retry toast.
  - Translation lookup by stable message hash with English fallback; all literals in the app wrapped by a `T`/`t()` helper.
- Notes: Freenpipe must not depend on a PostHog flag — expose `uiLocalizationEnabled` as a visible toggle (or enable by default when a translation for the system locale exists). Library choice is free (e.g. a small in-house `t()` with JSON catalogs); General Translation's cloud runtime must not be used (`runtimeUrl:null` in target too). Japanese strings themselves are vendor content — do not copy them; produce your own translations.

### frontend-004 — Browser-dev runtime (optional tooling)
- Kind: new · Size: M · Confidence: observed
- Evidence: `fe@app/layout-3ec03e167c097e07.js:34200-57800`: when `NEXT_PUBLIC_SCREENPIPE_WEB_DEV` is `mock` or `live`, installs fake `__TAURI_OS_PLUGIN_INTERNALS__`, an in-memory `plugin:store`, `plugin:fs` fixtures, and a `invoke` switch returning canned data for ~60 commands; env `NEXT_PUBLIC_SCREENPIPE_WEB_API_PORT` (default 3030), `NEXT_PUBLIC_SCREENPIPE_WEB_SCENARIO` (`ready`/`empty`/`backend-error`), `NEXT_PUBLIC_SCREENPIPE_WEB_API_KEY`; query `mockLocale`, `mockLocalization`.
- Base touchpoints: `app/layout.tsx` (dev-only import), new `lib/dev/browser-runtime.ts`.
- Spec: dev-only shim so `next dev` renders all pages in a normal browser; `live` mode forwards HTTP calls to a real engine on the given port. Never included in production bundles.
- Notes: useful for Freenpipe UI work; shapes in Appendix A were partly taken from these fixtures.

### frontend-005 — Webview hygiene: renderer heartbeat, context menu, swipe navigation
- Kind: new · Size: S · Confidence: observed
- Evidence: `fe@app/layout-…:140350` (`webviewRendererHeartbeat` every 1000 ms with in-flight guard; global `contextmenu` preventDefault; `setHistorySwipeNavigationEnabled(false)` on mount; wrapper making `__TAURI_EVENT_PLUGIN_INTERNALS__.unregisterListener` exception-safe; `ensureWebviewFocus`).
- Base touchpoints: `app/layout.tsx`, `src-tauri/src/commands.rs`, `src-tauri/src/window/*`.
- Spec: every window calls `webview_renderer_heartbeat` once per second (skip if previous call pending); Rust side records last-beat time per webview and may reload/recreate a renderer silent for N seconds (threshold not observed; choose ≥10 s). Disable the browser context menu app-wide. Call `set_history_swipe_navigation_enabled(false)` at boot (macOS WKWebView `allowsBackForwardNavigationGestures`; no-op on Windows).
- Notes: harmless on Windows; implement the command as a no-op on non-mac for swipe.

### frontend-006 — Home navigation v2 (sections, reorder/hide, badges)
- Kind: changed · Size: M · Confidence: observed
- Evidence: home `fe@…home/page-…:830841` (`tg` map: home "Chat", meetings, timeline, activity, brain "Library", pipes "Automations", connections); sidebar layout module `fe@378-…:46817` (`order`, `hidden`, migration); context/dropdown per nav item (`nav-<id>-options`), `sidebar-options`, `sidebar-show-<id>`, `sidebar-options-reset`, `sidebar-reset`; bottom nav test ids `nav-help`, `nav-feedback`, `nav-team`, `nav-settings`.
- Base touchpoints: `app/home/page.tsx` (`MainSection` union), `components/app-sidebar.tsx`, `lib/hooks/use-settings.tsx`.
- Spec:
  - Sections: `home, meetings, timeline, activity, brain, pipes, connections` (+ non-nav `help`, `history`). `memories` maps to `brain`. Hide `timeline` when `disableTimeline`; hide sections disabled by enterprise policy.
  - Setting `sidebarNavLayout = {order: string[], hidden: string[]}`; default order as listed, default hidden `["brain"]`. Normalisation: drop unknown/duplicate ids; if stored order equals the legacy order `["home","brain","meetings","pipes","timeline","connections"]` reset to default; insert missing ids after their default predecessor. At least one item must stay visible (refuse hiding the last one).
  - Drag to reorder (pointer sensor with 6 px activation distance, keyboard sensor); per-item menu (dropdown + right-click) offering hide/move up/move down; a sidebar-level options menu in the header that lists each hidden item with a show action, plus a reset-to-default action when the layout differs from default.
  - Trailing badges: Automations shows count of running scheduled tasks; Meetings shows a red dot while a meeting is recording.
  - Keyboard: `toggle_sidebar` (Ctrl/⌘+B) collapses the sidebar (persisted in localStorage `sidebar-collapsed`, base key).
- Notes: labels changed (Chat/Library/Automations); keep ids stable for deep links.

### frontend-007 — Recording status popover + low-disk guard
- Kind: changed · Size: M · Confidence: observed
- Evidence: home `fe@…:702040-710000` (test ids `recording-status-trigger`, `recording-status-popover`, `recording-status-pause-all`; per-device rows; states disabled/paused-low-storage/paused/unavailable/recording); `fe@…:822275` (status source: `/health`, `/audio/device/status`, `/vision/device/status`, `is_capture_paused`); low-disk check `fe@…:830841` (`get_active_data_dir` → `get_disk_usage(true, dir)` → `available_space_bytes` vs `get_low_disk_guard_config().thresholdBytes`); Rust `disk_pressure_notifications` (`str@0x49907f8`).
- Base touchpoints: `app/home/page.tsx` (device status already polled), `components/app-sidebar.tsx`, `src-tauri/src/disk_usage.rs`, `src-tauri/src/recording.rs`.
- Spec:
  - Trigger in the sidebar header shows one status: recording, paused, paused because storage is low, disabled, or status unknown; while a meeting runs a meeting-notes suffix is appended.
  - Popover: list monitors and audio devices with per-device pause/resume (POST `/vision/device/stop|start` body `{monitor_id}`; `/audio/device/stop|start` body `{device_name}`; optimistic toggle with rollback); global pause/resume for every device (calls the existing stop/start capture path); when both `disableAudio` and `disableVision` are true show a notice that capture is off with a link into settings; a low-storage block offering a re-check of free space and a jump to storage management (→ settings `storage`).
  - Low-disk guard only active when `stopRecordingOnLowDisk` is not false. New command `get_low_disk_guard_config → {thresholdBytes}`; threshold value is engine-defined (not observed; choose e.g. 2 GiB).
  - Refresh on `audio-device-status-changed` and `tray-recording-state-changed`.

### frontend-008 — Command palette, in-app keybindings, shortcut guide
- Kind: new · Size: M · Confidence: observed
- Evidence: home `fe@…:72312-76068` (`command-palette-input`, `command-palette-list`, `command-palette-<id>`; actions: search, open timeline overlay, new chat, pause/resume recording, next/previous chat tab, toggle sidebar, open settings, shortcut guide, edit global shortcuts); keybinding table `fe@7451-…:80200` (ids + defaults); guide `data-testid="shortcut-guide"` (in settings and chat chunks).
- Base touchpoints: `app/home/page.tsx`, new `components/command-palette.tsx`, new `lib/keybindings.ts`, `components/settings/shortcut-section.tsx`.
- Spec:
  - In-app (window-local, fixed) bindings — id → mac / other: `new_chat` ⌘N / Ctrl+N; `close_tab` ⌘W / Ctrl+W; `archive_chat` ⇧⌘A / Ctrl+Shift+A; `rename_chat` ⌥⌘R / Ctrl+Alt+R; `pin_chat` ⌥⌘P / Ctrl+Alt+P; `branch_chat` ⌥⌘F / Ctrl+Alt+F; `command_menu` ⌘K / Ctrl+K; `shortcut_guide` ⌘/ / Ctrl+/; `toggle_sidebar` ⌘B / Ctrl+B; `toggle_right_sidebar` ⌥⌘B / Ctrl+Alt+B; `next_recent_chat` ⌃Tab / Ctrl+Tab (hold Ctrl to cycle, release to commit); `previous_recent_chat` ⌃⇧Tab / Ctrl+Shift+Tab. Matching uses `KeyboardEvent.code` first, then case-insensitive `key`. Ignore when a Radix dialog/menu is open (`[role=dialog][data-state=open]`, `[role=menu][data-state=open]`).
  - Command palette (cmdk) with fuzzy filter; analytics events in target (`command_palette_opened/action`) must not be sent.
  - Shortcut guide modal: lists in-app shortcuts (Fixed) and global shortcuts (from settings keys `showScreenpipeShortcut`, `showChatShortcut`, `starSessionShortcut`, `searchShortcut`, `start/stopRecordingShortcut`, `start/stopAudioShortcut`; disabled ones listed as Disabled) with a link to edit global shortcuts in settings.

### frontend-009 — Chat tabs, split pane, side chat, groups, archive, branch
- Kind: changed · Size: L · Confidence: observed
- Evidence: chat chunk `3696-…` (`chat-tab-strip`, `chat-tab-rename-<id>`, `chat-tab-close-<id>`, `chat-split-pane`, `split-chat-message-<id>`, `chat-tab-worktree-<id>`, an ephemeral side-chat entry, and tab-menu actions for split open/close and for closing the other tabs or every tab to the right); home chat sidebar `fe@…home:629146-687133` (`chat-sidebar`, Pinned/Recents/Automations groups, `chat-sidebar-group-<id>`, `chat-row-<id>`, `chat-row-move-to-group-<id>`, `archive-all-recent-chats`, `recents-filter-<id>`, sort, search scopes chats/scheduled tasks/archived/all, bulk select/archive/restore/delete); events `chat-sidebar-group-changed`, `chat-preset-restore`; localStorage `screenpipe:group-expanded:<id>`, `screenpipe:recents-subsection-collapsed:<id>`.
- Base touchpoints: `components/chat-sidebar.tsx`, `components/standalone-chat.tsx`, `components/chat/recent-chat-switcher*.tsx`, `lib/stores/chat-store.ts`, `lib/chat-storage.ts`, `components/hooks/use-chat-conversations.ts`.
- Spec:
  - Conversation file (`~/.screenpipe/chats/<id>.json`) gains: `lastContentAt`, `lastViewedAt` (unread = content after view), `archived` (inferred from Archive/Unarchive actions; base used `hidden`), sidebar group id (user-defined groups; name validated non-empty), `importedFrom` (frontend-010), coding-workspace link (frontend-017).
  - Open chats form a tab strip (small working set; closing a tab never deletes/stops the conversation). Tab status glyph: error / live (thinking, using tool, streaming) / N queued / unread dot / relative age. Tabs can be renamed inline, closed, closed-others, closed-to-the-right, opened in a split pane (second conversation side-by-side), or opened as a temporary side chat (ephemeral; not available for ACP coding agents because their history cannot be guaranteed ephemeral).
  - Row/context actions: rename, pin/unpin, branch (copy messages into a new conversation; error toast on failure), archive/unarchive (with an undo toast), copy (markdown/link/chat ID/worktree path), move to an existing or newly created group, delete (confirm).
  - Sidebar: sections Pinned, Recents (filterable, sortable, with options controlling how recents are organised and which chats appear there, plus archive-all), Automations (pipe runs grouped per pipe, older runs revealed on demand).
  - Use `list_chat_entries_by_mtime(dir)` + `get_chats_dir()` to list conversations fast instead of reading every file.
- Notes: keep base `hidden` semantics readable for back-compat.

### frontend-010 — Import chats from Claude Code and Codex
- Kind: new · Size: M · Confidence: observed
- Evidence: home `fe@…:595677-599000` (`l1`: `~/.claude/projects/<project>/*.jsonl` source `claude-code`; `l3`: `~/.codex/sessions/YYYY/MM/DD/*.jsonl` source `codex`; lookback 7 days; codex scan capped at 100 sessions; files > 32 MiB skipped; maintenance action `remove-screenpipe-background-import` for sessions that screenpipe itself spawned); an import dialog with per-source checkboxes and counts; a result toast reporting how many chats were new, updated, skipped and failed.
- Base touchpoints: `components/chat-sidebar.tsx` (entry point), `lib/chat-storage.ts`, new `lib/chat-import/*.ts`.
- Spec: parse each JSONL session into a conversation with `importedFrom:{source, sourceId, importedAt, config:{model, reasoningEffort|modeId}, harness}`; conversation id derived deterministically from `(source, sourceId)` so re-import updates in place (no duplicates); preserve user-set titles (`titleSource:"user"`). An imported conversation is resumable: opening it starts a session with the recorded agent/model when available.
- Notes: read-only access to the user's home dir via `plugin-fs`; no network.

### frontend-011 — Chat composer upgrades (slash commands, tokens, filters, dictation, large paste)
- Kind: changed · Size: M · Confidence: observed
- Evidence: slash registry `fe@starred-sessions/page-…:2400` (module 28535: `/new` aliases clear/reset, `/stop` aliases cancel/abort, `/inspector` aliases sources/trace, `/pipes` "Automations" aliases automations/schedules/tasks; `requiresEmptyComposer`); token parser (same chunk ~11500: `~N days` 1–36500, `~lastweek`, `~lastmonth`, `~<month>[YYYY]`, `~(MM/DD/YYYY)`, ranges, plus base `@today/@yesterday/@last-week/@this-morning/@last-hour/@selection/@audio/@screen/@input/@<app>`); chat chunk `3696-…:357302-383918` (menu groups Commands, Recent chats, Installed skills, Time ranges, Time, Content type, Speakers, Tags, Apps; `/speakers/search`, `/tags/autocomplete`); dictation `3696-…:3300-19198` (`composer-dictation-recording`, `composer-dictation-waveform`; recorded webm blob POSTed to the vendor cloud `/listen?language=multi` with header `X-Transcription-Provider: deepgram` and the user's bearer token; 401/403 → sign-in error, 429 → rate-limited); oversized pasted context written out to a file `3696-…:668621`.
- Base touchpoints: `components/standalone-chat.tsx`, `lib/chat-utils.ts`, `lib/chat-queue-controls.ts`.
- Spec:
  - Typing `/` at the start of an empty composer opens a command menu (builtins above + installed skills from `list_imported_skills`). Unknown slash text is sent as-is.
  - Extend the time-token grammar with `~` forms listed; tokens become chips, and each resolved range gets a human-readable label (a trailing N-day window, the prior week, the prior month, or a named month plus year).
  - Composer popover for attachments and filters: Time, Content type, Apps, Tags, Speakers, Connections (adds a connection context chip, which the user can remove again).
  - Dictation: press-to-toggle or hold; Esc cancels, Enter finishes; shows waveform; failure states retry/dismiss. In target audio is sent to the vendor cloud for transcription (copy says so) → **decloud**: route to the local transcription engine (or the engine's `/audio/*` transcription API) instead.
  - Pasted text over the inline limit is written to a local chunk file and referenced instead of sent inline; if writing fails the message is not sent.
  - Send button disabled while an attachment is extracting; queued/steer (base) unchanged.

### frontend-012 — Rich chat rendering blocks
- Kind: new · Size: L · Confidence: observed
- Evidence: `3696-…` case labels `text, thinking, tool, connection_action, agent_action, plan`; charts `chat-chart` with types `stat, bar, line, grouped_bar, stacked_bar, proportion, heatmap, timeline, calendar, funnel, waterfall, range, scatter` (`3696-…:100905`); `chat-plan-block`/`chat-plan-progress`/`chat-plan-entry`; `structured-output`; `chat-rich-result-*`; `tool-activity-widget`/`tool-activity-item`/`tool-activity-group-details`; `ask-user-tool-card` (+`ask-user-answer-*`, `ask-user-reply`); `agent-action-card` (ACP permission choices `allow_once, allow_always, reject_once, reject_always`); sandboxed HTML preview; `context-usage-panel`; turn status `chat-turn-status` (starting, analyzing, tool, writing, done, stopped, interrupted, failed); `source-citation-toggle` (open in search/timeline/preview); selected-text actions (`selected-text-actions`: insert the selection into the chat, or ask about it in a side chat).
- Base touchpoints: `components/standalone-chat.tsx`, `components/markdown.tsx`, `components/chat/source-citation-footer.tsx`, `lib/chat/tool-presentation.ts`, `lib/pipe-ndjson-to-chat.ts`.
- Spec:
  - Message content is a list of typed blocks; renderers per kind. Chart block: a JSON spec fenced in markdown (type + series/categories/values + optional title/units); the renderer computes Average/Peak/Total summaries and offers chart actions (copy data/image). Implement with SVG (no external chart lib observed).
  - Plan block: ordered entries with status (pending/in-progress/completed) and progress bar.
  - Tool activity: collapse consecutive tool calls into a widget with current target, step count, expandable list (Command/Response technical details, JSON pretty view).
  - Ask-user card: when the agent requests input (options and/or free text), show choices + custom answer; the user's pick is posted back to the agent as a new chat turn.
  - Agent action card: ACP permission request with the four choices; also sign-in prompts for agents.
  - Context usage panel: shows context window usage; estimated breakdown by category: system prompt, tool definitions, rules, skills, MCP and dynamic tools, subagent definitions, the summarised part of the conversation, and the live conversation.
  - HTML artifacts render in a sandboxed iframe without network (scripts/images/requests blocked) with a source/rendered toggle (shared with viewer, frontend-028).

### frontend-013 — Right sidebar: Browser tabs, Files & outputs, Connections
- Kind: changed · Size: L · Confidence: observed
- Evidence: `3696-…:253323-291315` (side panel home with three entries; `right-panel-tab-strip`, `right-panel-tab-<id>`, `file-preview-sidebar`, `owned-browser-toolbar`, back/forward/address/reload, session cookie prompts incl. Windows Chrome/Edge app-bound encryption notice and Browser Bridge extension suggestion); commands `owned_browser_tab_*`, `owned_browser_history`, `owned_browser_navigate(url, owner, reveal)`; events `owned-browser:navigate|state|session-access-request|v20-cookie-blocked`.
- Base touchpoints: `components/browser-sidebar.tsx`, `src-tauri/src/owned_browser.rs`, `src-tauri/src/owned_browser_cookies.rs`, `lib/owned-browser-ownership.ts`, `lib/browser-state-cache.ts`.
- Spec:
  - Right sidebar toggled with `toggle_right_sidebar`; tabs: Browser tabs (many), Files and outputs, Connections. Empty home lists the three tools.
  - Browser: each tab is a child webview identified by `tabId`, positioned with `owned_browser_tab_set_bounds(tabId, parent, x, y, width, height)`; navigation via `owned_browser_tab_navigate(tabId, url, owner)` where `owner` is the conversation id that owns the tab; history via `owned_browser_history(tabId, "back"|"forward")`; close/hide/clear data per tab. Rust emits `owned-browser:state` (url/title/loading/canGoBack/canGoForward per tab — inferred) and `owned-browser:navigate`.
  - Cookie access: global `browserCookieAccessGranted` + per-session confirmation (`confirm_browser_cookie_access_for_session`, `owned_browser_resolve_session_access(requestId, allow)`); on Windows when Chrome v20 app-bound cookies block reuse, show an explanatory card and let the user proceed signed-out.
  - Files and outputs: files previewed or created in this chat (reveal in OS, copy content, copy path) — uses `reveal_in_default_browser`/`open_note_path`.

### frontend-014 — Response & notification feedback (local)
- Kind: new · Size: M · Confidence: observed
- Evidence: module 68186 `fe@3822-215756f0622728d9.js:8421` (POST `/feedback` with `{target:{kind:"notification"|"chat", id}, rating:"up"|"down", comment?, snapshot:{title,body}|{response,truncated}, producer_ref:"pipe:<name>"|"agent:screenpipe-chat", context:{…}}`; response snapshot capped at 12 000 chars); chat thumbs `3696-…:485367` (reason list, plus a note that feedback stays on the device by default); notification feedback component (`notification-feedback`, hover-reveal, up/down + an optional improvement comment); Live View card feedback (frontend-022).
- Base touchpoints: `components/standalone-chat.tsx`, `app/notification-panel/page.tsx`, `components/notification-bell.tsx`; engine route `/feedback` (API dimension).
- Spec: thumbs up/down on assistant messages (down opens a short reason picker), on pipe-originated notifications, and on Live View cards. All feedback is stored locally through the engine `/feedback` endpoint; a pipe producer can read it to improve later runs. Show a status confirming local storage; on failure allow retry.
- Notes: no network beyond localhost.

### frontend-015 — ACP coding-agent presets and sessions
- Kind: new · Size: XL · Confidence: observed
- Evidence: agent catalog `fe@7451-…:11323` (ids `pi-acp`, `codex-acp`, `claude-acp`, `opencode` (disabled), `cursor`, `github-copilot-cli`, `kimi` (disabled), plus `custom` "Agent command"; launch kinds `npx` with pinned package or `binary` with command/args; optional `login.args`, `installUrl`, `installer`, `httpMcp`, `flag`, `cloudRouting`); also embedded in exe (`str@0xa0c9c05`); preset UI `7451-…:20881-51370` (`acp-setup-progress` states checking/downloading/installing/starting/connecting/ready; `acp-install-gate`; `acp-preset-install`; `acp-preset-signin`; `acp-boundaries`; agent command, startup options one-per-line, env var **names** only); chat `3696-…:308979-316000` (permission modes `read-only, default, agent, plan, agent-full-access`, `acp-permission-trigger/popover/option-*`, thinking level "Effort"); commands `pi_acp_*`, `pi_set_thinking_level`, `pi_get_thinking_level`; images `fe:images/acp/*.svg`.
- Base touchpoints: `components/settings/ai-presets.tsx`, `components/rewind/ai-presets-selector.tsx`, `lib/hooks/use-settings.tsx` (`AIProviderType`), `components/standalone-chat.tsx`, `src-tauri/src/pi.rs`, `src-tauri/src/pi_command_queue.rs`.
- Spec:
  - New provider `acp` in `AIPreset.provider`; preset carries `acpAgent: {id, command?, args?, envNames?, config?, modeId?, useScreenpipeCloud:false}` (`command/args/envNames` inferred from the custom-agent form).
  - Settings → Models & keys gains an entry for adding another agent, either from the catalog or as a custom command. Flow: probe (`pi_acp_probe_agent`) to list models/modes; if missing, offer install (`pi_acp_agent_install`, poll `pi_acp_agent_install_status`, `pi_acp_agent_download_pending`), then agent-owned login (`pi_acp_external_login`). Show a "boundaries" panel explaining the agent runs as its own process with its own account; only env var names are saved, and the agent picks up their values from the environment each time it starts.
  - In chat: mode selector (`pi_acp_set_mode`), config options (`pi_acp_set_config_option`, boolean or string), permission mode (`pi_acp_set_approval_mode`), thinking/effort (`pi_set_thinking_level`), re-auth (`pi_acp_reauthenticate`); billing/limit errors from agents surface as recoverable errors.
  - `pi_start_and_prompt(sessionId, projectDir, userToken, providerConfig, message)` starts the agent and sends the first message atomically; `pi_stop_if_idle` used on tab close.
- Notes: drop `cloudRouting` (vendor proxy for Claude) — **decloud**. Pin npx package versions as configurable defaults, not hard-coded vendor endpoints. Keep `pi` as default harness.

### frontend-016 — Pi extension packages and extension UI requests
- Kind: new · Size: M · Confidence: observed
- Evidence: `5977-…:195682-197002` (`pi_list_extension_packages`, `pi_install_extension_package(source)`, `pi_remove_extension_package(source)`); mock shape `{source, scope:"user", filtered, installed, acpCompatible}`; `pi_extension_ui_response(sessionId, requestId, response)` from chat `3696-…:672896`; Rust error strings for a malformed (non-object) extension UI reply and for an attempt to turn off the mandatory subagents extension (`str@0x4992810`).
- Base touchpoints: `components/settings/skills-card.tsx` (or new extensions card), `components/standalone-chat.tsx`, `src-tauri/src/pi.rs`.
- Spec: list installed/available Pi extension packages, install by source string (npm spec), remove; refuse changes while the agent is mid-turn. When an extension asks for UI input, render a prompt in chat and reply with `pi_extension_ui_response`. The `npm:pi-subagents` extension is mandatory and cannot be removed.

### frontend-017 — Coding workspace (per-chat git worktree)
- Kind: new · Size: M · Confidence: observed
- Evidence: `3696-…:343090-346325` (`coding-workspace-row`, `-badge`, `-popover`; isolated-worktree explainer: created from HEAD leaving uncommitted changes untouched, kept with the conversation and never auto-removed, setup failure → send again without worktree); `3696-…:393744-399700` (`coding_workspace_prepare/create/get`); Rust module `coding_workspace` (`str@0x498dfd9`).
- Base touchpoints: `components/standalone-chat.tsx`, new `src-tauri/src/coding_workspace.rs`.
- Spec: for ACP coding-agent chats, `coding_workspace_prepare(conversationId, prompt, startingPath)` resolves a repository; `coding_workspace_create(conversationId, repositoryPath)` creates a git worktree from HEAD under the screenpipe data dir and returns its path; `coding_workspace_get` returns the existing one. The chat header shows a worktree badge with path (with a copy-path action), and the agent's project dir is the worktree.

### frontend-018 — First-run summary, learning banner and agent hand-off
- Kind: new · Size: M · Confidence: observed
- Evidence: chat `3696-…:23040-35243` (`first-run-ask-agent`, `first-run-captured-app-*`, `first-run-open-summary`, `first-run-learning-banner`, `first-run-countdown`, `first-run-setup-ready/complete`); `(main)` layout `fe@…:14261-18230`; event `first-run-summary-state`; `get_onboarding_status` fields `firstRunSummaryPhase`, `firstRunSummaryStartedAt`, `firstRunSummaryChatId`; deep links `screenpipe://first-run-summary`, `screenpipe://first-run-agent?target=…`; Rust `first_run_summary` (`str@0x498e97e`).
- Base touchpoints: `components/standalone-chat.tsx`, `app/home/page.tsx`, `lib/hooks/use-onboarding.tsx`, `src-tauri/src/commands.rs` (onboarding status).
- Spec: after onboarding, Rust waits until enough activity is captured, then generates an evidence-backed summary of the apps/activity since setup into a new chat and emits `first-run-summary-state`. Phases: `idle` → learning (banner with countdown and chips naming the apps being read) → writing → ready (card linking to the summary) or empty (not enough activity; setup marked ready anyway). Hand-off: a per-agent ask picker (Claude, Codex, Cursor, ChatGPT) copies a prompt to the clipboard and opens the agent (deep link/URL) — local only.
- Notes: trial/paywall variants of this card (`trial-activation-*`) are vendor commerce → omit (frontend-051).

### frontend-019 — First-run guide (3-step coach marks)
- Kind: new · Size: S · Confidence: observed
- Evidence: home `fe@…:714376-729074` (`firstrun-scrim`, `firstrun-invite`, `firstrun-accept`, `firstrun-decline`; steps "1 of 3" prefilled prompt → send, "2 of 3" automation being set up, "3 of 3" run the scheduled task; Esc exits; rerun from Help); setting `firstRunGuideDone`; event `first-run-guide-pending`.
- Base touchpoints: `app/home/page.tsx`, `components/standalone-chat.tsx`.
- Spec: optional guided tour shown once (`firstRunGuideDone=false`): offer → prefill a chat prompt that creates a scheduled task → point to Automations → prompt to run it; any skip sets `firstRunGuideDone=true`.

### frontend-020 — Home chat landing cards and agent actions
- Kind: changed · Size: S · Confidence: observed
- Evidence: `3696-…:460672-475548` (`summary-card-*`, `custom-summary-open`, `home-contextual-suggestion-*`), `home-card-agent-actions-*` (Run in agent / copy prompt) + CSS module `home-card-agent-actions_*` and `fe:pr-assets/home-card-agent-actions/*.jpg`; suggestions `get_cached_suggestions`, `force_regenerate_suggestions`; setting `showChatSuggestions`.
- Base touchpoints: `components/chat/summary-cards.tsx`, `components/chat/custom-summary-builder.tsx`, `components/standalone-chat.tsx`.
- Spec: keep base cards; add a per-card agent-launch stack (Codex/Claude/Cursor/Screenpipe icons) that copies the card prompt and opens the chosen agent; contextual suggestion cards seeded from the user's recent activity; suggestion chips above the composer obey `showChatSuggestions` (hide button on chips writes false).

### frontend-021 — Library section (Live Views · Memories · Artifacts)
- Kind: changed · Size: L · Confidence: observed
- Evidence: home `fe@…:297694-351392` (`section-brain`, `brain-view-switcher`, `brain-filter-*`, `brain-search-input`, `brain-add-memory-*`, `brain-edit-memory-*`, `brain-select-all`, `brain-delete-selected`, `brain-artifacts-error`, `brain-item-artifact-*`, `brain-artifact-preview-*`, `brain-detail-panel`, `brain-detail-ask-ai`, `brain-artifact-request-*`); API `/memories` (GET/POST), `/memories/<id>` (PATCH/DELETE), `/artifacts`, `/artifacts/<id>`, `/tags/autocomplete`; engine routes `/memories/tags`, `/artifacts/register` (`str@0x5257ca8`); deep link `screenpipe://artifact`.
- Base touchpoints: `components/settings/memories-section.tsx` (base memories UI), `app/home/page.tsx`, `lib/source-citations.ts`.
- Spec:
  - Library has a view switcher: Live Views (frontend-022), Memories, Artifacts. Filters popover: Labels (tags), Sources, People, Dates with search.
  - Memories: list with search, add (text + tags), edit, copy, delete (confirm), multi-select delete; footnote that scheduled tasks create memories automatically.
  - Artifacts: outputs registered by scheduled tasks/chats (title, kind, preview, source run/chat). Actions: open in viewer window (`open_viewer_window`), go to task run, go to chat, ask AI, copy, delete; HTML artifacts get rendered/source toggle; "SAF" artifacts (structured SOP/frames) show frame thumbnails from `/frames/<id>` and step lists (`saf-sop-view`, `saf-sop-steps`). An artifact request state covers three cases: still being located, not available, and the index cannot be reached.
- Notes: Library nav item is hidden by default (frontend-006); onboarding completion opens `{Home:{page:"brain"}}`.

### frontend-022 — Live Views (brain-view dashboards)
- Kind: new · Size: XL · Confidence: observed
- Evidence: commands A.1 (Library / Live Views); data shapes from mock `fe@app/layout-…:41000-57800` and template kits `fe@app/layout-…:34200`; home `fe@…:33600-296000` (blocks, editor, AI composer, templates, share, onboarding activation); Rust `brain_views` (`str@0x49875a0`); schema id `live-view-canvas.v1` (`str@0x5280510`); deep link `screenpipe://live-view/<id>`.
- Base touchpoints: new `components/library/live-views/*`, `app/home/page.tsx`, new `src-tauri/src/brain_views.rs`; storage under the screenpipe base dir (path not observed — choose `~/.screenpipe/brain-views/`).
- Data model (interop field names):
  - `BrainView {id, title, revision, timeRange, periodPolicy, slots[], createdAt, updatedAt}`; `periodPolicy` = `{type:"fixed.v1", value}` or `{type:"selectable.v1", values:[…]}` with range tokens `today, 24h, 7d, 30d`.
  - `Slot {id, title, component, width (1–12; UI widths Quarter=3, Half=6, Full=12), order, intent (natural-language spec), binding:{pipeName}, value?:{payload, evidence:[{eventId, frameId, transcriptionId, ts, deviceId}], sourcePipe, artifactOutputId, artifactVersion, updatedAt}, feedback:{upCount, downCount, current}, itemActions:{items:[]}}`; components `markdown.v1, list.v1, bar-chart.v1, line-chart.v1, timeline.v1` (others possible).
  - `Canvas {schema:"live-view-canvas.v1", viewId, revision, mode:"dashboard", viewport:{x,y,zoom}, blocks:[{slotId,x,y,width,height}], notes:[], arrows:[], strokes:[], updatedAt}`.
  - `TemplateKit {id, title, description, version, timeRange, periodPolicy, pipes:[{name, distribution:"bundled"|"store"}], slots:[…]}`; built-ins observed: `daily-memory` (pipes `day-recap`, `missed-todos`), `meeting-follow-ups`, `work-patterns` (`activity-summary`), `process-map` (`automate-my-work`). Goal categories map to templates: `work_memory`, `meeting_follow_through`, `work_patterns`, `process_automation`, `custom`.
  - Saves use optimistic concurrency: `save_brain_view({…, expectedRevision})` and `save_brain_view_canvas({…, expectedRevision})` increment revision; a mismatch errors and the UI reloads.
- Behaviour:
  - Empty state: choose an outcome (template gallery `live-view-template-gallery`, readiness per template), describe the desired view in a prompt, or start blank. A dashboard limit exists (exact number not observed; implement a configurable cap, e.g. 20).
  - Each slot is filled by its bound pipe: the UI enables/configures the pipe (`/pipes/<name>/config`, `/pipes/<name>/enable`, install bundled/store pipes) and triggers `/pipes/<name>/run`; values come from the pipe's published artifacts (`/outputs/targets/<…>`, `/artifacts`). Freshness indicators: stalled sources, unconfigured blocks, warnings on blocks that have no automatic refresh; a manual refresh action.
  - Header: dashboard selector/rename/duplicate/delete/restore previous, time-range picker (only when policy is selectable), undo last change, send (connected share), refresh.
  - Editor (`brain-overview-editor`): drag blocks, change width, add/edit/delete block (title, type, connected scheduled task, intent). Canvas mode uses React Flow with persisted viewport and free-form notes/arrows/strokes.
  - AI composer: user describes a change; an agent session (`pi_start`/`pi_prompt` with a builder prompt) proposes slot changes, shown as a diff review (`live-view-ai-review`: accept all / reject all / apply accepted / discard). Requires a configured AI preset; quota errors shown.
  - Card feedback: thumbs + an optional improvement comment routed to the bound pipe via `/feedback` (frontend-014). Item actions on list items: remind me (1 h / tomorrow / next week), fix item (teach the task), remove (move to Handled; reopenable), resolve.
  - Connected share (`connected-share-dialog`): send a frozen snapshot to Slack (`/connections/slack/instances`, `/connections/slack/conversations`, `/connections/slack/send`) or create a Linear issue (`/connections/linear/proxy/graphql`), either unchanged or via an editable chat prompt that must ask before writing.
  - Onboarding activation: after first-task goal selection the matching template is created and the page shows a cold-start state until the first real result arrives (`onboarding-live-view-*`), then a 3-step spotlight guide.

### frontend-023 — Activity section (activity ledger + generated activity history)
- Kind: new · Size: L · Confidence: observed
- Evidence: home `fe@…:751200-769000` (`activity-ledger`, `activity-range`, `activity-artifact-preview`, `activity-ledger-skeleton*`); HTTP `/activity-ledger?start_time&end_time&depth=task&include_artifacts=true&refresh=false` → `{intervals:[…]}`, `/activity-summary` with `include_key_texts|include_memories|include_snippets|include_recording|include_guidance=false`, `/meetings?start_time&end_time&limit=100`, `/frames/preview-samples`, `/app-icon`; commands `get_activity_history`, `generate_activity_history`; event `activity-history-updated`; settings `activitiesEnabled`, `activitiesIntervalMinutes` (settings section `section-settings-activities`, toggle `activities-enabled-toggle`, intervals 5/15/30/60 min); Rust `activity_history`, `activity_history\quota` (`str@0x5287fcc`, `str@0x621d890`).
- Base touchpoints: `app/home/page.tsx` (new section), new `components/activity/*`, new `src-tauri/src/activity_history.rs`, `components/settings/*` (new section).
- Spec:
  - Range picker: presets (today etc.) or custom date range; ranges persisted in localStorage (3 keys, names not observed). Validation: start < end.
  - Ledger: time intervals from `/activity-ledger` merged with meetings (clipped to range; title fallback "<app> meeting") and frame preview samples; rows link to evidence (opens timeline at frame/timestamp via `navigate-to-frame`/`navigate-to-timestamp`).
  - Generated history: `get_activity_history(start,end)` returns stored entries + `coverage` intervals; the generate button calls `generate_activity_history(start,end,rangeKey)` (idempotency key = range id) with progress copy; errors `activity_no_data:not_recording|no_capture_in_range|empty_but_recording` mapped to specific empty states. Generating once sets `activitiesEnabled=true` so Rust keeps entries updated every `activitiesIntervalMinutes` (default 15) and emits `activity-history-updated`.
  - Entry actions: turn the entry into a skill (hand to Workflows/skill generation), or open a chat with the entry as context. AI preset selector (automatic choice by default) and refresh.
  - Quota exhaustion pauses automatic summaries with a retry button in settings.

### frontend-024 — Meetings workspace upgrades
- Kind: changed · Size: L · Confidence: observed
- Evidence: home `fe@…:358940-589343` (Coming up sources `native|google|ics`; merge dialog `/meetings/merge`; copy full meeting; delete; search; attendees via `/speakers/search`; Google Meet/Zoom/Teams join; `replay-*` player; note editor `note-editor`, `slash-command-menu`, `format-toolbar`, image drop; `meeting-summary-*` surface with streaming, status `/meetings/<id>/summary-status` (`running|ready|failed|pending`), model change; `meeting-transcript-scroll`, `transcript-row`, follow-live; `meeting-chat-panel` (ask questions about the meeting); share `meeting-share-button` (copy rich text via `copy_rich_text_to_clipboard`, send to connected apps), export mp4 `start_export_recording` + `export:event`; retranscribe `/meetings/<id>/retranscribe`; resume mic `/audio/device/start`; 15-minute inactivity auto-stop notice; audio health popover; consent reminder); `(main)` layout calendar nudge + join suggestion (`/connections/calendar/events`, `/connections/google-calendar/events`, `calendar_status`, `oauth_status`, `ics_calendar_get_upcoming`).
- Base touchpoints: `components/meeting-notes/*` (index, list-view, note-view, note-editor, transcript-panel, replay-strip, coming-up, past-meetings, calendar-connect-dialog), `lib/hooks/use-meetings.tsx`, `src-tauri/src/meeting_export.rs`.
- Spec deltas over base:
  - Replay player: frame viewport synced to audio; play/pause, speed, mute, scrubber, per-segment transcript; a jump from the current replay position to the same moment in the main timeline.
  - Summary surface: streaming summary with cursor, states for in progress, ready, needs attention, nothing to summarise, and turned off; a picker to switch the summary model; summary runs only after the meeting stops; retranscribe refreshes summary when auto-summary is on.
  - Meeting chat panel: resizable side conversation scoped to the meeting with suggestions, citations, model controls.
  - Share: copy summary as rich text (HTML+plain) without transcript, copy meeting+transcript, send to connected apps suggested because they were in use while the meeting ran, with a review step first.
  - Export mp4: save dialog (`<title>-<date>.mp4`), `start_export_recording(meetingId, null, null, outputPath)` returns `{jobId}`; listen on `export:event` for that job: `completed` (toast with frames/audio chunks/size; open file) or `failed`.
  - Editor: tiptap with slash block menu and floating format toolbar; drop images (png/jpg/gif/webp/bmp/svg) inserted as note images (blocked while summary is writing).
  - Live: inactivity auto-stop after 15 min of silence with resume; a resume-microphone action if microphone capture is stopped; join suggestion for a nearby calendar event; consent reminder line.

### frontend-025 — Timeline additions
- Kind: changed · Size: M · Confidence: observed
- Evidence: `1491-4c6947e70c72ead9.js` (filters Monitors/Apps/Domains/Speakers/Calls/Tags; `timeline-daily-summary-trigger/panel`, `daily-summary-enable-dialog`; selection export `/export` with a success toast pointing at the exports folder; `timeline-*` events; a paused-screenshot placeholder; `/frames/next-valid`; `/tags/vision/batch`; native timeline `native-timeline-*`; `section-timeline-disabled`).
- Base touchpoints: `components/rewind/timeline.tsx`, `components/rewind/daily-summary.tsx`, `components/rewind/timeline/*`, `lib/hooks/use-timeline-selection.tsx`.
- Spec:
  - Selection toolbar: Ask AI, delete range (`/data/delete-range`), re-transcribe audio in range, tag (custom), **export selection as video** (POST `/export` with `{start,end,include_audio:true}` → `{output_path}`; toast on success).
  - Daily summary: on-demand only, read-only bounded to the selected day; first use asks to turn on Enhanced AI (`set_enhanced_ai_suggestions`). Target requires vendor sign-in for this — **decloud**: use the configured AI preset.
  - Frames saved without an image (privacy/low-battery pause, or `disableScreenshots`) render a placeholder marking the screenshot as paused rather than an error.
  - Listen to `timeline-open-search|open-chat|toggle-sidebar|return-to-activity|open-daily-summary|ask-ai-selection|delete-range|export-video-selection` (emitted by native menus) and route to the same actions.
  - Enterprise panel explaining the timeline is turned off when policy hides it.

### frontend-026 — Native timeline bridge (macOS only)
- Kind: new · Size: S · Confidence: observed
- Evidence: `native_timeline_show(port, apiKey, embedded)`, `_hide`, `_close`, `_navigate(timestamp, frameId)`, `_is_available`; events `native-timeline-attached`, `native-timeline-attach`, `native-timeline-detach` (`1491-…:268581-270355`).
- Base touchpoints: `components/rewind/timeline.tsx`, `src-tauri/src/commands.rs`.
- Spec: if available, the webview timeline can hand off rendering to a native view; on Windows `native_timeline_is_available` returns false and the web timeline is used. Implement commands as no-ops returning false/Ok on non-mac.

### frontend-027 — Search window v2
- Kind: changed · Size: M · Confidence: observed
- Evidence: `3308-e676c712bfd27088.js` (tabs All/Screen/Keys(Keyboard & clipboard)/Chats, `#` tags and `@` apps & people filters, Recent activity list, `/search/keyword`, `/search`, `/raw_sql`, `/frames/<id>/text`, `/frames/<id>/thumbnail`, keyboard hints, `resize_search_window`, Ctrl+Enter ask AI); search page `fe:_next/static/chunks/app/search/page-…` (events `search-hidden`, `search-reset`; `search_navigate_to_timeline(..., timelineOrigin)`); localStorage `recentChatSearchOrigin` + key `recent-chat-search-handoff`.
- Base touchpoints: `app/search/page.tsx`, `components/rewind/search-modal.tsx`, `lib/hooks/use-keyword-search-store.tsx`.
- Spec: add "Keys" tab for input/clipboard events and "Chats" tab searching local conversations; `timelineOrigin` arg (`"home"|"chat"|"overlay"` — inferred) passed to `open_search_window` and `search_navigate_to_timeline` so the timeline knows where to return; reset state on `search-reset`, cancel in-flight queries on `search-hidden`. Ask-AI from a result opens the chat with the result attached.

### frontend-028 — Viewer window: sandboxed HTML preview and safer file handling
- Kind: changed · Size: S · Confidence: observed
- Evidence: viewer chunk outline (`html-render-toggle`, sandbox notice, `file-preview-markdown`, `markdown-code-block`, a file-not-opened error that lists the searched locations, unsafe-to-render notice, `read_viewer_file`, `reveal_in_default_browser`, open full file).
- Base touchpoints: `app/viewer/page.tsx`, `src-tauri/src/viewer.rs`.
- Spec: `read_viewer_file(path)` returns `{kind:"text", text, name, path, truncated, total_bytes}` or `{kind:"error", message, path}` (other kinds possible: image/binary/unsafe — inferred). HTML files default to source view with a toggle to a sandboxed iframe (`sandbox` without `allow-scripts`/network). Unsafe/binary files offer opening in the OS default application. Missing files list the directories searched. Header actions: Reveal, Copy, Copy path; images fit/zoom.

### frontend-029 — Starred work sessions
- Kind: new · Size: L · Confidence: observed
- Evidence: page `fe:_next/static/chunks/app/starred-sessions/page-fef15017d60c36f1.js` (hook at offset ~1249: GET `/starred-sessions?limit=100`, POST `/starred-sessions`, POST `/export`, 5 s polling, focus refresh, durations 5/15/30/60, +15 min, HD toggle, end session, edit start/end, ask about session, show saved video; window auto-hide after 5 s idle); commands `toggle_starred_sessions`, `hide_starred_sessions`; events `starred-session-state`, `starred-sessions-changed`, `starred-sessions-visibility`, `starred-session-toggle` (`str@0x524e740`); Rust `commands\starred_sessions` (`str@0x4999838`); API doc string `str@0x511e39f`; DB `starred_sessions` (`str@0x516d018`, `str@0x516d1d0`); setting `starSessionShortcut` (mac `Control+Super+B`, Windows `Alt+Shift+B`), shortcut arm `star_session` (`str@0x5150568`); home sidebar "Starred sessions" list (`fe@…home:709872`).
- Base touchpoints: new `app/starred-sessions/page.tsx` + layout, `src-tauri/src/window/show.rs` (new window label `starred-sessions`), `src-tauri/src/shortcuts.rs`, `components/settings/shortcut-section.tsx`, `app/shortcut-reminder/page.tsx`; engine routes (API dimension).
- Spec:
  - Data: `{id (UUID), start, end (RFC3339), hd_requested (bool), revision (int; 0 to create, echo returned value to edit), has_audio (bool, server-computed)}`. POST is an upsert; 409 → reload list. Sort newest first.
  - Global shortcut `starSessionShortcut` toggles the small black panel window `starred-sessions` near the shortcut bar. Panel: if no active session — one duration button each for 5/15/30/60 min (creates `{start: now, end: now+N min, hd_requested, revision:0}`); a checkbox requesting HD capture for the session. If active — minutes left, an extend-by-15-min button, an end button (sets `end = max(now, start+1ms)`). An expandable list shows recent sessions with inline start/end editing (validates end > start), an ask action (opens chat with a context prompt referencing `starred_session_id=<id>` and bounds), and a saved-video action (POST `/export` `{start,end,include_audio:true}` → open `output_path`; only after the session ended).
  - The window emits `starred-session-state` with the active session's end (or null) so the shortcut bar shows a star badge; any change dispatches/emits `starred-sessions-changed`. When opened from the shortcut, the panel auto-hides after 5 s without pointer/keyboard activity (timer resets on any interaction and is suspended while saving, editing or showing an error); `hide_starred_sessions` on Escape/close.
  - Search: chat/search can filter by `starred_session_id`.

### frontend-030 — Notification inbox window
- Kind: new · Size: M · Confidence: observed
- Evidence: page `fe:_next/static/chunks/app/notification-inbox/page-1108e01f2e609f2b.js` (GET `/notifications` every 5 s, POST `/notifications/<id>/read`, DELETE `/notifications`, DELETE `/notifications/<id>`; Priority/All tabs; an urgent group and a group for everything else; expand marks read; actions; copy; open source; ask AI; manage settings; feedback); command `show_notification_inbox`; window label `notification-inbox` (`str@0x4999dd0`); analytics schema in module 690 (`notification_v2`).
- Base touchpoints: `components/notification-bell.tsx` (base popover), new `app/notification-inbox/page.tsx`, `src-tauri/src/window/*`, `src-tauri/src/notifications/*`.
- Spec:
  - The bell in Home now opens a separate small inbox window (positioning/always-on-top not observed) via `show_notification_inbox` (test ids keep the base `notification-bell-*` names: `-trigger`, `-unread-dot`, `-popover`, `-clear-all`, `-view-<tab>`, `-empty`, `-item-<id>`, `-dismiss-<id>`, `-action-<id>`, `-expanded-<id>`, `-copy-<id>`, `-source-<id>`, `-ask-ai-<id>`, `-manage-settings`).
  - Notification fields used: `id, type ∈ {general, meeting, pipe, system}, priority ∈ {high, normal, low}, title, body, read, pipe_name, source_session_id, source_message_id, source_url, actions[{label, type ∈ {api, chat, copy, deeplink, dismiss, link, manage, meeting_join, mute, pipe, source}, open_in_chat, …}]`. Ids prefixed `workflow-review:` and `onboarding-live-view-follow-up:` are special categories.
  - Priority view shows only high-priority items plus a link stating how many other updates are waiting in the All view; All view shows high first then others. Expanding an item marks it read (POST `/notifications/<id>/read`, optimistic). Executing an action removes the item (DELETE) unless it fails.
  - Feedback row for pipe-originated items (frontend-014).

### frontend-031 — Notification panel (toast) updates
- Kind: changed · Size: S · Confidence: observed
- Evidence: panel chunk outline (feedback, a jump to the originating chat, opening the file in the OS default app, restart server action via `stop_screenpipe`+`spawn_screenpipe`+`/health`, copy); Rust suppression strings `str@0x4999f98`, `str@0x499a02f` (repeat cooldown; master/snooze/quiet suppression; meeting notification rendered inside shortcut overlay).
- Base touchpoints: `app/notification-panel/page.tsx`, `src-tauri/src/notifications/*`, `lib/hooks/use-notification-panel.ts`.
- Spec: add per-toast feedback (pipe items), an action that opens the originating chat when `source_session_id` exists, an open-with-default-app action for file-backed items; Rust suppresses toasts when notifications are master-off, snoozed, in quiet hours (VIP pipes excepted) or repeated within a cooldown; meeting-start notifications can render inside the shortcut overlay instead of a separate toast.

### frontend-032 — Shortcut reminder overlay v2
- Kind: changed · Size: L · Confidence: observed
- Evidence: `fe:_next/static/chunks/app/shortcut-reminder/page-a5341a3196435d05.js` (sizes at `~9500`: collapsed 22×16, base 192×62, meeting 164×96, 160×40, incident 280×80, multiplied by size scale small 1 / medium 1.5 / large 2; `/ws/meeting-overlay` with backoff 1 s→10 s; `/meetings/stop {id: stoppableMeetingId}` then verify `/meetings/status` after 6 s; health state parsing; anchors `top-center, bottom-center, left-center, right-center`; drag test ids `shortcut-reminder-drag-stage|pad|ghost`; context menu with a one-hour hide entry and a settings entry → `showWindow({Home:{page:"display"}})`); commands `get_recording_health_state`, `overlay_restart_recording`, `overlay_dismiss_incident`, `snooze_shortcut_reminder_for_hour`, `set_shortcut_overlay_anchor`, `toggle_starred_sessions`; Rust `overlay_health` (`str@0x498b756`, `str@0x498b880`, `str@0x49f012b`, `str@0x5a0c5e2`, `str@0x5a0c6d8`), `commands\overlay_anchor` (`str@0x51796c6`).
- Base touchpoints: `app/shortcut-reminder/page.tsx`, `app/shortcut-reminder/use-overlay-data.ts`, `src-tauri/src/native_shortcut_reminder.rs`, `src-tauri/src/windows_overlay.rs`, `components/settings/display-section.tsx`.
- Spec:
  - Shortcut bar shows overlay/search/chat/star shortcuts (from settings, honouring `disabledShortcuts`; defaults: overlay mac `Super+Ctrl+S` / other `Alt+S`; chat `Control+Super+L` / `Alt+L`; search `Control+Super+K` / `Alt+K`; star `Control+Super+B` / `Alt+Shift+B`). Click opens timeline; right-click opens options menu (hide for 1 h via `snooze_shortcut_reminder_for_hour`, open overlay settings). Re-read settings on store `settings` key change (250 ms debounce) and on `shortcut-reminder-update`.
  - Recording health: `get_recording_health_state()` at mount + `recording-health-state` events; payload `state|reason|component|recovery` where state ∈ `normal | failure | fixing | recovering | recovered`, component ∈ `audio | screen | ""`, recovery `manual-reopen` means only quitting and reopening helps. Failure view: red pulsing dot + a line naming the failing component, a restart button (`overlay_restart_recording`; switch to `fixing`) or, when recovery is manual, an instruction to quit and relaunch instead, and a dismiss button (`overlay_dismiss_incident`). Rust reveals a hidden overlay while an incident is active and logs `recording_incident_shown|recovered|dismissed|restart_clicked` locally. Startup progress phases shown: generic start-up, pipe loading, audio start-up, database update, engine start-up.
  - Live meeting preview: WebSocket `/ws/meeting-overlay` (auth token appended) streams `{type:"status", data:{active, …}}` and transcript items `{deviceType:"input"|"output", speakerName, text, capturedAt}`; the preview de-duplicates mic text that echoes speaker output (≥60 % word overlap within ±6 s for lines ≥24 chars); shows last speaker + text, pin/unpin transcript, and a button that ends the meeting.
  - Drag-to-anchor: drag the bar to snap to one of four edge-centre anchors on any display; persist with `set_shortcut_overlay_anchor(anchor, display)`.
  - Starred sessions button toggles the starred window; star badge while a session is active (`starred-session-state`).

### frontend-033 — Permission recovery window updates
- Kind: changed · Size: S · Confidence: observed
- Evidence: permission-recovery chunk outline (rows Microphone/Accessibility/Screen/Secure storage; `permission-recovery-restart-prompt`, `-restart-button`, `-all-fixed`, `-page`; `restart_after_screen_recording_permission`, `retry_screenpipe`, `enable_keychain_encryption`, `get_keychain_status`, `set_window_always_on_top_native`).
- Base touchpoints: `app/permission-recovery/page.tsx`, `src-tauri/src/permissions.rs`.
- Spec: add a secure-storage row (keychain encryption status; enable via `enable_keychain_encryption`); after screen-recording grant show a restart prompt that calls `restart_after_screen_recording_permission`; a retry-recording button calls `retry_screenpipe`; window closes automatically once everything is fixed. macOS-centric; on Windows only the secure-storage row is meaningful.

### frontend-034 — Onboarding v2
- Kind: changed · Size: L · Confidence: observed
- Evidence: `fe:_next/static/chunks/app/onboarding/page-960f31d90328bcc8.js` (`fe@…:106900-114500` step list and legacy map; acquisition options `fe@…:40774`; recommended-setup pipes `fe@…:98150`; trust disclosure test ids `onboarding-trust-*`, `onboarding-data-dir-*`; timeline step `onboarding_timeline_choice`; engine startup ids `onboarding-engine-startup`, `onboarding-startup-skip`; final `onboarding-final-setup`; first task (a search-style option vs a workflow-understanding option); a progress header showing the current step out of the total, with sub-progress).
- Base touchpoints: `app/onboarding/page.tsx`, `components/onboarding/*` (login-gate, permissions-step, engine-startup, connect-apps, pick-pipe), `lib/hooks/use-onboarding.tsx`, `src-tauri/src/commands.rs` (onboarding status/step).
- Spec:
  - Steps (persisted via `set_onboarding_step`, retried once after 300 ms): `login`, `acquisition`, `permissions`, `timeline`, `engine`, `plan`, `recommended-setup`, `first-task`. Filtering: `acquisition` skipped for managed deployments; `timeline` only for low-tier devices (`deviceTier === "low"`) unless screenshots/vision are policy-locked; `plan` only when a paid-trial flow applies (vendor; **omit in Freenpipe**); `recommended-setup` skipped when managed; `first-task` only when the Workflows rollout is on and not managed.
  - Legacy step mapping on resume: `connect-apps|integrations|connections|first-dashboard|pipe|encrypt|read|shortcut → engine`; `welcome|intro → login`; `usecases|status|setup → permissions`.
  - Login: optional for Freenpipe (base already has "skip"); a short trust note (data stays on this machine; capture can be paused at any time) expands to show the active data dir (`get_active_data_dir`) with an Open button (`reveal_in_default_browser`).
  - Acquisition (where the user heard about the product): options `search, friend, x, linkedin, reddit, hacker_news, youtube, github, newsletter, podcast, ai_assistant, ai_directory, other`, skippable; stores `acquisitionSource` — analytics only → **omit or keep local-only**.
  - Permissions: microphone, accessibility (live check), screen recording (restart required after grant); Windows shows only what applies.
  - Timeline (low-tier devices): choose whether to record screenshots → writes `disableScreenshots`.
  - Engine: base engine-startup plus compatibility notice when CPU lacks AVX2 (whisper unavailable, parakeet/cloud only), port-conflict and permission-reset troubleshooting, and an option to proceed with recording off.
  - Recommended setup: toggles for pipes `digital-clone` (store), `speaker-reconciliation` (bundled), `skill-learning` (bundled), `daily-email-summary` (store, needs Gmail connection); installs via `/pipes/bundled/<name>/install` or `/pipes/store/install`, then `/pipes/<name>/config` + `/pipes/<name>/enable`; per-call timeouts 20 s (store install) / 10 s; typed error codes `timeout, network, http_error, invalid_response, backend_error, verification_failed, model_unavailable, registry_*, pipe_not_found, installation_failed, permission_denied, disk_full, invalid_state, io_error, operation_failed, free_pipe_limit_reached`; deferring the setup is allowed.
  - First task: choose the search/recall path (chat) or the workflow path (Workflows) + optional free-text of a repeated task; sets the initial workspace.
  - Completion opens `{Home:{page:"brain"}}` (Library) and closes onboarding. Window sized once at mount (no per-slide sizes).
  - Language selector available on the login step (frontend-003).

### frontend-035 — Settings navigation v2
- Kind: changed · Size: M · Confidence: observed
- Evidence: `fe@…settings/page-…:659464-666926` (groups and ids), `fe@9668-…:8422` (ordered section list, aliases, default `recording`, localStorage `screenpipe:settings:last-section`), settings search (fuse.js threshold 0.4, `settings-search-input`, `settings-search-results`, `settings-search-result-*`, Ctrl/⌘K focus, matched-field scroll + 1.6 s highlight ring).
- Base touchpoints: `app/settings/page.tsx`, `components/settings/settings-search.tsx`, every `components/settings/*` `searchIndex`.
- Spec: groups — Capture & data: `recording` (Screen), `audio` (Audio & meetings), `speakers`, `storage` (Disk & retention), `privacy`, `permissions` (macOS only); AI: `activities`, `ai-settings` (AI features), `ai` (Models & keys), `usage` (AI credits); Account: `account`, `team`, `referral`; App: `general`, `display` (Appearance), `notifications`, `shortcuts`. Default section `recording`; remember the last section in localStorage `screenpipe:settings:last-section`. Enterprise-hidden sections redirect to the first visible one (`ai-settings` follows `ai`, `audio` follows `recording`). In Freenpipe hide `usage`, `team`, `referral` (cloud) or replace `usage` with local usage stats.

### frontend-036 — Screen settings (recording)
- Kind: changed · Size: M · Confidence: observed
- Evidence: settings outline lines ~810-838 and keymap (`disableVision` labelled as the screen-context capture switch, `disableScreenshots`, `useAllMonitors`, `monitorIds`, `videoQuality` low/balanced/high/max with widths 1280/1920/3840/native, capture frequency `idleCaptureIntervalMs` slider `capture-frequency-slider` ranging from an automatic power-profile stop to one capture every 10 s, `recordingDetail`, scroll capture with an automatic level plus three graded levels from lightest to most detailed (`captureScroll`), Power & battery `powerMode` auto/performance/battery_saver + `keepComputerAwake` via `set_keep_awake`, `/power`, thermal throttling notice, Chinese mirror `useChineseMirror`, apply/restart `recording-settings-apply-restart`); Rust `power_awake` (`str@0x49877d0`).
- Base touchpoints: `components/settings/recording-settings.tsx`, `components/settings/battery-saver-section.tsx`, `src-tauri/src/recording.rs`.
- Spec: split the base recording page into Screen vs Audio (frontend-037) via a `section` prop. Screen adds: separate toggles for screen context capture (`disableVision`) and screenshot images (`disableScreenshots`, keeps accessibility text); recording quality presets mapping to `videoQuality`; minimum capture interval slider bound to `idleCaptureIntervalMs` with an Auto (power-profile) stop; scroll capture intensity; Keep computer awake (`set_keep_awake(enabled)` + `keepComputerAwake`). A note tells users that right-clicking the Timeline nav item hides it (nav hide ≠ capture off).

### frontend-037 — Audio & meetings settings
- Kind: changed · Size: L · Confidence: observed
- Evidence: settings outline lines ~650-809 + keymap: `disableAudio`, `audioCaptureMode` (always/meetings-only), engine select (Cloud: screenpipe-cloud, Deepgram; Offline: Whisper Turbo / Turbo quantized / Tiny / Tiny fast, Qwen3-ASR experimental; Other: OpenAI Compatible; Disabled), `audio-engine-fallback-*`, OpenAI-compatible form with model listing from `/v1/models` or Ollama `/api/tags`, STT-only filter, raw WAV, custom headers JSON, **test-before-enable** `test_openai_compatible_transcription`, `batchMaxDurationSecs`, languages, AEC (`aecMode` off/screenpipe/macos/windows; remote safety disable notice), audio exclusions (`read_audio_exclusions`/`write_audio_exclusions`, Windows: one app), Meetings: live captions provider (`meetingLiveTranscriptionProvider`: selected-engine / screenpipe-cloud / deepgram-live / disabled), `appendTypedTextToMeetingNote`, auto-start meetings `disableMeetingDetector` + ignored apps picker (`meeting-apps-picker-*`, known apps Zoom/Teams/Meet/Discord/Webex/FaceTime/WhatsApp/Telegram/Signal), devices (`useSystemDefaultAudio`, `audioDevices`, `alwaysRecordBluetoothMic`), voice (`userName`, `train_voice`), vocabulary bulk paste (`vocabularyWords`), background transcription backlog (`/audio/reconciliation/backlog`, `/audio/retranscribe`, drop chunk), HD recording (`/capture/hd*`, `hdRecordingDefault`, `hdRecordingIntervalMs` 200/100/67/33 ms).
- Base touchpoints: `components/settings/recording-settings.tsx`, `components/settings/meeting-apps-picker.tsx`, `components/settings/hooks/use-transcription-diagnostics.ts`, `components/settings/hooks/use-voice-training.ts`, `src-tauri/src/audio_exclusions.rs`.
- Spec deltas: new `audioCaptureMode` select; OpenAI-compatible edits are not saved until `test_openai_compatible_transcription(endpoint, apiKey, model, headers, rawAudio)` succeeds (the probe transcribes a brief clip of silence); model list fetched from the endpoint with manual fallback; AEC mode select replaces the separate Windows/macOS AEC toggles; Windows audio exclusion limited to a single app; Bluetooth mic always-record toggle; vocabulary bulk paste (comma/newline/semicolon/tab separated) with dedupe and limit; backlog dialog listing waiting chunks (age, captured, file, status) with transcribe-now/drop and audio preview. Cloud live-caption providers are **decloud** (keep deepgram-live only with user key; hide screenpipe-cloud).

### frontend-038 — Privacy settings v2
- Kind: changed · Size: L · Confidence: observed
- Evidence: settings outline lines ~958-1216 + keymap (`privacy-tab-apps`, `privacy-tab-websites`, `privacy-category-switches`, `privacy-app-row`, `privacy-app-filter-list`, `privacy-allowlist-notice`, window rules `App::Title`, website allow/block with exact vs subdomains + exceptions (`ignoredUrls` objects, `includedUrls`), legacy broad match; capture rules: incognito (`ignoreIncognitoWindows`, `enhancedIncognitoDetection` via browser automation), DRM pause, clipboard, keyboard, click capture, record while locked, schedule; Data protection: PII mode Off/Basic/Smart (`usePiiRemoval`, `asyncPiiRedaction`, `asyncImagePiiRedaction`), apply to Text/Images, where it runs Local/Cloud(enclave) (`piiBackend` `local`/`tinfoil`), categories to redact (`piiRedactionLabels`: credentials and secret keys, identity-document numbers, personal names, email addresses, phone numbers, postal addresses, URLs carrying tokens, dates, health and financial data), additional capture fields to scan (`piiRedactionColumns`: values typed into form fields, URLs, labels on buttons and menus, tooltip/help text, links embedded in app data), consistent pseudonyms (`piiRedactionPseudonyms`), agent logs (`redactAgentSessionSecrets`); Security: API auth (`apiAuth`, `set_api_auth_key`, `regenerate_api_auth_key`, reveal/copy), LAN (`listenOnLan`), encryption at rest (`encryptStore`, keychain commands); Support access remote logs (`remoteLogCollectionEnabled`); telemetry (`analyticsEnabled`); admin team token (enterprise)).
- Base touchpoints: `components/settings/privacy-section.tsx`, `components/settings/window-picker.tsx`, `components/settings/schedule-settings.tsx`.
- Spec deltas: two tabs Apps / Websites; category switches add ordinary rules in bulk (partial state when some rules exist); allowlist notice when included lists are non-empty; website rules store `{domain, includeSubdomains, excludedSubdomains}` objects (strings still accepted); PII section rebuilt as Off/Basic/Smart with label/column pickers and a live preview card; click capture toggle (`disableClickCapture`, default on). Enforce `piiBackend="local"` in Freenpipe (**decloud** the enclave option) and default remote support logs off / hidden (**decloud**).

### frontend-039 — Disk & retention settings v2 (storage migration, retention modes)
- Kind: changed · Size: L · Confidence: observed
- Evidence: settings outline lines ~1247-1407 (`storage-migration-card`, migrate/resume/finish/cancel/delete-original; retention `/retention/status`, `/retention/configure`, `/retention/run`, `/data/storage-preview`, modes off/media/lean/all with confirm dialog `retention-mode-confirm-dialog`; erase recent `/data/delete-range` dialog `retention-recent-delete-dialog`; compact `/data/compact` dialog `retention-compact-dialog`; low-disk toggle `low-disk-recording-guard-toggle`; disk usage breakdown Data/Cache/Free/Media/Audio/Database/Logs/Scheduled tasks/Other; data dir change with `validate_data_dir`; cache list/delete `list_cache_files`/`delete_cache_files`); Rust `storage_migration` (`str@0x498edf0`).
- Base touchpoints: `components/settings/storage-section.tsx`, `components/settings/retention-settings.tsx`, `components/settings/disk-usage-section.tsx`, `src-tauri/src/disk_usage.rs`, new `src-tauri/src/storage_migration.rs`.
- Spec:
  - Storage migration card (when `can_migrate`): explain conversion to the compressed format; start (`start_storage_migration(root)`), progress (records converted, bytes saved, elapsed), cancel/use original (`cancel_storage_migration`), finish switching after verification, optional permanent delete of the original (`delete_original_storage_database(root, generation, true)` only when `can_delete_source`, with explicit confirm). The engine-side format is another dimension; the UI contract is Appendix A.
  - Retention policy radio: `off` keep everything; `media` drop video+audio keep text (recommended); `lean` media + drop per-element OCR/accessibility geometry, keep text/memories; `all` delete everything past cutoff — written as `localRetentionMode` (+ `localRetentionEnabled = mode != off`, `localRetentionDays`). Confirm dialog shows the storage preview of what will be removed.
  - Erase recent activity (last N minutes) via `/data/delete-range`; Reclaim disk space (`/data/compact`) with confirm; low-disk guard toggle (`stopRecordingOnLowDisk`).

### frontend-040 — Notifications settings v2
- Kind: changed · Size: M · Confidence: observed
- Evidence: `fe@…settings/page-…:578300-600290` (groups `recording, meetings, system, automation, app`; prefs `captureStalls` (experimental, default off, mirrors `showRestartNotifications`), `meetingReminders`, `meetingLiveNotes`, `audioCaptureStalled`, `liveTranscriptStalled`, `displayChanges`, `powerModeChanges`, …; master on/off, snooze 30 min/1 h/2 h/until tomorrow (`snoozeUntil`), quiet hours (`enabled, start, end` minutes), per-pipe mute (`mutedPipes`) and an always-notify (VIP) list, quick-set presets, reset, search, meeting reminder timing 15 s/30 s/1 min/2 min/5 min before); test ids `notification-snooze-*`, `notification-turn-off`, `notification-quiet-hours`, `notification-pipe-*`, `notification-pipe-vip-*`, `notification-preset-*`, `notification-reset`, `notification-search`, `notification-group-*`, `notification-pref-*`.
- Base touchpoints: `components/settings/notifications-settings.tsx`, `lib/utils/notification-toggle.ts`, `src-tauri/src/notifications/*`.
- Spec: extend `notificationPrefs` with `enabled` (master), `snoozeUntil` (ms epoch), `quietHours:{enabled,start,end}` (overnight ranges allowed), `alwaysNotifyPipes: string[]` (VIP; exact key name not observed — inferred), `meetingReminders`, `meetingReminderLeadSeconds` (inferred), `powerModeChanges`; keep base fields. Rust must apply master/snooze/quiet/VIP before showing toasts (frontend-031).

### frontend-041 — General, Appearance and Shortcuts settings deltas
- Kind: changed · Size: M · Confidence: observed
- Evidence: settings outline lines ~847-957, 304-347 and keymap: General — auto-start (`set_autostart`), search-after-quit (`keepSearchAvailableAfterQuit`), auto-update (temporarily required/locked), release channel Standard/Pre-release (`updateChannel` stable/beta), check now (`trigger_update_check`), auto-update scheduled tasks (`autoUpdatePipes`), version links, Your goal (`userGoalCategory`), reset onboarding (`show_onboarding_window`); Appearance — theme System/Light/Dark (`set_native_theme`), font size, overlay mode, chat always on top (`set_chat_always_on_top`), show chat suggestions (`showChatSuggestions`), hide overlay in screen recordings (`hideOverlayInScreenRecording` + `set_app_screen_capture_protection`, `hide-overlay-in-screen-recording-toggle`), translucent sidebar (mac), hide dock icon (`hideDockIcon`, mac), minimize to tray (Windows), shortcut reminder on/off + size; Shortcuts — global list incl. star session, conflict detection, restore defaults, `suspend/resume_global_shortcuts` while recording a combo.
- Base touchpoints: `components/settings/general-settings.tsx`, `components/settings/display-section.tsx`, `components/settings/shortcut-section.tsx`, `components/settings/shortcut-row.tsx`, `src-tauri/src/shortcuts.rs`.
- Spec: add the new toggles listed with their keys; register `starSessionShortcut` as a global shortcut handled in Rust (arm `star_session`) that emits `starred-session-toggle`/toggles the starred window. `keepSearchAvailableAfterQuit` behaviour: on Quit, stop capture and close windows but leave the engine running so external tools can query history (Rust `search_only` module, `str@0x4987800`). Auto-update lock and version download links are vendor-specific → keep auto-update user-controllable and point to Freenpipe releases.

### frontend-042 — AI features & Models & keys
- Kind: changed · Size: M · Confidence: observed
- Evidence: `section-settings-ai-settings` (Enhanced AI `set_enhanced_ai_suggestions`, cloud media analysis `set_cloud_media_analysis_skill` with `cloud-media-analysis-preview`, auto-generate chat titles); models & keys (providers Screenpipe cloud, ChatGPT sign-in, Claude API, Gemini (`https://generativelanguage.googleapis.com/v1beta/openai`, strips `models/` prefix), Ollama (`/api/tags`, `/api/show`; recommended small local models listed), API key/custom URL, ACP command; connection test required before save with stages auth/models/chat; advanced limits (model context tokens, max output tokens); share preset to team (cloud)).
- Base touchpoints: `components/settings/ai-presets.tsx`, `components/settings/hooks/use-openai-models.ts`, `lib/utils/chatgpt-preset.ts`.
- Spec: split AI settings into "AI features" (toggles) and "Models & keys" (presets). Add a Gemini provider shortcut (OpenAI-compatible URL above). Require a passing connection test (auth → list models → chat) before a preset can be created/updated; show per-stage failure. Hide screenpipe-cloud provider and cloud media analysis in Freenpipe (**decloud**) unless the user supplies their own endpoint.

### frontend-043 — Permissions settings section (macOS)
- Kind: new · Size: S · Confidence: observed
- Evidence: `section-settings-permissions`, `permissions-required`, `permissions-optional`, `permission-row-*`, `permission-granted-*`, `permission-manage-*`, `permission-restart-*`, `permission-enable-*`, `permission-reset-input-monitoring`; rows Screen Recording, Microphone, Accessibility (required), Input Monitoring, Calendar (optional).
- Base touchpoints: new `components/settings/permissions-section.tsx`, `components/settings/input-monitoring-card.tsx`.
- Spec: macOS-only section listing permission states with Enable/Manage/Restart actions; hidden on Windows (filter in nav and search index).

### frontend-044 — Global storage-migration & database-recovery overlays
- Kind: new · Size: M · Confidence: observed
- Evidence: root layout `fe@app/layout-…:3082-10356` (`storage-migration-prompt`), `fe@…:132989-137682` (`storage-migration-progress`, a modal for storage recovery / recording restoration), deep links `screenpipe://database-recovery`, `screenpipe://database-restart-verification` (`fe@…:67950`); event `storage-migration-activity`; Rust `db_recovery_notifications`, `db_relaunch`, `db_self_heal` (`str@0x514cfc3`, `str@0x5a61758`, `str@0x49ee5ae`).
- Base touchpoints: `app/layout.tsx`, `app/providers.tsx`, `src-tauri/src/main.rs`.
- Spec: when `get_storage_migration_status` reports a pending/available migration, show a one-time prompt (start immediately, postpone, or stay on the original database). While a migration or recovery is busy, every main window shows a progress modal (records converted, bytes saved, elapsed; a reminder not to quit; the computer is kept awake) that can be hidden but not dismissed; during that time capture and the history views are offline, while settings/updates remain usable. Database-recovery notifications deep-link into `start_database_recovery` / `restart_database_verification`.

### frontend-045 — Deep-link hosts
- Kind: changed · Size: S · Confidence: observed
- Evidence: root layout host switch: `activity, artifact, auth, chat, database-recovery, database-restart-verification, first-run-agent, first-run-summary, frame, home, install-pipe, live-view, meeting, oauth, pipe, purchase-successful, settings, subscription-success, timeline, view`; other chunks: `calendar-connections`, `scheduled`, `workflows`.
- Base touchpoints: `components/deeplink-handler.tsx`, `src-tauri/src/main.rs` (deep-link registration).
- Spec: add handlers for `activity` (open Activity, optional range), `artifact` (open artifact in Library/viewer), `chat` (open/prefill chat), `database-recovery`, `database-restart-verification`, `first-run-agent` (`?target=` agent hand-off), `first-run-summary`, `live-view/<id>`, `scheduled` (Automations), `workflows` (`?reviewRequest=` deduped via sessionStorage `screenpipe:last-workflow-review-request`), `calendar-connections`. `purchase-successful`/`subscription-success` are vendor billing → ignore.

### frontend-046 — Feedback & logs submission
- Kind: decloud · Size: M · Confidence: observed
- Evidence: `7737-fae548fffc70b86b.js` (`start_feedback_upload(request)`, `feedback-upload-completed`, `read_log_tail(path, maxBytes)`, `redact_pii_for_feedback(text, settingsJson)`); Rust `feedback_upload`, `feedback_redact`, `diagnostic_logs` (`str@0x5a63148`, `str@0x498c4f0`, `str@0x498c3a0`); help section log-sending entry (`section-help`).
- Base touchpoints: `components/feedback-dialog.tsx`, `components/share-logs-button.tsx`, `lib/stores/feedback-store.ts`.
- Spec: the feedback dialog collects text, optional screenshot and log tails, PII-redacts text with the user's privacy settings (`redact_pii_for_feedback`) and uploads to the vendor in the background. Freenpipe: keep redaction and log tail, but write a local zip bundle (or open a prefilled GitHub issue) instead of uploading; emit `feedback-upload-completed` with the bundle path.

### frontend-047 — Workflows workspace
- Kind: new · Size: XL · Confidence: observed
- Evidence: `755-b4c485e5a5bfbc14.js`, `4656-37ad501298baf0a7.js`, `3768-…`, `9783-…`, CSS modules `workflow-editor_*`, `workflow-agent-actions_*`, `workflow-question_*`, `workflow-library_*`, `sop-document_*`, `workflow-assistant_*`; local routes `/workflows/rollout`, `/workflows/catalog`, `/workflows/workspace`, `/workflows/pipeline`, `/workflows/context`, `/workflows/edits`, `/workflows/corrections`, `/workflows/:id` (`str@0x5257ca8`), `/workflow-videos`, `/workflow-voice`, `/pi-workflows-guide/video-projects`; commands A.1 (Workflows); events `workflow-review-ready`, `workflow-video-<id>`; Rust `workflows_runtime`, `workflow_video`, `workflows_media` (`str@0x5174fe9`, `str@0x498bcc8`, `str@0x498cf98`); cloud `/api/sops`, `/api/sops/access`, `/api/trajectories`, `/api/enterprise/member-workflows`.
- Base touchpoints: new `app/(main)/home` workspace mode or new route, new `components/workflows/*`, new Rust modules; pipes `pi-workflows-*` (engine/pipes dimension).
- Spec (UI contract):
  - Workspace switcher at the head of the Home sidebar choosing between the chat workspace and the Workflows workspace; onboarding first-task can choose Workflows.
  - Workflows home: catalog of discovered repeated workflows (from history; workflow discovery runs in background via `analyze_workflows(days, profile)` and agent pipes); a background-maintenance card enabling four background agent tasks (discover, investigate evidence, review drafts, maintain accuracy) with enable/disable-all.
  - Workflow editor: title, timing (estimated elapsed incl. pauses), description, trigger, ordered steps (drag; Alt+Up/Down), blocks per step (text/heading/media caption), outcome, captured moments with local recording replay (`load_workflow_recording(timestamp, appName)`, `release_workflow_recording`, `open_workflow_captured_moment(frameId, timestamp)`), voice notes that fill answers (`/workflow-voice`), questions to confirm, undo, autosave with a conflict path that drops the local draft and loads the newest version, corrections/edits posted to `/workflows/corrections` / `/workflows/edits`.
  - SOP document: guide summary, before-you-start, steps with screenshots (screenshot picker), expected result, exceptions, check result, still-to-confirm; export as self-contained HTML; generate skill (`generate_workflow_skill(workflow, profile)` → `save_workflow_skill(draft)`); narrated video (`create_workflow_video(id, scenes)`, progress `workflow-video-<id>`, cancel/discard, download captions; last three previews kept).
  - Library of saved SOPs and skills; workflow assistant chat (floating or sidebar) scoped to the current page.
  - The Tinfoil enclave attestation UI, the opt-in contribution sharing back to the vendor (`redact_workflow_contribution` + `/api/trajectories`) and the web-hosted SOP link (`/api/sops`) are vendor-cloud → **decloud**: omit or replace with local export.
- Notes: large feature; the engine pipes/prompts belong to other dimensions. LLM prompt text must be re-authored.

### frontend-048 — Skills, external-agent integration and provider automations
- Kind: new · Size: L · Confidence: observed
- Evidence: `5977-…:149435-174849` (`starter-skills-card`, `provider-skill-catalog`, `organization-skills`; `fetch_skills_registry`, `install_registry_skill`, `list_imported_skills`, `scan_device_skills`, `import_skill`, `remove_imported_skill`); `9665-…` (AI tool targets `claude, codex, cursor, gemini, openclaw, hermes`; `install_external_agent_skills(target)`, `remove_external_agent_skills(target)`, `set_ai_tool_auto_connect_opt_out(target, optOut)`, `resolve_ai_tool_config_path(path)`, `bun_check`, `grokbot_connection(action)`; skill catalog of 20 workflow skills (descriptions e.g. bug report from recording, commitments tracker, customer context, decision trace, incident timeline, meeting prep/follow-up, SOP from procedure, handoff, worklog, research synthesis…)); provider automations `5977-…:530471-534872` (`provider-automations-panel`, `provider-heading`); skills registry source `str@0x4995d13`/`str@0x4995df0` (GitHub tree API, size/file-count limits, bundled catalog fallback).
- Base touchpoints: `components/settings/skills-card.tsx`, `components/settings/agent-card.tsx`, `components/settings/hermes-card.tsx`, `components/settings/openclaw-card.tsx`, `components/settings/connections-section.tsx`, `src-tauri/src/skills.rs`.
- Spec:
  - Skills browser: Recommended (featured, ranked by recent app usage keywords), provider catalogs (`source` e.g. anthropic/openai repos), organization skills (enterprise; omit). Install downloads a GitHub folder containing `SKILL.md` into the skills store with limits on file count/size and rate-limit handling; installed skills appear in the composer slash menu.
  - External agents: one card per target agent; "connect" writes screenpipe MCP config + skill files into that agent's config (paths resolved via `resolve_ai_tool_config_path`), "disconnect" removes them; auto-connect opt-out per target. Grok bot connection status `{detected, connected, optedOut}`.
  - Provider automations panel: scheduled tasks that live inside external agents (e.g. Codex), listed read-only or manageable with pause/resume/delete via `manage_provider_automation(key, action)`; schedule shown as RRULE/cron with a label.
- Notes: the remote registry URL points at the vendor's GitHub repo → **decloud**: ship a bundled catalog and allow a user-configurable registry URL (default to the Freenpipe repo). Skill catalog descriptions must be re-written.

### frontend-049 — Automations (pipes) section deltas
- Kind: changed · Size: L · Confidence: observed
- Evidence: `5977-…:479455-705291` (`/pipes/schedule/preview`, `pipe-preset-chain`, `pipe-preset-chain-row-*`, `pipe-effort-select-*`, `pipe-run-destination-*`, `pipe-chat-destination-trigger`, `pipe-connection-add`, `pipe-connection-option-*`, `pipe-history-error-*`, `pipe-history-reset-*`, `pipe-create-pane`, `pipe-advanced-tab-*`, `/pipes/<name>/history`, `/pipes/store/check-updates|update|auto-update`, `/pipes/favorites`, `/api/pipes/share`), `cloud-agent-*` (`/cloud-agents/*`, `/api/enterprise/cloud-runner`), continuous-chat state `pipe-continuous-chat-state` (one chat vs separate chats per run).
- Base touchpoints: `components/settings/pipes-section.tsx`, `components/pipe-store.tsx`, `lib/hooks/use-pipes.ts`, `lib/hooks/use-running-pipes.ts`.
- Spec deltas: schedule preview (next runs) when editing a cron; preset chain (ordered AI presets used as fallbacks); effort level per pipe; run destination (where results go: chat, notification, connection); per-pipe connections; history with reset (clears pipe memory/one-chat context) and error rows; single-conversation mode where future runs and replies share a conversation; auto-update store pipes. Cloud agent runner (Cursor background agents etc.) and pipe sharing are vendor-cloud → **decloud**.

### frontend-050 — Connections section deltas
- Kind: changed · Size: M · Confidence: observed
- Evidence: `5977-…` (WhatsApp `/connections/whatsapp/status|pair|disconnect`; Bee `/connections/bee/pair/start|poll`; Obsidian instances `/connections/obsidian/instances|test`; IMAP `/connections/imap`, `/connections/imap/test`, `/connections/imap/mailboxes`; Google Docs proxy; MCP registry `/mcp-servers/registry`, `/mcp-servers/<id>/tools`, `/mcp-servers/test`, `/oauth/start|status|disconnect`; Composio `/api/composio/*`; user browser status; Windows shell targets `open_windows_shell_target`; `remote_sync_*`); onboarding Gmail/Calendar connect via `oauth_connect(..., variant)`.
- Base touchpoints: `components/settings/connections-section.tsx`, `components/settings/custom-mcp-card.tsx`, `lib/constants/connections.ts`.
- Spec: add cards for WhatsApp pairing, Bee pairing (start + poll), Obsidian multiple vaults, IMAP mail with test + mailbox picker; MCP server registry browser with tool listing and test; `oauth_connect` gains a `variant` argument (e.g. read-only vs full scopes — inferred). Composio-backed connectors are vendor-cloud → **decloud**.

### frontend-051 — Vendor commercial/cloud surfaces to remove or stub
- Kind: decloud · Size: L · Confidence: observed
- Evidence: trial activation & paywall (`trial-activation-*`, `/onboarding/checkout`, `/onboarding/checkout/start`), card ask (`card-ask-*`, `/business-trial/checkout`, `/api/subscription/checkout`), business upgrade (`business-upgrade-card`, `/api/subscription/offer`, `/api/subscription/change-plan`, `/api/billing`), annual plan banner, plan expiration notice, free-plan wall/counter, AI credits/usage limits (`hosted-usage-limits`, `usage-limits-panel`, `/usage`), team seat invite (`/api/team/billing/*`), referral (`/api/referral`, `/api/referral/invite`), entitlement gate (`9092-…`), announcements system (`announcement-*`, localStorage `screenpipe-announcement-preview`, `screenpipe-announcements-dismissed-v1`), data sync & cloud MCP (`/api/user/data-sync*`, `/ingest`, `/devices`, `/sync/reset-account`), Workflows sharing, remote support logs (`remote_support_logs`), remote control policy (`remoteControlPolicy`, `desktop_remote_control_applied`), AI gateway (`get_screenpipe_ai_gateway_url`), PostHog/Sentry, help survey link, session-expired login prompts.
- Base touchpoints: `components/app-entitlement-gate.tsx`, `components/login-dialog.tsx`, `components/settings/account-section.tsx`, `components/settings/usage-section.tsx`, `components/settings/team-section.tsx`, `components/settings/referral-card.tsx`, `lib/app-entitlement.ts`, `app/layout.tsx`.
- Spec: none of these ship in Freenpipe. Keep layout slots empty; ensure no code path calls vendor hosts. Where a target feature is gated on login/subscription (daily summary, dictation, live captions, cloud media), gate instead on "a local or BYO-key provider is configured". Remote control policy: treat every `forceDisabled` as false and every `defaultEnabled` as the local default.

### frontend-052 — Enterprise-only UI commands
- Kind: ee-stub · Size: M · Confidence: observed
- Evidence: `961-2df1b4b3553b61d1.js` (`get_enterprise_install_metadata`, `get_enterprise_host_identity`, `/api/enterprise/heartbeat`, `/api/enterprise/policy`, `set_enterprise_recording_authorized`, `sync_managed_team_skills`, `set_enterprise_policy`, `apply_enterprise_ui_visibility`, `set_sync_streams`, `set_activity_sync_enabled`, `save_enterprise_team_config(…, gatewayUrl)`, `save_enterprise_license_key`); `9092-…` enterprise key activation (`ENT-XXXX-XXXX-XXXX-XXXX`); `is_history_access_restricted` (`67-…`); `list_managed_team_skills`.
- Base touchpoints: `components/enterprise-license-prompt.tsx`, `components/enterprise-locked-setting.tsx`, `lib/hooks/use-enterprise-policy.ts`, `src-tauri/src/enterprise_policy.rs` (base already stubs `ee/`).
- Spec: register the commands so bindings compile, returning inert values: `apply_enterprise_ui_visibility → false`, `get_enterprise_host_identity → {machine_id_hash:null, os_user_id_hash:null}`, `is_history_access_restricted → false`, `list_managed_team_skills → []`, others `Ok(())`. `is_enterprise_build_cmd` stays false.

### frontend-053 — Settings store defaults and new keys (JS side)
- Kind: changed · Size: M · Confidence: observed
- Evidence: Appendix C; `fe@378-…:15705`; serde order `str@0x52c8878`; zod schema `fe@7451-…:56355`.
- Base touchpoints: `lib/hooks/use-settings.tsx` (`DEFAULT_SETTINGS`, `createDefaultSettingsObject`, `Settings` type), `src-tauri/src/store.rs` (Rust `SettingsStore`), `lib/utils/tauri.ts` (regenerate).
- Spec: add every key in Appendix C with the listed default; `createDefaultSettingsObject(isPro=false)` returns `disableClipboardCapture=!isPro`, `disableKeyboardCapture=!isPro`, `asyncPiiRedaction=isPro`, `asyncImagePiiRedaction=isPro`, `piiBackend=isPro?"tinfoil":"local"` — Freenpipe always uses the non-pro branch (`local`). Platform overrides unchanged plus `starSessionShortcut` (Windows `Alt+Shift+B`). Migration: fill missing array-typed keys from defaults (target `V()` does this for any array default), require ≥1 AI preset. Changed defaults: `usePiiRemoval=true`, `autoUpdate=true`, `filterMusic=true`, `experimentalCoreaudioSystemAudio=true` (macOS only effect).
- Notes: Rust struct must deserialize unknown/missing keys leniently (serde defaults) so screenpipe-written `store.bin` files load.

### frontend-054 — Removed UI pieces
- Kind: removed · Size: S · Confidence: observed
- Evidence: A.2; settings nav lacks base "Get free month" label and Display label; pipe-suggestion settings commands gone; base localStorage `screenpipe-ui-theme`, `screenpipe:upcoming-collapsed`, `screenpipe_claude_connected`, `console_logs` unused.
- Base touchpoints: `components/settings/*` (pipe suggestions UI in notifications/general), `src-tauri/src/pipe_suggestions_scheduler.rs`, `lib/utils/tauri.ts`.
- Spec: drop the periodic "pipe suggestions" notification settings (commands `pipe_suggestions_get_settings/update_settings` and keys `pipeSuggestionsEnabled`, `pipeSuggestionFrequencyHours` become unused; keep reading them harmlessly). Theme now persists only in the settings store (`uiTheme`) + `set_native_theme`.

### frontend-055 — Help section refresh
- Kind: changed · Size: S · Confidence: observed
- Evidence: home `fe@…:84484-93896` (`section-help`, send logs, `help-getting-started` tutorial video, `help-survey-link` `/survey`, docs, videos, ideas `/ideas`, GitHub issues, `help-discord-link`, changelog).
- Base touchpoints: `app/home/page.tsx` (help case), `components/share-logs-button.tsx`.
- Spec: keep a Help panel with: send logs (local bundle, frontend-046), getting started, docs, issues, changelog, keyboard shortcuts, rerun first-run guide. Point every link at Freenpipe resources (vendor survey/Discord/ideas links → **decloud**).

### frontend-056 — Tauri command & event surface (bindings regeneration)
- Kind: changed · Size: XL · Confidence: observed
- Evidence: Appendix A/B; bindings chunk `fe:_next/static/chunks/8001-00ed120106af2449.js`; ACL list `str@0x5177a68`; handler arg-name strings `str@0x4993348`, `str@0x4993580`, `str@0x4993f18`; typed channel names `str@0x5179541`.
- Base touchpoints: `src-tauri/src/main.rs` (`generate_handler!`/specta collector), `src-tauri/src/specta_bindings.rs`, `src-tauri/capabilities/*.json` (ACL), `lib/utils/tauri.ts` (regenerate with `bun run bindings:generate`), `lib/events/types.ts`.
- Spec: add the 112 commands of A.1 (each with `#[tauri::command]` **and** `#[specta::specta]`, camelCase args as listed), apply the 7 argument changes of A.3 as optional trailing params (so old callers still work), keep or drop the 8 removed ones (A.2). Commands that are mac-only (`native_timeline_*`, `set_history_swipe_navigation_enabled`, `restart_after_screen_recording_permission`) or enterprise/cloud (A.1 last group) compile everywhere and return inert values on Windows/Freenpipe. Add the new events of Appendix B to the typed event registry, including channels `job:event`, `export:event`, `engine:event`, `notification:action`.
- Notes: `bun run bindings:check` must pass; the ACL capability file must list every new command or invocations fail silently at runtime.

---

## 4. Suggested implementation order (Windows-first)

1. frontend-053 (settings keys) + frontend-056 (command/event stubs) — unblocks everything.
2. Small native/UI wins: 005, 007, 008, 029 (starred sessions; needs engine `/starred-sessions` + `/export`), 030, 031, 032, 041.
3. Settings v2: 035–040, 042; onboarding 034 (without plan/acquisition).
4. Chat: 009, 011, 012, 013, 014, 010; then ACP 015–017.
5. Library/Activity/Meetings: 021, 023, 024, 025, 027, 028; Live Views 022.
6. Workflows 047, skills/integrations 048–050, localization 003.
7. Throughout: 051/052 decloud & ee stubs; never ship vendor endpoints.

## 5. Open questions

- Exact low-disk guard threshold returned by `get_low_disk_guard_config` (UI only compares `thresholdBytes`).
- `owned-browser:state` payload fields and `notificationPrefs` key names for VIP pipes / reminder lead time are inferred from UI behaviour, not observed literals.
- Live View dashboard limit and on-disk storage path for brain views/canvases were not observed in UI chunks (Rust `brain_views` module owns them).
- `timelineOrigin` value set for `open_search_window`/`search_navigate_to_timeline` (seen values: `home`, `chat` in recent-chat handoff; overlay inferred).
- Whether `recording-health-state` reason strings are localized on the Rust side (overlay shows them raw).
- Which of the 20 skill-catalog entries and template kits Freenpipe should ship (content must be re-authored).

---

## Verification

Adversarial pass over frontend-001 … frontend-056. Method:
- **(a) Base check.** Every claimed identifier was checked against the BASE (`apps/screenpipe-app-tauri` app/components/lib/src-tauri plus `crates/`) with `tools\vfe_bulk.py` (out: `tools\out\vfe2_bulk.tsv`, 245 identifiers). Each partial overlap was then read in the base source.
- **(b) Evidence check.**
  - Every cited `str@` offset was re-resolved with `tools\vfe_offchk.py` (`out\vfe2_offchk.txt`; all 54 land on the expected Rust module, route or event line).
  - Cited `fe@chunk:offset` anchors were checked with the new `tools\vfe2_fechk.py` (`out\vfe2_fechk.txt`, 63 checks).
  - The binding diff was re-run with `tools\tauri_cmds.py` (`out\vfe2_cmds.json`). It reproduces target 310 / base 206 / new 112 / removed 8 / arg-changes 7. All 112 new commands appear in this spec (`tools\vfe2_specmiss.py`).
- **Independent sweeps.**
  - Events: `tools\vfe2_events.py`. Every app-level Tauri event that is new versus base is in Appendix B; the rest are library internals.
  - Storage keys and window labels: `tools\vfe2_lskeys.py`. All new keys are listed.
  - New test ids: also swept. They map onto existing items, apart from the deep-link gap below.
- **REA.**
  - Summary view `ev_d205fc1fda693bff953c410e8912bcbd3a6e1ff1e195914f6446ed80da215f84` (parent `ev_57f23857…6216d`) shows 599 of 1446 nodes examined.
  - The trace `ev_b4269cde65b35a4e8a765865520538c914da8f161d389baa72949e5e6bb558ca` (channel seed `storage-migration-activity`) returns no-match on a partial graph. This confirms that direct chunk reading, not the REA graph, is the right evidence route for these literals.

### Verdicts

| Item | Verdict | Note (what differs / what was checked) |
|---|---|---|
| 001 | confirmed | `15.5.21` in main chunk.<br>`19.2.0-canary` in `4bd1b696` (pages framework still 18.3.1).<br>React Flow `rf__*` ids in `1a258343`.<br>`mermaid.min.86edda15.js` is a static asset.<br>General Translation runtime in `3188`/`6139`.<br>Space Grotesk ships as a real `@font-face` (`4fcea525…css`). Base only names it as an unloaded fallback in `components/onboarding/login-gate.tsx:284`.<br>Base: next 15.1.4, react ^18.3.1, mermaid ^11.12.3 (dynamic import). |
| 002 | partially_in_base | Already in base:<br>- base settings page already renders the shared `AppSidebar`, including the "Resize sidebar" handle (`app-sidebar.tsx`).<br>- base home already aliases `feedback→help` and `disk-usage/cloud-archive/cloud-sync→storage`.<br>- base settings already redirects `connections`.<br>New: the `(main)` route-group layout, `main-window-drag-surface`, the `memories→brain` alias and `?from=` back-nav.<br>Correction: when `from` is present the target calls `router.back()`. The `home:<id>` form only pre-stores `pending-chat-conversation`. Base simply pushes `/home`. |
| 003 | confirmed | `uiLocale`/`uiLocalizationEnabled` (serde `str@0x52c8878`), both localStorage keys and `desktop-localization` are absent from base. |
| 004 | confirmed | `NEXT_PUBLIC_SCREENPIPE_WEB_DEV/_API_PORT/_SCENARIO` and `mockLocale` are in the root layout. Base has no browser mock (no `mockIPC`/OS-plugin shim). Dev-only. |
| 005 | partially_in_base | Base `app/layout.tsx` already has the exception-safe `unregisterListener` wrapper, `ensureWebviewFocus` on focus, and the IndexedDB-lost reload.<br>New: the 1 s `webview_renderer_heartbeat` with in-flight guard (`fe@layout:140350`), global `contextmenu` suppression, and `setHistorySwipeNavigationEnabled(false)`. |
| 006 | partially_in_base | Base pipes nav already shows a running-pipe count badge (`PipeActivityIndicator`), and the meeting button has a pulse dot. Base section hiding exists only through enterprise policy.<br>New: `sidebarNavLayout` order/hidden with legacy-order migration, drag reorder, per-item and sidebar option menus, Chat/Library/Automations labels, and the `activity`/`brain` sections. |
| 007 | partially_in_base | Base sidebar device row already pauses and resumes audio device groups via `/audio/device/stop|start`, with optimistic rollback (`app/home/page.tsx:~1030`).<br>New: status popover, per-monitor `/vision/device/stop|start`, pause-all, `is_capture_paused`, and the low-disk guard (`get_low_disk_guard_config`, `stopRecordingOnLowDisk`). |
| 008 | partially_in_base | Base already binds Ctrl/Cmd+N new chat and Ctrl/Cmd+B toggle sidebar (`app/home/page.tsx:445-460`). Base also has Ctrl+Tab hold-to-cycle recent chats (`components/chat/recent-chat-switcher-controller.tsx:109`).<br>New: cmdk command palette, keybinding registry, `close_tab`/`archive_chat`/`rename_chat`/`pin_chat`/`branch_chat`/`toggle_right_sidebar`, and the shortcut-guide modal. |
| 009 | partially_in_base | Base `chat-sidebar.tsx` already has Pinned/Recents/Archived, Pin/Rename/Archive(+Undo)/Delete and pipe-run rows. Base persists `pinned`/`hidden`; `lastViewedAt` exists in base `chat-store.ts`.<br>New: tab strip, split pane, side chat, branch, user groups, filters/sort/bulk ops, persisted `lastContentAt`, and `list_chat_entries_by_mtime`/`get_chats_dir`. |
| 010 | confirmed | `.claude/projects` / `.codex/sessions` import is at `fe@home:595727/596407`. `importedFrom` is absent from base. Base `.codex` hits are MCP-config paths only. |
| 011 | confirmed | Slash registry (`requiresEmptyComposer`, `/inspector`) at starred-sessions chunk ~3838–4161. Dictation (`composer-dictation-*`, `X-Transcription-Provider`) is present. None of these are in base; base `@` tokens are acknowledged by the spec. |
| 012 | partially_in_base | Base `standalone-chat.tsx` already has typed `ContentBlock` (text/tool/thinking, :623), collapsible grouping of consecutive tool blocks (:1732), loader phases analyzing/thinking/tool/streaming (:857) and `source-citation-footer`.<br>New: chart, plan, structured-output, rich-result, ask-user, agent-action and context-usage blocks; the turn-status v2; sandboxed HTML; selected-text actions. |
| 013 | partially_in_base | Base `components/browser-sidebar.tsx` already listens to `owned-browser:navigate|state|session-access-request|v20-cookie-blocked`. It renders the Chrome v20/locked cookie card and the session-access prompt. Base already has the single-browser commands `owned_browser_navigate/set_bounds/hide/clear_browsing_data/resolve_session_access`.<br>New: per-tab `owned_browser_tab_*`, `owned_browser_history`, `navigate(owner,reveal)`, the right-panel tab strip, and the Files & outputs and Connections tabs.<br>Two of the item's listed ids are base events. |
| 014 | confirmed | `producer_ref` POST `/feedback` at `fe@3822:9097`. No `/feedback` route exists in base crates, and `notification-feedback` is new. |
| 015 | confirmed | Every `pi_acp_*`, `pi_set_thinking_level` and `pi_start_and_prompt` command is absent from base. Base `AIProviderType` has no `acp`. The catalog is embedded at `str@0xa0c9c05`. |
| 016 | confirmed | The four commands are absent from base. The mandatory-subagents string is at `str@0x4992868`; the cited `0x4992810` is the adjacent extension-UI-response line. |
| 017 | confirmed | `coding_workspace_*` and `src\coding_workspace.rs` (`str@0x498dfd9`). Base has no worktree code. |
| 018 | confirmed | `first-run-summary-state`, `firstRunSummaryPhase`, `src\first_run_summary.rs`. All absent from base. |
| 019 | confirmed | `firstrun-scrim` at `fe@home:718625`, plus `firstRunGuideDone` and `first-run-guide-pending`. All absent from base. |
| 020 | confirmed | Base summary cards and suggestions exist (acknowledged by the spec). `home-card-agent-actions`, `home-contextual-suggestion` and `showChatSuggestions` are new. |
| 021 | partially_in_base | Base `components/settings/memories-section.tsx` already lists, searches, adds, edits and deletes memories via `/memories` as the "Memories" nav section.<br>New: the Library wrapper with view switcher, filters, the Artifacts browser (`/artifacts`) and SAF frames. |
| 022 | confirmed | All seven `*brain_view*` commands, `live-view-canvas.v1` (`str@0x5283f82`) and `src\brain_views.rs` are absent from base. |
| 023 | confirmed | `/activity-ledger` (`str@0x498d6d0`), `activity_no_data`, the activity commands and settings, and `src\activity_history.rs`. All absent from base. |
| 024 | partially_in_base | Base `components/meeting-notes` already has:<br>- the replay strip with scrubber and a jump into the timeline (`replay-strip.tsx`)<br>- the 15-min inactivity stop prompt (`note-view.tsx:1392`)<br>- editor image drop (`note-editor.tsx:190`)<br>- retranscribe and coming-up calendar<br>- a **synchronous** mp4 export via `export_recording` (`src-tauri/src/meeting_export.rs`)<br>New: replay player controls (`replay-speed`, audio), streaming summary with `/meetings/:id/summary-status`, meeting chat panel, `copy_rich_text_to_clipboard` share, async `start_export_recording` + `export:event`, slash menu/format toolbar, and the join suggestion/calendar nudge. |
| 025 | partially_in_base | Base timeline already has Domains and Calls filter rows (`components/rewind/timeline/timeline.tsx:1183,1266`). Base also has a daily-summary panel (`components/rewind/daily-summary.tsx`, Apple-Intelligence, auto plus manual).<br>New: selection video export POST `/export`, the paused-screenshot placeholder, `timeline-*` command events, and an on-demand summary with an enable dialog. |
| 026 | confirmed | `native_timeline_*` and the `native-timeline-*` events are absent from base. |
| 027 | partially_in_base | Base `components/rewind/search-modal.tsx:1688-1691` already has the All / Screen / Keyboard & Clipboard / Chats tabs, plus tag (`#`) and people (`@`) search.<br>New: recent-activity suggestion chips (`fe@3308:71532`), the `timelineOrigin` arg, and `search-hidden`/`search-reset`. |
| 028 | partially_in_base | Base `app/viewer/page.tsx` already uses `read_viewer_file` with `text`/`error`/`image` kinds, Reveal, Copy and Copy path.<br>New: HTML rendered/source toggle in a no-network sandbox, the unsafe-file guard and diagnostics listing the searched locations. |
| 029 | confirmed | `starred_sessions` table (`str@0x516d018`), `/starred-sessions` doc (`str@0x511e39f`), the events (`str@0x524e740`), `star_session` arm (`str@0x5150568`) and the page chunk. All absent from base. |
| 030 | partially_in_base | Base `components/notification-bell.tsx` popover already polls `/notifications` every 5 s. It already has item, dismiss, expanded, ask-AI, clear-all and manage-settings test ids, plus DELETE.<br>New: the separate `notification-inbox` window via `show_notification_inbox`, Priority/All views, per-item POST `/read`, typed actions, copy/source, feedback and the unread dot. |
| 031 | partially_in_base | Base `app/notification-panel/page.tsx` already has an open-with-default-app action and a restart action via `stopScreenpipe`/`spawnScreenpipe`.<br>New: toast feedback, the jump to the originating chat, Rust master/snooze/quiet/cooldown suppression (`str@0x499a02f`), and the meeting toast inside the overlay (`str@0x4999f98`). |
| 032 | partially_in_base | Base `app/shortcut-reminder/page.tsx:146` already scales small/medium/large as 1/1.5/2, and already uses `/ws/meeting-status`.<br>New: recording-health incidents, the `/ws/meeting-overlay` live transcript, anchors, 1 h snooze and the star button. |
| 033 | partially_in_base | Base `app/permission-recovery/page.tsx` already has the keychain row (`getKeychainStatus`/`enableKeychainEncryption`), auto-close/restart when fixed, and `permission-recovery-all-fixed`.<br>New: the restart prompt (`restart_after_screen_recording_permission`) and `retry_screenpipe`. |
| 034 | confirmed | Base has five slides `login/permissions/engine/connect-apps/pipe` with its own legacy map. The target's eight steps, acquisition options (`fe@onboarding:39872`) and recommended pipes (`:97817`) are verified. |
| 035 | partially_in_base | Base `components/settings/settings-search.tsx` already uses Fuse.js with `scrollToSettingsField`.<br>New: grouped nav, the `audio`/`activities`/`ai-settings`/`permissions` ids, default `recording`, and `screenpipe:settings:last-section`. |
| 036 | confirmed | Base has `disableVision`, the `videoQuality` select and `powerMode` (battery-saver).<br>`disableScreenshots`, `recordingDetail`, `capture-frequency-slider`, `keepComputerAwake` and `set_keep_awake` are new.<br>Offset note: the keep-awake control sits at `fe@settings:246210-249343` (power block), not in the cited 355053-366000 range. |
| 037 | partially_in_base | Base `components/settings/recording-settings.tsx` already has:<br>- vocabulary bulk import (`TranscriptionDictionary`, `showBulk`, :1216)<br>- the backlog list via `/audio/reconciliation/backlog` (:429)<br>- HD recording, audio exclusions and `meetingLiveTranscriptionProvider` (:2925)<br>- separate Windows/macOS AEC switches<br>New: `audioCaptureMode`, the unified `aecMode` select, the `test_openai_compatible_transcription` gate, `alwaysRecordBluetoothMic` and the Windows single-app exclusion. |
| 038 | partially_in_base | Base `components/settings/privacy-section.tsx` already has PII Off/Basic/Smart (`PiiMode`, :431), `piiRedactionLabels`, `piiBackend` local/tinfoil, API-key regenerate (:832), LAN, `encryptStore`, incognito, `ignoredUrls` and analytics.<br>New: Apps/Websites tabs, category switches, domain-object rules and `includedUrls`, `piiRedactionColumns`/`piiRedactionPseudonyms`, agent-log redaction, click capture, enhanced incognito and remote-log toggle. |
| 039 | partially_in_base | Base `components/settings/retention-settings.tsx` already has modes `media`/`all`, `/retention/status|configure|run`, `/data/storage-preview` and erasing the most recent activity via `/data/delete-range`.<br>New: the `off`/`lean` modes, `/data/compact`, the storage-migration card and the low-disk toggle. |
| 040 | confirmed | Base has `captureStalls` and `mutedPipes`; the spec keeps them as base fields. Master switch, snooze, quiet hours, groups, VIP, lead time, presets and search are new (`fe@settings:581325`). |
| 041 | confirmed | Base `display-section.tsx` already has a `chatAlwaysOnTop` toggle; what is new is the live `set_chat_always_on_top` command. Base `use-settings.tsx` already has the `updateChannel` key but no UI. `keepSearchAvailableAfterQuit`, `hideDockIcon`, `userGoalCategory`, `set_app_screen_capture_protection` and `starSessionShortcut` are new. |
| 042 | partially_in_base | Base `components/settings/ai-presets.tsx` already has the `maxTokens` slider with presets and `maxContextChars`. Base already has the `set_enhanced_ai_suggestions` and `set_cloud_media_analysis_skill` commands.<br>New: the separate AI-features section, the Gemini OpenAI-compatible preset (`fe@settings:141212`) and the mandatory connection test. |
| 043 | confirmed | `section-settings-permissions` (`fe@settings:526910`) and `permission-reset-input-monitoring` are new. Base only has `input-monitoring-card.tsx`. |
| 044 | confirmed | `storage-migration-prompt|progress`, `storage-migration-activity` (`str@0x5a63db9`) and the recovery commands are absent from base. |
| 045 | confirmed | New hosts verified in the root layout (`activity`, `live-view`, `chat`, `artifact`, `first-run-*`, `database-*`). Base handles only `subscription-success`, `auth`, `timeline`, `install-pipe`, `view`, `frame` and `meeting`. However, the handler list misses the `pipe` host and the versioned external-link contract; see frontend-M01. |
| 046 | confirmed | The commands and the `feedback-upload-completed` event are new. Note that base `components/share-logs-button.tsx` already uploads logs, a screenshot and the last 5 min of video to the vendor from JS, so this decloud debt already exists in base. |
| 047 | confirmed | `/workflows/*` routes (`str@0x5257ca8`), the workflow commands and `src\workflows_runtime.rs` are absent from base. |
| 048 | confirmed | Base has `import_skill`/`list_imported_skills`/`remove_imported_skill`/`scan_device_skills` and agent/hermes/openclaw cards with copy-paste instructions. The registry, auto-install into external agents, provider automations and grokbot are new. |
| 049 | partially_in_base | Base already checks, updates and auto-updates store pipes (`/pipes/store/check-updates|update|auto-update`, `autoUpdatePipes` in `pipes-section.tsx:930-943`).<br>New: `/pipes/schedule/preview` (`fe@5977:479463`), preset chain (`:525012`), effort, run/chat destinations, per-pipe connections, history reset and one-chat mode. |
| 050 | partially_in_base | Base `components/settings/connections-section.tsx` already has WhatsApp pairing (`/connections/whatsapp/status|pair|disconnect`) and multiple Obsidian vaults (`/connections/obsidian/instances|test`).<br>New: Bee pairing, IMAP, the MCP registry browser and the `oauth_connect` `variant` arg. |
| 051 | partially_in_base | Base already ships the referral card, team section, usage section, entitlement gate, login dialog, sync settings and PostHog/Sentry.<br>New target surfaces: trial activation/paywall, card ask, business upgrade, annual-plan banner, plan-expiry notice, announcements, remote-control policy, AI gateway, remote support logs and data-sync v2. |
| 052 | confirmed | All listed enterprise commands are absent from base `tauri.ts`. Base only has `enterprise_policy.rs`/`enterprise_sync.rs` stubs. |
| 053 | confirmed | Target defaults are `usePiiRemoval:!0` (`fe@378:16043`), `filterMusic:!0` and `autoUpdate:!0`. Base has `false` at `use-settings.tsx:525/546/596`. The new keys were confirmed via serde order (`str@0x52c8878`). |
| 054 | unsupported | Only partly true. The removal of `pipe_suggestions_*` and `e2e_*` commands is confirmed: zero hits in the frontend or exe strings.<br>But `screenpipe-ui-theme` is **still** the ThemeProvider `storageKey` (`fe@layout:1572`, `:90150`) and is read by the inline boot script in all 17 HTML pages. `console_logs` is still read from localStorage in 9 chunks (feedback/log share).<br>Strike both keys from the "removed" lists in this item and in Appendix C. |
| 055 | partially_in_base | Base `components/settings/feedback-section.tsx` already has `section-help`, `help-getting-started`, ideas, GitHub, Discord (`help-discord-link`) and changelog links.<br>New: the survey link, the keyboard-shortcuts entry and "rerun first-run guide". |
| 056 | confirmed | Re-run reproduces 310 / 206 / 112 / 8 / 7 exactly. All 112 new commands are named in this spec. The typed channels are at `str@0x5179541`. |

No duplicates found. 021/022, 006/008, 046/055 and 053/056 overlap in scope but describe distinct deltas.

### Missed items

**frontend-M01 — Versioned external deep links (`chat/new`, `pipe/<name>`)** · Size S · Kind new
- Evidence:
  - `fe@_next/static/chunks/5977-dd591e0cc8c27cfa.js:376015` (validator plus `external_deeplink_opened` analytics, to be dropped).
  - `fe@app/layout-3ec03e167c097e07.js:75010`: the `pipe` host routes to that validator; `:74397` handles the `chat` host.
  - Base `components/deeplink-handler.tsx` has neither.
- Behaviour (paraphrased):
  - The app accepts `screenpipe://chat/new?v=1&prompt=<text>` and `screenpipe://pipe/<name>?v=1`. The same links are also accepted on the `screenpipe-enterprise:` scheme.
  - A link is rejected as unsupported (no-op) when it:
    - carries a username, password or port;
    - has a fragment;
    - has any query parameter beyond the allowed set;
    - has `v` other than `1`.
  - A chat prompt must be non-empty after trimming, pass a control-character filter, and be at most 4096 UTF-8 bytes. It opens the Home chat prefilled, **without auto-send**, tagged as source `external-deeplink`.
  - A pipe name must pass the pipe-name validator, and then opens that pipe.
- Why it matters: this is a public interop contract for links from docs and other apps. Item 045 lists `chat` only as "open/prefill" and omits `pipe` and the `v=1` validation rules.

