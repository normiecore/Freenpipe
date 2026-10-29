# Shell dimension spec: Tauri shell, config, CLI, env vars, settings store, Windows specifics

Target: screenpipe v2.7.104 (Windows build, `<screenpipe-install>/screenpipe-app.exe`, Tauri 2.11.2).
Base: Freenpipe @ upstream 892199f (app crate version 2.5.28), `<repo>`.

Clean-room note. Everything below is a paraphrase of static evidence (string table offsets in
`app_strings.tsv`, frontend assets under `<re-workspace>/frontend/`, REA evidence). Identifiers (setting keys,
commands, events, env vars, file names, routes, registry paths, CLI flags) are quoted exactly for
interoperability. UI copy, prompts and JS bodies are summarised, never copied. The frontend
`CHANGELOG.md` shipped inside the bundle was deliberately NOT read (post-relicense changelog, see CLAUDE.md).

Offsets: `0x...` values are hex file offsets of the matching string inside `app_strings.tsv`
(the exe). Frontend paths are relative to `<re-workspace>/frontend/`.

REA evidence used:
- `ev_57f23857f31e9207a9f71e1d612412e78f9c5dc0d29af8c68f97372786b6216d` parent JS application analysis (frontend).
- `ev_d205fc1fda693bff953c410e8912bcbd3a6e1ff1e195914f6446ed80da215f84` summary view (394 JS files parsed, 2112 modules, coverage partial).
- `ev_8b32a918ea06290774201890397e899e56822883eafcffb970b4499a2063f04e` module trace of `_next/static/chunks/378-9f286ebe7c0f0713.js` (settings store + defaults module): imported by 15 renderer pages, i.e. every window shares one settings model.
- `ev_726906e03c96fa69a5c86e375c5dd06b9079ae2d24db619b1ed8dd974ed6f8b9` module trace of `_next/static/chunks/8001-00ed120106af2449.js` (generated Tauri command bindings): imported by the same 15 renderer pages.
- `ev_0e38935e25b3fa505f0d401808c5862722a2a9c2a1edb848c3286bc9389db3c2`, `ev_4a9af8bbd257e2fd96938359dd6ac3a083b02abe9121abaef0388559d201a901` string/route seed traces (no graph match; string evidence came from direct file inspection instead).

Helper scripts written for this pass (all in `<re-workspace>/tools/`, prefixed `shell_`): `shell_sgrep.py`,
`shell_sline.py`, `shell_near.py`, `shell_modules.py`, `shell_uniq.py`, `shell_srcfiles.py`,
`shell_cmds.py`, `shell_ctx.py`, `shell_cli_docs.py`, `shell_keydiff.py`, `shell_u16.py`.

---

## 0. Key structural facts

1. The target exe does not contain clap (no `clap` crate in `crates.txt`, no clap help text). The CLI
   (`screenpipe` binary) is shipped separately (npm `screenpipe@latest`, launched via `bun x`). Target
   CLI evidence therefore comes only from the CLI skill document embedded in the exe
   (0x6a57a13..0x6a5a94b and a duplicate at 0xa07d7e0..) and from log strings. Flags with defaults
   cannot be fully recovered; see shell-026.
2. App-crate module list recovered from tracing callsites `screenpipe_app::<module>` (see inventory
   table). New shell-relevant modules: `headless`, `search_only`, `process_exit`, `overlay_health`,
   `port_conflict`, `db_relaunch`, `db_recovery_notifications`, `db_self_heal`, `recording::db_wedge`,
   `recording::retry`, `recording::recovery_log`, `recording::server_shutdown`, `storage_migration`,
   `disk_pressure_notifications`, `update_diagnostics`, `update_restart`, `startup_auth`,
   `remote_support_logs`, `feedback_upload`, `feedback_redact`, `native_overlay_win`, `power_awake`,
   `localization`, `commands::overlay_anchor`, `commands::native_actions`, `commands::starred_sessions`,
   `enterprise::policy`, `enterprise::host_identity`, `enterprise_persistence`, `first_run_summary`.
3. New workspace crates seen in panic paths: `screenpipe-overlay-win` (render.rs, window.rs),
   `screenpipe-resource` (background_work.rs, cpu.rs), `screenpipe-sqlite-coordinator`,
   `screenpipe-sqlite-recovery`, `screenpipe-telemetry-wire`, `screenpipe-semantic`, `sonora*` (AEC).
4. Plugin set is unchanged in kind: opener, deep-link, http, os, updater, dialog, store, shell,
   notification, fs, permission-flow, process, autostart, single-instance, global-shortcut, cli,
   sentry (JS command allowlist dump at 0x5a12052; crate versions in `crates.txt`:
   tauri 2.11.2, plugin-updater 2.11.0, plugin-deep-link 2.4.9, plugin-store 2.4.4,
   plugin-global-shortcut 2.3.2, plugin-opener 2.5.4).

---

## 1. Base inventory (Freenpipe @ 892199f)

| Area | Base implementation |
|---|---|
| Tauri conf | `apps/screenpipe-app-tauri/src-tauri/tauri.conf.json` (+ `.prod/.beta/.windows/.enterprise/.linux/.macos/.e2e`). identifier `screenpi.pe` (prod), `screenpi.pe.beta`, `screenpi.pe.dev`; deep-link schemes `screenpipe` / `screenpipe-beta`; tray id `screenpipe_main`, icon `assets/screenpipe-logo-tray-white.png`, `menuOnLeftClick: true`; asset scope `$APPDATA/**`, `$HOME/.screenpipe/**`; CSP null; updater endpoint `https://screenpi.pe/api/app-update/{stable|beta}/{{target}}-{{arch}}/{{current_version}}` + minisign pubkey |
| Capabilities | `capabilities/main.json` (opener allowlist: http(s), mailto, tel, x-apple-reminderkit, obsidian) |
| Windows bundle | `tauri.windows.conf.json`: NSIS only, externalBin `bun`, resources ffmpeg/ffprobe/dlls, onnxruntime.dll, openblas, vcredist, assets; `windows/hooks.nsh` (pre-install taskkill + kill processes under $INSTDIR; post-uninstall removes `$LOCALAPPDATA\screenpipe\git-portable`) |
| Cargo features | `directml`, `vulkan`, `qwen3-asr`, `parakeet`, `redact-onnx-*`, `official-build`, `enterprise-build`, `e2e` |
| main.rs | db-lock refusal (`.db_recovery.lock`, `SCREENPIPE_IGNORE_DB_LOCK`), Windows CA bundle, single-instance pre-check via POST `/focus` on `SCREENPIPE_FOCUS_PORT` (11435), Sentry + PostHog (embedded keys), panic hook writing `last-panic.log` (rotated to `.prev`), `setx OLLAMA_ORIGINS *`, plugin init, CloseRequested (Windows: Home minimizes or hides to tray per `minimizeToTrayOnClose`), daily rolling log `screenpipe-app.<date>.log` max 5 files, `TESSDATA_PREFIX`, store init with defaults fallback, E2E seeds (`SCREENPIPE_E2E_SEED`), model pre-download, server+capture on dedicated runtime, tray, autostart enable/disable from `autoStartEnabled`, health check, engine events, shortcuts, schedulers |
| Settings | `src/store.rs` `SettingsStore` (flattened `screenpipe_config::RecordingSettings` + app keys + `extra`), `store.bin` under `~/.screenpipe`, optional encryption (`SPSTORE1`, `.encrypt-store`, `SCREENPIPE_ENCRYPT_STORE`), last-good snapshot/restore; frontend defaults in `lib/hooks/use-settings.tsx` |
| Shortcuts | `src/shortcuts.rs`: show, start/stop recording, start/stop audio, show_chat, search, lock_vault; suspend/resume; Escape window shortcut with generation counter |
| Tray | `src/tray.rs`: open_app, show_chat, show_search, show (timeline), status, monitor rows (read-only), audio device toggles, fix_permissions, plan_info/upgrade, update item, version, toggle_recording, Pause for (5/15/30/60), Record HD timers/stop/extend, settings, quit; recreate_tray (macOS) |
| Windows | `src/window/show.rs` labels `main`, `home` (alias `settings`), `search`, `onboarding`, `chat`, `permission-recovery`; plus `notification-panel`, `shortcut-reminder`, `overlay`, viewer |
| Health | `src/health.rs`: boot phase snapshot, recording status, sidecar health poll, tray icon health |
| Logs | `src/log_files.rs` lists `*.log` in data dir |
| Updater | `src/updates.rs`: periodic check, rollback endpoint, auth_required, what's-new notification |
| Control server | `src/server.rs` on 11435: `/notify`, `/notifications`, `/notifications/:id`, `/inbox`, `/log`, `/auth`, `/app-icon`, `/installed-apps`, `/window-size`, `/focus`; CORS `*` |
| Config crate | `crates/screenpipe-config` (`RecordingSettings`, `config.toml` persistence) |
| Engine CLI | `crates/screenpipe-engine/src/cli/*` (record, status, search, team, pipe, audio, vision, sync, mcp, connection, vault, install, login, logout, whoami, survey, doctor, auth, db, backup, export) |

## 2. Target inventory (v2.7.104), shell-relevant

| Area | Target evidence |
|---|---|
| Compiled conf | 0x5a0e038..0x5a0f360: `screenpipe`, asset scope `$APPDATA/**`,`$HOME/.screenpipe/**`, tray `screenpipe_main` + `assets/screenpipe-logo-tray-white.png`, devUrl `http://localhost:3000/`, icons incl. `icons/screenpipe-logo-tray-failed.png`, `icons/screenpipe-logo-tray-black.png`, mobile path `/purchase-successful`, opener regex now also allowing `claude://claude.ai/new?q=`, `cursor://anysphere.cursor-deeplink/prompt?text=`, `codex://threads/new?prompt=`, updater endpoint `https://screenpipe.com/api/app-update/stable/{{target}}-{{arch}}/{{current_version}}` with the same minisign pubkey as base |
| Install layout | `%LOCALAPPDATA%\screenpipe\`: `screenpipe-app.exe`, `bun.exe`, `ffmpeg.exe`, `ffprobe.exe`, `av*-62.dll`/`avfilter-11.dll`/`swresample-6.dll`/`swscale-9.dll` (+ .def/.lib/.dll.a), `onnxruntime.dll`, `onnxruntime_providers_shared.dll`, `DirectML.dll`, `vulkan-1.dll`, `VulkanRT-License.txt`, `libopenblas.dll`, `msvcp140*.dll`, `vcruntime140*.dll`, `screenpipe_wer_dump_helper.dll`, `uninstall.exe`, `assets\` (dmg-background.png, nsis-header.bmp, nsis-sidebar.bmp, 8 tray PNGs), runtime dirs `frames\cache_index.bin`, `models\` (segmentation-3.0.onnx, wespeaker_en_voxceleb_CAM++.onnx), `vad\silero_vad_v5.onnx` |
| Settings | full serde field list at 0x52c8878 (app + recording); defaults object in `_next/static/chunks/378-9f286ebe7c0f0713.js` |
| Commands | 310 commands in `_next/static/chunks/8001-00ed120106af2449.js`; Rust-side sorted list at 0x5177a68..0x5178fd4 |
| CLI | doc-only (skill markdown at 0x6a57a13+) |
| Env vars | see shell-025 |

## 3. Base-vs-target inventory table

| Feature | Base | Target | Delta id |
|---|---|---|---|
| Settings key set | 32 app keys + RecordingSettings | +~45 keys, 1 removed, several default changes | shell-001 |
| Settings migrations | restartNotifications, coreaudioTap V2, presets seeding, parakeet | + coreaudioTapMigrationV3, filterMusicDefaultedOn, provider/id rewrites, model rewrites, enterprise managed overlay, new-user privacy defaults | shell-002 |
| Cloud token storage | `settings.user.token` in store.bin | token stripped from store.bin, mirrored to secret store key `cloud.auth_token`; `get_cloud_token`; store load via `get_screenpipe_base_dir`; startup aborts on store init failure | shell-003 |
| Headless UI | none | `headless`, `headlessRecordOnly`, dormant UI, tray wake | shell-004 |
| Search after quit | none | `keepSearchAvailableAfterQuit` (default true), search-only mode | shell-005 |
| Low disk guard | none | `stopRecordingOnLowDisk` (default on), engine `disk_pressure`, notifications, `get_low_disk_guard_config` | shell-006 |
| Keep awake | none | `keepComputerAwake`, `set_keep_awake`, engine `SetThreadExecutionState` | shell-007 |
| Screen-share protection | overlay-only display affinity via `showOverlayInScreenRecording` | + `hideOverlayInScreenRecording`, `hideAppInScreenShare` (all windows), get/set commands | shell-008 |
| Shortcut overlay | size + show flag | + star-session shortcut, snooze, anchor, display, allow-hiding | shell-009 |
| Native Win32 overlay pill | macOS-only native reminder | Windows native pill (`screenpipe-overlay-win`) | shell-010 |
| Recording health/incidents | boot phase only | `recording-health-state`, incident UX, restart/dismiss | shell-011 |
| Engine watchdog | none (status only) | auto-respawn, port-conflict classification | shell-012 |
| Relaunch/exit | ad hoc | `process_exit` safe relaunch, pre-exit teardown budget, relaunch env | shell-013 |
| DB self-heal UX | CLI-only recovery | notifications, deep links, commands, recovery log | shell-014 |
| Storage migration | none | commands/events/files | shell-015 |
| Updater | screenpi.pe, stable/beta (TS only) | screenpipe.com, `stable`/`pre-release` used by Rust, attempt marker + logs, `restart_for_update`, autoUpdate default true | shell-016 |
| Crash dumps | `last-panic.log` | + WER runtime exception helper DLL + minidump harvest into `crash-dumps` | shell-017 |
| WebView watchdog | ProcessFailed handler | + 1 s renderer heartbeat, swipe navigation toggle | shell-018 |
| Focus/control server | `/focus {args, deep_link_url}`, CORS * | + `target`, `launch_exe`, `launchd_job_label`; exe/scheme checks; non-local Origin/Host rejected | shell-019 |
| Autostart | plugin, no args | `--autostart` arg, background launch, dedupe, dev isolation | shell-020 |
| Startup env | E2E seeds | `SCREENPIPE_SKIP_ONBOARDING`, `SCREENPIPE_DATA_DIR`, dynamic scope grant | shell-021 |
| CPU compat | none | AVX2 probe, compatibility mode | shell-022 |
| Tray | see base | vision toggles, schedule state, richer status, first-summary, Help, i18n templates | shell-023 |
| Multicall argv | `--check-arc-automation` (mac) | `--screenpipe-acp-runtime`, `--screenpipe-acp-process-guard`, `--screenpipe-portable-mcp` | shell-024 |
| Env vars | ~90 | +~40 | shell-025 |
| CLI | see base | `service`, `diagnose`, richer `status`, `agent setup` | shell-026 |
| Deep links | 7 hosts | 21 hosts | shell-027 |
| Opener allowlist | +obsidian | +claude/cursor/codex schemes | shell-028 |
| Install layout | ffmpeg, onnxruntime, openblas, vcredist, bun | + DirectML.dll, vulkan-1.dll, VulkanRT license, WER helper DLL, providers_shared | shell-029 |
| DirectML selection | feature flag only | DXGI adapter probe + `SCREENPIPE_DIRECTML` kill switch | shell-030 |
| Telemetry | Sentry + PostHog embedded | + piggyback telemetry, telemetry-wire device id, env kill switch | shell-031 (decloud) |
| Remote support logs | none | consent sync + polling vendor | shell-032 (decloud) |
| Feedback upload | none | vendor upload of redacted logs/screens | shell-033 (decloud) |
| Startup entitlement gate | `app_entitled_or_dev` | account refresh at startup, trial read-only server | shell-034 (decloud) |
| Enterprise | `enterprise_policy.rs` etc. | registry license, host identity, persistent updater, UI modes | shell-035 (ee-stub) |
| Localization | none | `uiLocale`, `uiLocalizationEnabled`, native localization manifest | shell-036 |
| hideThinkingBlocks | present | removed | shell-037 |
| Data-dir files | base set | new diagnostic/state files | shell-038 |
| Windows inventory | 6 core labels + panels | + `notification-inbox`, `starred-sessions`; headless/enterprise suppression; `set_chat_always_on_top` | shell-039 |

---

## 4. Delta items

### shell-001 Settings store schema (changed)

Evidence: serde field-name block 0x52c8878 (app keys then RecordingSettings keys, declaration order);
second block 0x52aeeda (specta/TS type view, includes `enterpriseManaged`, `uiLocale`, `uiLocalizationEnabled`);
0x5a42f8d confirms `apiKey` still exists between `apiAuth` and `listenOnLan`; defaults object `F` in
`_next/static/chunks/378-9f286ebe7c0f0713.js` (around char 15420..18700) and platform overrides in its
function `z`; REA `ev_8b32a918...`. Confidence: observed (keys), observed (defaults where present in `F`),
inferred (Rust-side defaults for keys not in `F`).

Storage: unchanged container. `~/.screenpipe/store.bin` (or the dir from `get_screenpipe_base_dir`),
tauri-plugin-store JSON, top-level keys `settings` and `onboarding` (+ others unchanged). All keys below
live in the `settings` object, camelCase.

Field order in the target struct (app part): `aiPresets, isLoading, uiLocale, uiLocalizationEnabled, devMode,
ocrEngine, dataDir, embeddedLLM, autoStartEnabled, platform, disabledShortcuts, user,
showScreenpipeShortcut, startRecordingShortcut, stopRecordingShortcut, startAudioShortcut,
stopAudioShortcut, showChatShortcut, searchShortcut, lockVaultShortcut, starSessionShortcut,
shortcutOverlaySize, showShortcutOverlay, shortcutOverlaySnoozedUntil, allowHidingShortcutOverlay,
shortcutOverlayAnchor, shortcutOverlayDisplay, deviceId, autoUpdate, updateChannel, autoUpdatePipes,
enhancedAI, remoteLogCollectionEnabled, remoteLogCollectionUserId, overlayMode,
showOverlayInScreenRecording, hideOverlayInScreenRecording, hideAppInScreenShare, chatAlwaysOnTop,
showRestartNotifications, stopRecordingOnLowDisk, translucentSidebar, uiTheme, minimizeToTrayOnClose,
headless, headlessRecordOnly, keepSearchAvailableAfterQuit` then the flattened recording keys.

Recording part (target, in order): `disableAudio, audioTranscriptionEngine, transcriptionMode,
audioCaptureMode, meetingLiveTranscriptionEnabled, meetingLiveTranscriptionProvider, audioDevices,
useSystemDefaultAudio, experimentalCoreaudioSystemAudio, experimentalMeetingPiggyback,
alwaysRecordBluetoothMic, windowsInputAecEnabled, macosInputVpioEnabled, screenpipeAecEnabled, aecMode,
audioChunkDuration, deepgramApiKey, filterMusic, batchMaxDurationSecs, vocabularyWords, disableVision,
disableScreenshots, enableSemanticContext, semanticContextMode, disableTimeline, monitorIds,
useAllMonitors, videoQuality, captureMaxWidth, maxSnapshotWidth, disableSnapshotCompaction,
disableMeetingDetector, ignoredMeetingApps, idleCaptureIntervalMs, visualCheckIntervalMs,
visualChangeThreshold, minCaptureIntervalMs, hdRecordingDefault, hdRecordingIntervalMs,
captureOnKeystroke, captureOnClipboard, captureScroll, prioritizeInputLatency, extractionThreadPriority,
pauseExtractionOnInputMs, ignoredWindows, includedWindows, ignoredUrls, includedUrls,
ignoreIncognitoWindows, enhancedIncognitoDetection, pauseOnDrmContent, disableClipboardCapture,
disableKeyboardCapture, disableClickCapture, recordWhileLocked, languages, usePiiRemoval,
asyncPiiRedaction, redactAgentSessionSecrets, asyncImagePiiRedaction, piiBackend, piiRedactionLabels,
piiRedactionColumns, piiRedactionPseudonyms, userId, userName, openaiCompatibleEndpoint,
openaiCompatibleApiKey, openaiCompatibleModel, openaiCompatibleHeaders, openaiCompatibleRawAudio, port,
powerMode, recordingDetail, keepComputerAwake, useChineseMirror, analyticsEnabled, analyticsId,
enableWorkflowEvents, deviceTier, scheduleEnabled, scheduleRules, apiAuth, apiKey, listenOnLan`.

New keys, their type and target default (frontend `F` unless noted). Keys owned by other dimensions are
listed so the struct is complete; the owner dimension specifies behaviour.

| Key | Type | Target default | Owner / notes |
|---|---|---|---|
| `uiLocale` | string | `"system"` | shell-036 |
| `uiLocalizationEnabled` | bool | false in production (only true in a dev mock mode) | shell-036 |
| `starSessionShortcut` | string | Windows `Alt+Shift+B`; other `Control+Super+B` | shell-009 |
| `shortcutOverlaySnoozedUntil` | string or null (timestamp) | null | shell-009 |
| `allowHidingShortcutOverlay` | bool | false | shell-009 |
| `shortcutOverlayAnchor` | string enum `top-center` / `right-center` / `bottom-center` / `left-center` | unset, UI treats as `top-center` | shell-009 |
| `shortcutOverlayDisplay` | string (display identity) | unset | shell-009, written by Rust only |
| `updateChannel` | `"stable"` / `"pre-release"` | `"stable"` | shell-016 (base TS had `"stable"|"beta"`, Rust ignored it) |
| `remoteLogCollectionEnabled` | bool | true in target; Freenpipe must default false | shell-032 |
| `remoteLogCollectionUserId` | string or null | null | shell-032 |
| `hideOverlayInScreenRecording` | bool | false | shell-008 |
| `hideAppInScreenShare` | bool | true | shell-008 |
| `stopRecordingOnLowDisk` | bool | true (UI treats missing as on) | shell-006 |
| `headless` | bool | false | shell-004 |
| `headlessRecordOnly` | bool | false | shell-004 |
| `keepSearchAvailableAfterQuit` | bool | true | shell-005 |
| `keepComputerAwake` | bool | false | shell-007 |
| `audioCaptureMode` | `"meetings-only"` / `"always"` | `"meetings-only"` | audio dim (0x517a06d lists both values) |
| `experimentalMeetingPiggyback` | bool | feature-flag default (smart recording) | audio dim |
| `alwaysRecordBluetoothMic` | bool | false | audio dim |
| `screenpipeAecEnabled` | bool | false | audio dim |
| `aecMode` | string | `"off"` | audio dim |
| `disableScreenshots` | bool | false | capture dim |
| `enableSemanticContext` | bool | true | semantic dim |
| `semanticContextMode` | string | `"memory"` | semantic dim |
| `captureMaxWidth` | number | not in `F` (inferred unset) | capture dim |
| `includedUrls` | string[] | [] | capture dim |
| `enhancedIncognitoDetection` | bool | false | capture dim |
| `disableClickCapture` | bool | false | capture dim |
| `redactAgentSessionSecrets` | bool | not in `F` | privacy dim |
| `piiRedactionColumns`, `piiRedactionPseudonyms` | arrays/bool | not in `F` | privacy dim |
| `recordingDetail` | string | `"auto"` | capture dim |
| Frontend-only (land in `extra`): `dataSyncEnabled` false, `activitiesEnabled` false, `activitiesIntervalMinutes` 15, `userGoalCategory`, `remoteControlPreferences`, `remoteControlPolicy`, `firstRunGuideDone` false, `sidebarNavLayout`, `enterpriseManagedSettings`, `workflowSharing`, `pendingStorageMigration` (Rust-written, 0x5a644c8), `browserCookieAccessGranted` (base already) | various | as stated | other dims |

User object: target adds `user.enterprise_account` (null default) to the base field list
(0x52d73ea lists the full refresh field set: `email image clerk_id api_key credits stripe_connected
stripe_account_status github_username bio website contact cloud_subscribed credits_balance app_entitled
enterprise_account`). AIPreset gains `enterpriseManaged` (0x52aeeda). Default preset changed: id
`screenpipe` (anonymous) or the pair `chat` + `pipes` for subscribed users, provider `screenpipe-cloud`,
`url` empty, `model` `auto`, `maxContextChars` 200000, empty prompt.

Changed defaults vs base: `usePiiRemoval` false -> true; `autoUpdate` false -> true;
`filterMusic` false -> true (also forced once by migration); `experimentalCoreaudioSystemAudio`
false -> true (macOS only, forced by V3 migration); `deviceId` default now empty string and filled by the
loader; new-user privacy defaults depend on an install predicate (see shell-002).

Removed: `hideThinkingBlocks` (shell-037).

Implementation notes:
- Rust: add fields to `SettingsStore` (`src/store.rs`) with `#[serde(rename=...)]` and defaults listed;
  add recording fields to `screenpipe_config::RecordingSettings` (only the ones whose owner dimension
  implements them, but keep unknown keys round-tripping through `extra`; the flatten-`extra` safety net
  already exists).
- `apiKey` must stay readable; target hides it from the TS type (specta skip) but keeps the serde key.
- TS: extend `Settings` type and `DEFAULT_SETTINGS` in `lib/hooks/use-settings.tsx`; widen
  `UpdateChannel` to `"stable" | "pre-release"` and map legacy `"beta"` to `"pre-release"`.
- Freenpipe policy overrides: `remoteLogCollectionEnabled` false, `analyticsEnabled` false,
  `autoUpdate` follows shell-016 policy, default AI preset must not point at the vendor cloud
  (provider owned by the AI dimension).
- Regenerate bindings (`bun run bindings:generate`).

### shell-002 Settings load migrations and sentinel keys (changed)

Evidence: `378-9f286ebe7c0f0713.js` loader (functions after `z`). Confidence: observed.

Loader behaviour in target, in order (paraphrased):
1. Resolve store path via command `get_screenpipe_base_dir` (fallback `<home>/.screenpipe`), open
   `store.bin` with autosave off.
2. If no `settings`, return platform defaults. New-install privacy defaults are computed by a predicate
   (an async check the frontend calls before building defaults; its meaning is not visible, likely
   "eligible for cloud redaction"): when the predicate is true, `disableClipboardCapture` and
   `disableKeyboardCapture` become false, `asyncPiiRedaction` and `asyncImagePiiRedaction` true, and
   `piiBackend` `"tinfoil"`; otherwise the inverse with `piiBackend` `"local"`. The same values back-fill
   these five keys when missing on an existing store. Freenpipe: always use the false/`local` branch.
3. Hydrate `user.token` from the secret store when missing (shell-003).
4. Back-fill any missing array-valued default keys; guarantee the preset list is non-empty (a save
   with an empty preset list fails).
5. Normalise `userGoalCategory` (seed one if invalid).
6. `deviceId` generated if empty.
7. Sentinels (each runs once, then the sentinel key is persisted true):
   `restartNotificationsDefaultedOff` (forces `showRestartNotifications` false, base had it),
   `coreaudioTapMigrationV2` (forces tap off, base had it), `coreaudioTapMigrationV3` (NEW: forces
   `experimentalCoreaudioSystemAudio` true), `filterMusicDefaultedOn` (NEW: forces `filterMusic` true),
   `_presetsSeededForUser` (base), `_proCloudMigrationDone` (base), `_parakeetDefaultMigrationDone`
   (target: whisper/cloud/parakeet engines become `parakeet` on Windows/Linux and
   `whisper-large-v3-turbo-quantized` on macOS), `_proCloudAudioDefaultsAppliedV2` (base),
   `_chatOpusAppliedForPro` (NEW; model rewrite for vendor cloud presets, irrelevant to Freenpipe).
8. Semantic-context legacy keys `semanticContextPreference`, `semanticContextRemoteDefault`,
   `semanticContextRemoteForceDisabled` are folded into `remoteControlPolicy` /
   `remoteControlPreferences` and deleted.
9. Preset rewrites: provider `pi` -> `screenpipe-cloud`; preset id `pi-agent` -> `screenpipe-cloud`;
   retired model names are remapped (vendor model ids; Freenpipe should implement a generic
   "unknown model -> keep, let provider error" rule instead).
10. Ensure `chatHistory`, `starSessionShortcut`, `showChatShortcut`, `startAudioShortcut`,
    `stopAudioShortcut` are non-empty (platform defaults), `platform` matches the OS,
    `disabledShortcuts` is an array.
11. Enterprise-managed overlay: if the device has managed settings, merge them over the user settings
    and store a copy in `enterpriseManagedSettings`; otherwise delete that key (ee, shell-035).
12. If anything changed: save, then call `reencrypt_store`.
- Save path: refuse to save with empty `aiPresets`; refuse changing `piiBackend` away from `tinfoil`
  while `workflowSharing` is on (other dim); strip `user.token` before writing (shell-003).
- `resetSetting(key)` resets a single key to its platform default (never `deviceId`).
- Rust side (`store.rs`, 0x52ad380/0x52ad539): still renames `enableUiEvents` -> `enableAccessibility`,
  sanitises unknown providers to `custom` (`opencode` alias kept), handles
  `restartNotificationsDefaultedOff`.

Base touchpoints: `lib/hooks/use-settings.tsx`, `src-tauri/src/store.rs`.

### shell-003 Cloud token in secret store; store location command; fail-closed store init (changed)

Evidence: `378-9f286ebe7c0f0713.js` (save strips `user.token` after `set_cloud_token` succeeds; load
calls `get_cloud_token`); 0x49ecdb0 a log line recording the one-time move of the cloud token into the
encrypted secret store, next to `cloud.auth_token`, `store.bin.last-good`; 0x6a7f008 `auth.json store.bin
/settings/user/token`; 0x5178fd4 a log line that settings-store init failed and startup is aborted, plus a
user-facing cannot-start message; commands `get_screenpipe_base_dir`, `get_active_data_dir`, `get_cloud_token`
(`8001-...js`). Confidence: observed for identifiers, inferred for exact ordering.

Behaviour:
- One-time Rust migration at startup: if `settings.user.token` is present in `store.bin`, write it to
  the secret store under key `cloud.auth_token`, then remove it from `store.bin` (keep `store.bin.last-good`
  consistent). Legacy `~/.screenpipe/auth.json` remains a read source.
- Frontend `set_cloud_token(token|null)` persists to the secret store; on success the token is removed
  from the object written to `store.bin`. `get_cloud_token()` returns the token for hydration.
- `get_screenpipe_base_dir()` returns the directory that holds `store.bin` (normally `~/.screenpipe`);
  `get_active_data_dir()` returns the recording data dir actually in use (custom `dataDir` or default).
- If the settings store cannot be initialised, target shows an error and exits instead of starting with
  defaults (base: falls back to defaults). Rationale: never overwrite a real store with defaults.
Freenpipe: the token concept matters only if a BYO cloud account exists; still implement the
secret-store key so data written by screenpipe can be read. Base touchpoints: `src/store.rs`,
`src/commands.rs` (`set_cloud_token`), `src/main.rs` (store init), `crates/screenpipe-secrets`.

### shell-004 Headless mode (new)

Evidence: module `screenpipe_app::headless` 0x52b6495..0x52b6b2c: log lines covering the UI going dormant
(with a count of destroyed webviews and a `search_only` flag), keeping an unfinished onboarding webview,
failures to unregister or restore global shortcuts, the UI being woken from the tray, failure to restore
tray visibility, and webview teardown failure; 0x5177480 a startup log saying the UI starts dormant and
the tray reopens it; window suppression 0x514f97e (a log naming the window label suppressed while
dormant); 0x52b6aa7 a log that record-only headless mode is on; settings keys `headless`, `headlessRecordOnly`.
Confidence: observed (behaviour), inferred (exact transitions).

Behaviour:
- `headless=true`: the app runs capture + local API with the UI dormant. At startup no windows are
  created; when toggled on at runtime, every webview window is destroyed (to free WebView2 memory)
  except an onboarding window when onboarding is incomplete. Global shortcuts are unregistered while
  dormant. Any request to show a window (shortcuts, deep links, notifications) is ignored and logged
  while dormant.
- Waking: choosing the tray's open-app item (or equivalent) wakes the UI: restore tray visibility,
  re-register shortcuts, create the Home window.
- `headlessRecordOnly=true`: stricter variant; recording continues, UI stays dormant (inferred: tray
  limited to recording controls).
- Shortcut module states for hidden UI (0x499c7c8): with tray present, register no shortcuts and let the
  tray keep pause controls; with tray suppressed, register only recording controls.
Implementation: new `src-tauri/src/headless.rs` with `enter_dormant(app)`, `wake(app)`; guard in
`window/show.rs` `ShowRewindWindow::show`; hook in `shortcuts.rs`; tray "open_app" wakes. Settings UI
toggle in General. Freenpipe header on new file.

### shell-005 Keep search available after quit (search-only mode) (new)

Evidence: `screenpipe_app::search_only` 0x4987800..0x4987950: event names `search_only_ready`
("outcome=capture_stopped; api=retained; ui=closed"), `search_only_failed`, `search_only_stop_slow`
("cause=quit_transition_timeout; outcome=full_exit"), a message that the local history server cannot be reached;
0x49ed369 `search_update_recovered` ("outcome=search_restored; capture=paused"),
`search_update_recovery_failed`; engine `crates\screenpipe-engine\src\search_only.rs` (0x49a7a48);
settings UI `_next/static/chunks/app/(main)/settings/page-d9c69007346aaf6a.js` @379090 (switch id
`keep-search-after-quit`, default on). Confidence: observed.

Behaviour: when `keepSearchAvailableAfterQuit` is true (default), the tray/app "Quit" does not exit the
process. It stops capture (vision + audio), closes/destroys all UI windows and the tray stays (inferred:
tray remains so the user can reopen), but keeps the local HTTP API (port 3030) serving existing data so
external tools/MCP clients can still search. If the transition fails or exceeds its time budget, fall back
to a full exit. After an app update restart, the app may come back in the same search-only state
(search restored, capture still paused). Reopening the app restarts capture.
Implementation: in `main.rs` `ExitRequested` / tray `quit`: if the setting is on, call
`CaptureSession::stop()` but keep `ServerCore`, destroy windows (reuse shell-004 dormant helpers), emit
log line; a second explicit quit (or the setting off) performs full exit. Engine side: a
"search-only" ServerCore start mode (server without capture) already exists conceptually (the base
logs a capture-only stop that leaves the server running; the same log is in target 0x5129090).

### shell-006 Low-disk recording guard (new)

Evidence: engine `screenpipe_engine::disk_pressure` 0x6aa7120..0x6ab2e38: events `disk_space_low`
(fields `available_bytes`, `threshold_bytes`, `data_dir`), `disk_space_recovered`
(`recovery_threshold_bytes`), log messages for failing to resolve the volume behind the data dir, for
space recovering past the re-arm headroom, and for the monitor stopping; app
`disk_pressure_notifications` 0x49907f8..0x49909b8 and 0x5224620..0x5224ae8: notification
types `disk_space_low` / `disk_space_recovered`, a review-storage action -> `screenpipe://settings?section=storage`,
templates with `{available}` and `{threshold}`, state file `disk-pressure-notification-state.json`
(0x51a1520), a log that the guard is off so capture keeps running; command
`get_low_disk_guard_config` returning `{ thresholdBytes }` (settings page @541205); settings switch id
`stop-recording-on-low-disk`, test id `low-disk-recording-guard-toggle` (@546838). Confidence: observed;
numeric thresholds unknown.

Behaviour:
- Engine probes free space on the volume holding the data dir periodically. When free bytes drop to or
  below `threshold_bytes`, publish `disk_space_low` on the event bus; when free bytes rise above a higher
  `recovery_threshold_bytes` (hysteresis), publish `disk_space_recovered`.
- App: on `disk_space_low`, if `stopRecordingOnLowDisk` (default true) stop capture (not the API, not
  pipes) and post one notification with an action to open storage settings; if the guard is disabled,
  only log. The "in flight" incident and whether it was notified persist in
  `disk-pressure-notification-state.json` so it is not repeated across restarts; recovery re-arms it.
  Enterprise-managed variant of the message mentions organisation-managed storage (ee).
- `get_low_disk_guard_config()` -> `{ thresholdBytes: u64 }` for the settings label.
Implementation: new engine module `crates/screenpipe-engine/src/disk_pressure.rs` (event bus publish
via `screenpipe-events`), new app module `src-tauri/src/disk_pressure_notifications.rs`, command in
`commands.rs`. Choose threshold (suggest max(2 GiB, 2 % of volume)) and recovery threshold (+1 GiB);
document as Freenpipe choice (not observed).

### shell-007 Keep computer awake (new)

Evidence: setting `keepComputerAwake` (default false); command `set_keep_awake` with arg `enabled`
(0x49935d1); app log for a failure to apply the setting (0x49ed87b); engine thread name
`screenpipe-keep-awake` (0x6a9dfeb), engine logs for an enable timeout, for enabled/disabled
transitions and for `SetThreadExecutionState` failing (0x6a08b88); import `SetThreadExecutionState`
(0xa6a95b0); module `power\awake.rs`. Confidence: observed.

Behaviour: when enabled, a dedicated engine thread calls `SetThreadExecutionState` with continuous
system-required semantics (prevents idle sleep, display may still turn off; inferred) and holds it
until disabled; disabling clears the continuous flag from the same thread. Enabling waits with a
timeout for the thread to confirm. Applied at startup from settings and live through `set_keep_awake`.
macOS/Linux: equivalent assertion or no-op. Implementation: `crates/screenpipe-engine/src/power/awake.rs`
(new file in the existing `power` dir), command in `commands.rs`, settings toggle.

### shell-008 App screen-share protection (new)

Evidence: settings `hideAppInScreenShare` (default true), `hideOverlayInScreenRecording` (default
false) alongside base `showOverlayInScreenRecording`; commands `get_app_screen_capture_protection`,
`set_app_screen_capture_protection`; "SetWindowDisplayAffinity failed" (0x5173eb5). Confidence: observed
(identifiers), inferred (exact window set).

Behaviour: when `hideAppInScreenShare` is true, every app window (Home, chat, search, panels) gets
`WDA_EXCLUDEFROMCAPTURE` so screen sharing / recording tools (and screenpipe's own capture) do not show
it; false restores `WDA_NONE`. Newly created windows apply the current value. The overlay keeps its own
rule: `hideOverlayInScreenRecording` forces exclusion for the timeline overlay even if
`showOverlayInScreenRecording` was set (inferred precedence: hide wins). `get_*` returns the effective
bool. Implementation: reuse `windows_overlay.rs` affinity helper; call it in `window/show.rs` builders
and in the setter command; macOS uses `NSWindow.sharingType = none` (keep cfg paths compiling).

### shell-009 Shortcut overlay and star-session shortcut (new)

Evidence: settings keys above; commands `snooze_shortcut_reminder_for_hour`,
`set_shortcut_overlay_anchor(anchor, monitorName)`, `toggle_starred_sessions`, `hide_starred_sessions`;
`commands::overlay_anchor` 0x51796c6 (`bottom-center top-center left-center right-center`, plus an
unknown-anchor error), 0x5179880 (a log for failing to persist the anchor or display); snooze strings
0x499a6e0..0x499a760; shortcut ids 0x499c670 `start_recording stop_recording start_audio stop_audio
show_chat star_session`, analytics `shortcut_used`; frontend `_next/static/chunks/app/shortcut-reminder/page-a5341a3196435d05.js`
(anchor list `top-center,right-center,bottom-center,left-center`; legacy corner values `top-left`,
`top-right` map to `top-center` and `bottom-left`, `bottom-right` map to `bottom-center`; size scale small 1,
medium 1.5, large 2). Confidence: observed.

Behaviour:
- New global shortcut `starSessionShortcut` (Windows default `Alt+Shift+B`, else `Control+Super+B`),
  disabled-list key `star_session`; action toggles the starred-sessions window/marker (starred sessions
  feature itself belongs to another dimension).
- Snooze: the one-hour hide action writes `shortcutOverlaySnoozedUntil` = now + 1 h; a Rust timer clears the
  value at expiry and re-shows the overlay; expired values are cleared at startup.
- `allowHidingShortcutOverlay` gates whether the overlay can be hidden permanently via
  `showShortcutOverlay=false` (inferred).
- Anchor: the pill can be dragged; on drop it snaps to the nearest edge-center of the monitor work area
  and persists `shortcutOverlayAnchor`; Rust also records which monitor in `shortcutOverlayDisplay`.
  Layout: left/right anchors are vertically centred, top/bottom horizontally centred with a small margin
  (4 logical px times scale).
Base touchpoints: `src/store.rs`, `src/shortcuts.rs` (add star_session to config + `is_disabled` map),
`src/commands.rs` (`show_shortcut_reminder`), `app/shortcut-reminder/*`.

### shell-010 Native Win32 overlay pill (new, Windows)

Evidence: `screenpipe_app::native_overlay_win` 0x498faf8, 0x52d8845..0x52d8ca8: logs for the native
window being shown and for falling back to the webview when it is unavailable, payload keys `metrics_ws_url`,
`events_ws_url`, `status`, `items`, `active`, `activeMeetingId`, `metrics`, item ids `overlay chat star
search`, `shortcutOverlaySize`, `shortcutOverlayAnchor`, `deviceName deviceType itemId speakerName
capturedAt isFinal`; feeds `ws://127.0.0.1:<port>/ws/metrics`, `ws://127.0.0.1:<port>/ws/meeting-overlay`,
`/audio/audio_level_rms`, `/audio/mic_capture_active`; crate `screenpipe-overlay-win` (render.rs,
window.rs); imports `d2d1.dll`, `dwrite.dll` (0xa6a9c6e). Confidence: observed (existence), inferred
(rendering details).

Behaviour: on Windows the shortcut-reminder pill is drawn by a native layered Win32 window (Direct2D +
DirectWrite), mirroring the macOS SwiftUI panel already in base (`native_shortcut_reminder.rs`). It
subscribes to the engine metrics websocket for audio level / mic activity and to meeting live
transcript events, shows shortcut chips (overlay/timeline, chat, star, search) and reports clicks back
to Rust. If creation fails, use the existing webview `shortcut-reminder` window.
Implementation: new crate `crates/screenpipe-overlay-win` (Windows-only, `windows` crate Direct2D);
`native_shortcut_reminder.rs` gets a Windows branch. Medium priority (webview fallback already works).

### shell-011 Recording health state and overlay incidents (new)

Evidence: `screenpipe_app::overlay_health` 0x498b756..0x5a0d270: event `recording-health-state`;
incident analytics `recording_incident_shown`, `recording_incident_recovered`,
`recording_incident_dismissed`, `recording_incident_restart_clicked`; states `normal failure recovering
fixing recovered manual-reopen`; startup phases `starting`, `loading pipes`, `starting audio`,
`updating database`, `starting engine`; incident reasons in health (0x61857a0) cover the engine failing
to start, the engine stopping (expectedly or not), recording data that cannot be written, stale screen
capture, stale audio capture, both going stale, several errors at once, and a missing permission; status codes `stale not_started
meeting_detector_unavailable permission_denied capture_stalled waiting_for_meeting`; commands
`get_recording_health_state`, `overlay_restart_recording`, `overlay_dismiss_incident`,
`retry_screenpipe`, `is_capture_paused`; frontend shortcut-reminder page parses the payload as a
`|`-separated string with four fields. Confidence: observed (identifiers), inferred (state machine timings).

Behaviour:
- Rust keeps a recording-health state machine fed by health polling (shell-012), boot phases and engine
  events. It emits `recording-health-state` with payload string `"<state>|<detail>|<kind>|<extra>"`,
  where `state` in {`normal`, `failure`, `recovering`, `fixing`, `recovered`, `manual-reopen`}, `detail`
  is a short human reason, `kind` in {`audio`, `screen`, other} drives the headline, fourth field optional.
  `get_recording_health_state()` returns the same string for initial render.
- Incident flow: when an incident is confirmed (a failure persisting across several checks), the
  overlay pill is revealed even if hidden/snoozed and shows a failure chip with restart and dismiss actions.
  Restart (`overlay_restart_recording`) stops then restarts capture; if a manual-only recovery is
  active (e.g. screen permission requires relaunch) it refuses and shows `manual-reopen` with guidance to
  quit and reopen. After restart the state goes `fixing` -> `recovering` -> `recovered` once healthy for
  N consecutive checks, else back to `failure`. Dismiss records the incident as dismissed until a new
  incident starts.
- `retry_screenpipe` re-runs the full start sequence after a failed start.
Base touchpoints: `src/health.rs` (extend), new `src/overlay_health.rs`, `app/shortcut-reminder/*`.

### shell-012 Engine watchdog: auto-respawn and port-conflict handling (changed)

Evidence: `screenpipe_app::health` 0x5112ef0, 0x6185c38..0x6185d87: logs covering an auto-respawn after
N failed checks while recording is meant to be on (with a count of recent attempts), a failed respawn
with later attempts still scheduled, a respawn skipped because the port is held, the port freeing up and
the engine restarting, the port freeing up while recording is not wanted (error cleared only), the
engine coming back after the conflict, and a stuck start-in-progress flag being ignored for tray status
while the server answers (0x4995388); `screenpipe_app::port_conflict` (0x524c930,
0x52d80e8) runs `netstat -ano -p tcp` and `taskkill /PID <pid> /F`; notifications 0x6202233: unhealthy
process holding local port `{value1}`, could not reclaim port, another healthy screenpipe already
running but not focusable, another healthy screenpipe using port `{port}`, and a plain already-running
notice. Base: `recording.rs` / `server.rs` kill every non-self PID on the port. Confidence: observed
(behaviour), inferred (counts/windows).

Behaviour:
- Watchdog: while the user intends recording ON, health polls the embedded engine; after K consecutive
  unreachable checks, respawn the engine (stop + start ServerCore/Capture). Rate limit respawns (max M per
  window), keep scheduling later attempts if one fails.
- Port conflict classification before killing anything: list PIDs listening on the API port
  (`netstat -ano -p tcp`). If a PID is ours, ignore. If the holder answers `/health` as a healthy
  screenpipe, do not kill it: tell the user another screenpipe instance holds the port and try to focus it.
  If the holder is a screenpipe-like process that is unhealthy, kill it (`taskkill /PID n /F`), wait,
  retry. If a non-screenpipe process holds it, do not kill; notify the user to close it.
  When the port frees up later and recording is intended, restart the engine; if recording is not
  intended, only clear the error.
- The tray must not show "Starting" forever if the start flag is stale while the server responds.
Freenpipe suggested constants (not observed): check every 5 s, respawn after 3 misses, max 3 respawns per
10 min. Base touchpoints: `src/health.rs`, `src/recording.rs` (replace blind kill), `src/server.rs`.

### shell-013 Safe relaunch and process-exit teardown (new)

Evidence: `screenpipe_app::process_exit` 0x5a0b592..0x5a0c35c: logs for pre-exit teardown finishing,
failing or overrunning its budget (then force-exiting), for recording + server and the Pi sidecar being
stopped before exit, for a safe relaunch being requested and its replacement pid spawned, for failing to
resolve the current binary, for a relaunch deferred because the recording mode could not be carried over,
for quit requests ignored during teardown or during protected database recovery, and for the final
force-exit with a status code; env passed to the replacement: `SCREENPIPE_RELAUNCH_HOME_VISIBLE` (`true`/`false`),
`SCREENPIPE_DATA_DIR`; analytics `app_relaunch_requested`, `relaunch_window_state`; main.rs 0x524ded0
(logs for a pending restart that spawns the replacement and force-exits, and for an `ExitRequested`
during app restart being allowed through); `recording::server_shutdown` 0x5129148 (logs enforcing that
server generations do not overlap during shutdown and that the old engine drains before the database is
reopened); engine drain 0x524c448 (HTTP drain 3 s, audio manager shutdown timeout 15 s, a log once all
SQLite pools are closed); commands `restart_for_update`,
`restart_after_screen_recording_permission`. Confidence: observed.

Behaviour:
- A single teardown routine used by quit, update restart and relaunch: stop capture, drain HTTP server
  (3 s), stop schedulers, shut down audio manager (15 s budget), close SQLite pools, stop Pi/agent
  children, all under an overall budget; if exceeded, log and force-exit.
- Safe relaunch: compute whether the Home window is visible, spawn the same executable with env
  `SCREENPIPE_RELAUNCH_HOME_VISIBLE=true|false` and `SCREENPIPE_DATA_DIR=<active data dir>`, then
  force-exit. The new process restores Home only if the flag is true (otherwise starts in background,
  `startup_window_restore outcome=home_kept_closed`, 0x5177431).
- Server generation rule: a new engine generation may only open the DB after the previous generation
  fully drained.
- Quit requests during teardown or during protected DB recovery are ignored.
Implementation: new `src/process_exit.rs`; refactor `main.rs` `RunEvent::Exit` and update/restart paths
to use it.

### shell-014 Database self-heal user flow (new; DB internals owned by the db dimension)

Evidence: `db_relaunch` 0x49ecf38..0x49ed1e0 and 0x5a61680..0x5a61b30 (error classes, matched as exact
substrings of SQLite/engine error text: `initialize database`, `disk i/o error`, `database disk image is
malformed`, `database unavailable`, `database is locked`, `database is busy`; telemetry fields `sqlite_quarantine_state`, `sqlite_marker_metadata`,
`sqlite_extended_code`, `sqlite_primary_code`, `sqlite_marker_age` buckets `under_1h 1h_to_1d
1d_to_7d over_7d clock_skew`); `db_recovery_notifications` 0x514cf40..0x514d709 (actions
`recover-database`, `dismiss-database-recovery`, deep link `screenpipe://database-recovery`, types
`db_recovery`, `db_recovery_needs_recovery`, `db_recovery_restart_failed`, `dbRecoveryFailed`);
`recording::db_wedge` 0x512a2dc..0x512a4a8 (restart recording to rebuild DB pools + shared WAL index,
debounce, a log that the database is still down and the next recovery attempt is being waited for);
`recording::retry` (`retry_started`, `retry_completed`, `retry_failed`); recovery diagnostics file
`recording-recovery.log` (0x517a06d); commands `start_database_recovery`, `restart_database_verification`;
deep link host `database-restart-verification`; a log that database init failed because the recording data
dir is inaccessible, with the UI kept up while recording retries (0x517714c). Confidence: observed.

Shell-side behaviour (implement in the app; recovery engine is the db spec):
- On DB open failure classify the error; transient (locked/busy/unavailable) -> keep UI up, retry with
  backoff, notify once that recording was interrupted and is retrying. Verified damage -> pause recording,
  notify with a recover-database action (and a defer action), open via deep link or command
  `start_database_recovery()`. Recovery requires free space of about twice the DB size.
- `restart_database_verification()` relaunches into a fresh process to verify a quarantined DB generation
  (uses shell-013 relaunch).
- DB wedge: if writes stall, restart recording (new server generation) after a debounce.
- Append a one-line diagnostic per recovery attempt to `<data dir>/recording-recovery.log`.
Base touchpoints: `src/recording.rs`, `src/notifications/*`, `src/main.rs`; new `src/db_relaunch.rs`,
`src/db_recovery_notifications.rs`.

### shell-015 Storage migration orchestration (new; storage format owned by the db dimension)

Evidence: `storage_migration` 0x498ee28..0x5a64988: event `storage-migration-activity` with fields
`RecordingPreference root recording app_session_id busy pending in_place using_new_storage generation
source_bytes migrated_bytes bytes_saved available_bytes can_migrate can_cancel can_delete_source
blocked_reason recovering elapsed_seconds completed_records total_records`; error file
`storage-migration-error.txt`; settings key `pendingStorageMigration`; telemetry
`storage_migration_started/completed/failed/stalled`, `original_db_bytes migrated_db_bytes
compression_multiplier`; blocked reasons (vault protection enabled `vault.meta`/`.vault_locked`, data dir
change pending, restarting); commands `get_storage_migration_status`, `get_storage_migration_activity`,
`start_storage_migration`, `cancel_storage_migration`, `delete_original_storage_database`; settings page
requires free space >= 2 x DB size + 512 MiB (2 x DB bytes + 0x20000000, page @541205). Confidence: observed.

Shell behaviour: a resumable, user-started migration of the history database into a new storage format.
Recording is paused during migration, the user chooses whether recording resumes afterwards; a failed run
writes a retry block and disables automatic retries until the user retries explicitly; the original DB is
kept until the user deletes it. Progress is pushed via `storage-migration-activity`.
Implement only after the db dimension defines the new storage format; until then the commands may
return "not available".

### shell-016 Updater changes (changed + decloud)

Evidence: endpoint built at runtime 0x6203b1d/0x6203b6a: base `https://screenpipe.com/api/app-update/` +
channel (`stable` or `pre-release`, 0x6203ada) + `/{{target}}-{{arch}}/{{current_version}}`; compiled conf
default `https://screenpipe.com/api/app-update/stable/...` (0x5a0e250); rollback
`https://screenpipe.com/api/app-update/rollback/` (0x5128497); a log that the update check is skipped in
dev mode (TAURI_ENV_DEBUG, 0x4997c38); update notifications with `{value1}` version templates covering a
sign-in-required variant (auth_required), a restart-to-install variant and a download-failed variant that
asks the user to reopen the app to retry (0x5a41208);
`update_diagnostics`: files `update-install.log`, `update-recovery.log` (writes refused when a diagnostic
path is a symlink, 0x51751a4); marker `update-attempt.json` with keys `from_version to_version
ts_epoch_secs home_visible target executable outcome running attempt_ts`, `restart_committed`,
logs saying whether the previous install applied and how many leftover installer files were removed, outcome
`relaunch_observed` (0x6203cc6..0x6203e52); settings `autoUpdate` default true, `updateChannel`; settings UI
release-channel select with values `stable` (labelled as the standard channel) and `pre-release` (@380899); command
`restart_for_update`; NSIS update args `/UPDATE /R /ARGS`, `AUTOLAUNCHAPP=True` (tauri-plugin-updater
internals, 0x62462d0). Confidence: observed.

Behaviour (target): periodic update check (skipped in dev builds) against the channel endpoint; when an
update is found and `autoUpdate` is on, download and install; before restarting write
`update-attempt.json` (from/to version, timestamp, whether Home was visible, executable path), append a
line to `update-install.log`; after relaunch compare the running version with the marker: success ->
log `relaunch_observed`, show what's-new; failure -> `update-recovery.log` + notify that the update did
not apply, with a retry action; remove leftover installer files. Refuse to write diagnostics through symlinks.
`restart_for_update()` triggers the install + safe relaunch (shell-013).
Freenpipe (decloud): MUST NOT contact screenpi.pe / screenpipe.com. Replace endpoints with Freenpipe
GitHub Releases (`https://github.com/normiecore/Freenpipe/releases/latest/download/latest.json`
pattern, own minisign key) or ship with updater disabled (`official-build` feature off) and a state that
reports auto-updates as unavailable. Map channel `pre-release` to GitHub prereleases. Drop rollback endpoint or point to
GitHub tags. Keep the attempt marker + logs (local only). Base touchpoints: `src/updates.rs`,
`tauri.prod.conf.json`, `tauri.beta.conf.json`, settings UI.

### shell-017 Windows crash dumps via WER helper (new, Windows)

Evidence: install dir contains `screenpipe_wer_dump_helper.dll` (147 KB) exporting
`OutOfProcessExceptionEventCallback`, `OutOfProcessExceptionEventSignatureCallback`,
`OutOfProcessExceptionEventDebuggerLaunchCallback`, importing `MiniDumpWriteDump`,
`QueryFullProcessImageNameW`, `GetModuleFileNameW`, `CreateDirectoryW`; strings `crash-dumps`, `.dmp`
(DLL offsets 0x19100, 0x1911c); app strings 0x5a0d2d8..0x5a0d502: `crash-dumps`, a log counting the
minidumps recovered into a directory, a log that the helper DLL is missing beside the app (normal in dev
builds), registry key
`Software\Microsoft\Windows\Windows Error Reporting\RuntimeExceptionHelperModules`, a log for failing to
write the helper-module registry value, `WerRegisterRuntimeExceptionModule failed`, a log stating whether
fail-fast minidump capture is enabled or unavailable, thread/marker `screenpipe-wer-dump-harvest`. Confidence: observed (mechanism),
inferred (exact paths).

Behaviour:
- At startup (Windows only) locate `screenpipe_wer_dump_helper.dll` next to the exe; ensure a REG_DWORD
  value named with the DLL full path exists under `HKCU\Software\Microsoft\Windows\Windows Error
  Reporting\RuntimeExceptionHelperModules` (HKLM requires admin; per-user install so HKCU inferred), then
  call `WerRegisterRuntimeExceptionModule(dll_path, ctx)`. This captures fail-fast crashes (stack buffer
  overrun, `__fastfail`, abort) that Rust panic hooks cannot see.
- The DLL callback runs inside WerFault, writes a minidump of the crashed process into a `crash-dumps`
  directory (inferred: next to the DLL or under the user's local app data) and declines further WER
  handling.
- On next launch a harvest step moves any `*.dmp` into `<data dir>/crash-dumps/` and logs the count.
Freenpipe: implement the DLL as a small `cdylib` crate (e.g. `crates/freenpipe-wer-helper`) with original
code; never upload dumps; surface them in the diagnostics bundle (shell-033). Add to
`tauri.windows.conf.json` resources.

### shell-018 WebView renderer heartbeat and swipe navigation (new)

Evidence: command `webview_renderer_heartbeat` invoked every 1000 ms per webview, non-overlapping
(`_next/static/chunks/app/layout-3ec03e167c097e07.js` @140350); base WebView2 ProcessFailed handler kinds
still present (0x5a40bf8: `browser_process_exited render_process_exited render_process_unresponsive
frame_render_process_exited gpu_process_exited sandbox_helper_process_exited utility_process_exited
unknown_process_exited`, event kind `webview-process-failed`); command
`set_history_swipe_navigation_enabled(enabled)` called with false by the root layout; log strings for a
cancelled Windows swipe-setting update and for a failed swipe-navigation configuration. Confidence:
observed (frontend), inferred (Rust reaction to stale heartbeats).

Behaviour: each window pings Rust once a second; Rust records last-seen per window label. A window whose
heartbeat is stale beyond a threshold while visible is treated as a hung renderer: reload it, and if
reload fails destroy and recreate it (same path as the ProcessFailed handler). On Windows, WebView2
swipe (history) navigation is disabled via `ICoreWebView2Settings6::put_IsSwipeNavigationEnabled(false)`
so a two-finger swipe cannot navigate the SPA backwards. Base touchpoints: `src/window/content_process.rs`,
`src/commands.rs`, `app/layout.tsx`.

### shell-019 Focus handoff and local control server hardening (changed)

Evidence: `FocusPayload` fields `args deep_link_url target launch_exe launchd_job_label` (0x514eb24);
server.rs 0x514e2a0..0x514e448: logs for an incoming focus request (with `args_count`,
`deep_link_present`, `target`), for rejecting a handoff from a different executable (logging
`launch_exe` and `current_exe`), for a handoff that belongs to another install, for being unable to verify
the handoff executable, for rejecting a handoff whose scheme or deep link belongs to a different build,
and for ignoring a duplicate OS-startup handoff under autostart; 0x6185f70/0x61860d0 logs for the control
server rejecting a non-local `Origin` / non-local `Host`. Base `server.rs` allows CORS `*` and focus body `{args, deep_link_url}`.
Confidence: observed.

Behaviour:
- Second instance POSTs `/focus` on `127.0.0.1:${SCREENPIPE_FOCUS_PORT:-11435}` with
  `{ "args": [...], "deep_link_url": string|null, "target": string|null, "launch_exe": "<abs path of the
  launching exe>", "launchd_job_label": string|null }` (macOS label only).
- Receiver: canonicalise both exe paths; if they differ, reject (another install, e.g. stable vs beta or
  Freenpipe vs screenpipe) and let the caller continue starting/exit with a message. If a deep link's
  scheme is not this build's scheme, reject. If args contain `--autostart`, do not show any window
  (ignore duplicate OS-startup launches). Otherwise focus/show Home and forward `deep-link-received` /
  `second-instance-args` events (base).
- All routes on the 11435 control server reject requests whose `Origin` header is present and not a
  local origin (`tauri://localhost`, `http(s)://localhost*`, `http(s)://127.0.0.1*`) and whose `Host`
  is not `localhost`/`127.0.0.1` (DNS-rebinding protection). Replace the permissive CORS.
Base touchpoints: `src/main.rs` (single-instance pre-check), `src/server.rs`.

### shell-020 Autostart: `--autostart` argument and background launch (changed)

Evidence: `--autostart` literal next to main.rs strings (0x5a65bd8); logs for a launch from OS startup
going straight to background and for skipping the Home window on a background launch (logging
`restart_home_visible` and `autostart`) (0x5177380); a log for ignoring a duplicate login LaunchAgent
handoff (0x5a0dfb8, macOS); a log for skipping autostart registration under dev isolation (0x51776d5); registry paths used by the
auto-launch crate `SOFTWARE\Microsoft\Windows\CurrentVersion\Run` and
`...\Explorer\StartupApproved\Run` (0xa464609); `enforce_auto_start` / `enforceAutoStart` (enterprise).
Confidence: observed.

Behaviour: register autostart with launch args `["--autostart"]` (Windows: HKCU Run value named after
the product, value `"<exe>" --autostart`; StartupApproved respected when the user disables the entry in Task Manager).
When started with `--autostart`, the app starts tray + engine but does not open Home (unless onboarding is
incomplete, inferred). Dev/debug builds do not register autostart (avoid hijacking the installed app).
Enterprise policy may force-enable (ee). Base touchpoints: `main.rs` (`tauri_plugin_autostart::init(
MacosLauncher::LaunchAgent, Some(vec!["--autostart"]))`), startup window logic, `commands.rs::set_autostart`.

### shell-021 Startup environment controls (new)

Evidence: `SCREENPIPE_SKIP_ONBOARDING` (0x51772f0, success/failure logs for marking onboarding complete); `SCREENPIPE_DATA_DIR`
(0x5177271, next to the log line that prints the recording data directory; also passed on relaunch); logs for
failing to add the data dir to the fs scope or the asset protocol scope (0x5177285); `SCREENPIPE_STARTUP_AUTHENTICATION_STATUS` with values
`authenticated logged_out not_required` (0x52d7308). E2E seed commands were removed from the target
(`get_e2e_seed_flags`, `e2e_*` absent). Confidence: observed.

Behaviour: `SCREENPIPE_SKIP_ONBOARDING=1` marks onboarding complete at startup (replaces the base E2E
seed `onboarding`); `SCREENPIPE_DATA_DIR` overrides the recording data dir for this process (takes
precedence over `dataDir`); when the data dir is not under the static scopes, add it at runtime to the
fs plugin scope and the asset protocol scope (`app.asset_protocol_scope().allow_directory(dir, true)`
and fs scope) so the UI can load frames from it. Keep `SCREENPIPE_E2E_SEED` for Freenpipe tests.
`SCREENPIPE_STARTUP_AUTHENTICATION_STATUS` is cloud-account related (shell-034).

### shell-022 CPU compatibility mode (AVX2) (new)

Evidence: a startup log that the CPU has no AVX2, so the app falls back to a compatibility mode in which
Whisper and the non-Windows Qwen STT are turned off, but Parakeet and the ONNX-based Qwen build on Windows
remain usable (0x52d96b9, 0x5176fc9); a `cpu-smoke:` marker log confirming static init succeeded (0x52d97c2); a log
for skipping the whisper model pre-download on non-AVX2 CPUs (0x4993189); BootPhaseSnapshot gains
`cpuCompatMode` (0x61857a0). Confidence: observed.

Behaviour: very early in `main` detect AVX2 (`is_x86_feature_detected!("avx2")`); if missing set a global
compat flag: skip whisper pre-download, refuse whisper engines (fall back to parakeet/ONNX), expose
`cpuCompatMode: true` in the boot phase snapshot so onboarding/settings can explain. Requires building
the whisper/ggml parts so that AVX2 code is not executed at static init on such CPUs.
Base touchpoints: `main.rs`, `health.rs` (BootPhaseSnapshot), audio engine selection.

### shell-023 Tray menu changes (changed)

Evidence: 0x5a3f198..0x5a3f9d6, 0x5a3ec48, 0x5a3ed31..0x5a3ee9e; base `src/tray.rs`. Confidence: observed.

Changes vs base:
- Monitor rows become toggles: ids `toggle_vision_device_<name>` calling `/vision/device/start` and
  `/vision/device/stop` (mirrors audio `/audio/device/start|stop`).
- Status line composition: states for starting, recording, stopped, paused, outside work hours and error,
  plus compound states pairing screen recording with audio idling until a meeting (when
  `audioCaptureMode` is `meetings-only` and no meeting) or with meeting detection being unavailable.
- Toggle label gains variants for outside-work-hours (paused by the schedule) and for stopped because no
  devices are enabled.
- When capture is paused the secondary line tells the user that capture is on hold but pipes and
  search keep working.
- Pause submenu unchanged ids (`pause_5/15/30/60`); notifications on pause (with the auto-resume
  duration) and on resume; error notifications when pausing or resuming fails.
- New item that opens the first-run summary (other dim) during early use.
- Bottom section: settings, a help submenu (feedback, book call, releases/changelog; inferred
  grouping), quit (label capitalised). Plan labels name the vendor subscription tiers (a free tier, a
  basic tier, three business tiers and a lifetime tier) (Freenpipe: drop plan/upgrade rows entirely, decloud).
- Hidden-UI policy suppresses individual items (`tray_chat`, `tray_search`, `tray_timeline`,
  `tray_plan`, `tray_recording_controls`, `tray_settings`, base) and logs suppression.
- Tray recreation path now also exists on Windows (`recreate_tray`, with a log that the icon was
  recreated in the rightmost position) and a `tray_menu_update` loop with retries if menu install fails;
  duplicate updater spawn prevented.
- Labels use `{count}`, `{duration}`, `{plan}`, `{time}`, `{reason}`, `{fps}` templates through the
  native localization layer (shell-036).
- A parallel structured menu model exists (ids `navigation timeline search chat app recording
  recording_toggle pause_5m pause_15m pause_30m pause_60m hd_start_15m hd_start_30m hd_start_60m
  hd_start_120m hd_stop hd_extend_30m audio_device_toggle monitor_toggle privacy commercial`), likely
  for the macOS native menu bar panel; not needed on Windows.
Icon variants unchanged (white/black, `-failed`, `-updates`, `-beta`). Events unchanged
(`tray-show-timeline`, `tray-show-chat`, `tray-show-search`, `tray-upgrade`).

### shell-024 Multicall helper modes of the app executable (new)

Evidence: `--screenpipe-acp-runtime` with env `SCREENPIPE_ACP_ID`, `SCREENPIPE_ACP_CWD`,
`SCREENPIPE_BUN_PATH`, `SCREENPIPE_ACP_ARGS_JSON`, `SCREENPIPE_ACP_ENV_JSON`, `SCREENPIPE_ACP_UNATTENDED`
(0x499143f, 0xa0befc6); `--screenpipe-acp-process-guard` (an error when the guard executable cannot be
found, log prefix `[acp-process-guard]`, 0xa0cab6a); `--screenpipe-portable-mcp` with env
`SCREENPIPE_MCP_TRANSPORT=http`, `SCREENPIPE_MCP_HOST=127.0.0.1`, `SCREENPIPE_MCP_PORT`,
`SCREENPIPE_MCP_PATH=/mcp` and package checks (Bun path, package root, entrypoint must stay inside the
installed package) (0xa1170fe..0xa117260). Confidence: observed (argv contract), inferred (dispatch point).

Shell behaviour: `main()` checks argv[1] before any Tauri/single-instance work; if it equals one of the
helper flags, run that helper and exit with its status, never touching the GUI, the store lock, or the
focus port. Feature semantics belong to the agents/MCP dimension. Base touchpoint: top of `main.rs`
(next to the macOS `--check-arc-automation` early-exit block).

### shell-025 Environment variable inventory (new/changed)

Evidence: `shell_uniq.py` over `app_strings.tsv`, split on packed literals; offsets given where useful.
Confidence: observed (names), inferred (semantics marked "inf").

New in target (not in base):
| Variable | Meaning |
|---|---|
| `SCREENPIPE_SKIP_ONBOARDING` | mark onboarding complete at startup (shell-021) |
| `SCREENPIPE_RELAUNCH_HOME_VISIBLE` | `true`/`false`, set on safe relaunch (shell-013) |
| `SCREENPIPE_DATA_DIR` | data dir override (base had it only in other contexts; now honoured by the app at startup) |
| `SCREENPIPE_STARTUP_AUTHENTICATION_STATUS` | `authenticated` / `logged_out` / `not_required` (cloud, shell-034) |
| `SCREENPIPE_DISABLE_TELEMETRY` | disables engine analytics; `GITHUB_ACTIONS` / `CI` also disable (0x6a7f678); `is_telemetry_disabled_by_env` command |
| `SCREENPIPE_DIRECTML` | set to a false value to disable DirectML ("explicitly disabled", 0x6b13f20) |
| `SCREENPIPE_COMPACTION_ENCODER` | snapshot compaction encoder: `software`, `hardware`, or one of `hevc_videotoolbox hevc_nvenc hevc_qsv hevc_amf libx264`; unknown -> default probe order (0x6a78910) |
| `SCREENPIPE_KEEP_NORMAL_PRIORITY` | inf: skip lowering process priority (0xa043dd8) |
| `SCREENPIPE_RESOURCE_BREAKDOWN`, `SAVE_RESOURCE_USAGE` | resource monitor per-process breakdown logging / persistence (0x6a6780b) |
| `SCREENPIPE_OAUTH_TIMEOUT_SECS` | connection OAuth callback wait (0x51122a9) |
| `SCREENPIPE_MEETING_DETECTOR` | inf: meeting detector override endpoint (0x6a44810) |
| `SCREENPIPE_TRAJECTORIES_DIR`, `SCREENPIPE_TRAJECTORIES_MAX_RUNS` | pipe trajectory JSONL recording (0xa0eca35) |
| `SCREENPIPE_TOOLS_HTTP_PORT` | ACP runtime tools HTTP server port (0xa0cb0f3) |
| `SCREENPIPE_ACP_*` (`ID CWD ARGS_JSON ENV_JSON COMMAND AUTH_METHOD SYSTEM_PROMPT SESSION_CONFIG_JSON TOOL_ALLOWLIST USER_MCP_JSON RESUME_SESSION_ID UNATTENDED`), `SCREENPIPE_INTERNAL_ACP_WRAPPED_COMMAND`, `SCREENPIPE_INTERNAL_ACP_WRAPPED_ARGS_JSON` | ACP runtime contract (agents dim) |
| `SCREENPIPE_MCP_TRANSPORT/HOST/PORT/PATH`, `SCREENPIPE_MCP_CLIENT`, `SCREENPIPE_MCP_SERVER_ALLOWLIST` | portable MCP + MCP allowlist (MCP dim) |
| `SCREENPIPE_CHAT_SESSION_ID`, `SCREENPIPE_CHAT_CONTROL_ADDR`, `SCREENPIPE_CHAT_CONTROL_TOKEN`, `SCREENPIPE_CHAT_CONTROL_DISABLED`, `SCREENPIPE_VIDEO_CLI`, `SCREENPIPE_PIPE_NAME`, `SCREENPIPE_CLI` | env given to agent children (agents dim); `SCREENPIPE_CLI` = path of a pre-resolved CLI binary |
| `SCREENPIPE_PI_AGENT_DIR`, `SCREENPIPE_GLM_MODEL` | Pi agent dir; vendor confidential model id (decloud) |
| `SCREENPIPE_AI_TOOLS_HOME` | override home used to find AI tool configs (`.claude`, `.codex`, `.cursor`, `.gemini`, `.openclaw`, `.hermes`) for MCP auto-connect |
| `SCREENPIPE_ENTERPRISE_DEVICE_ID`, `..._HIDE_APP`, `..._HIDE_TRAY`, `..._UI_MODE`, `..._INSTALL_SOURCE`, `..._UPDATE_MANAGER`, `..._LICENSE_HASH` | ee (shell-035) |

Unchanged/base vars still present: `SCREENPIPE_FOCUS_PORT` (11435), `SCREENPIPE_ENCRYPT_STORE`,
`SCREENPIPE_FD_LIMIT` (8192), `SCREENPIPE_ANALYTICS_ID`, `SCREENPIPE_DISTRIBUTION`/`SCREENPIPE_DIST`
(`desktop-app`), `SCREENPIPE_LOCAL_API_URL/PORT/KEY`, `SCREENPIPE_API_AUTH_KEY`, `SCREENPIPE_API_KEY`,
`SCREENPIPE_API_URL`, `SCREENPIPE_API_BASE_URL`, `SCREENPIPE_PORT`, `SCREENPIPE_BUN_PATH`,
`SCREENPIPE_SESSION_ID`, `SCREENPIPE_SUBAGENT`, `SCREENPIPE_PARENT_PID`, `SCREENPIPE_WEB_URL`,
`SCREENPIPE_E2E_SEED`, `SCREENPIPE_ENABLE_MDNS`, `SCREENPIPE_FILTER_PII`, `SCREENPIPE_AUDIO_EXCLUSIONS_PATH`,
`SCREENPIPE_MEETING_*`, `SCREENPIPE_PRIVACY_FILTER_*`, `SCREENPIPE_EMBEDDER(_VERSION)`,
`SCREENPIPE_HOST_APP/VERSION`, `SCREENPIPE_TELEMETRY_*`, `SCREENPIPE_SUPPORT_ID`, `SCREENPIPE_CUSTOMER_ID`,
`SCREENPIPE_ORG_ID`, `SCREENPIPE_DEPLOYMENT_ID`, plus third-party `HF_ENDPOINT` (Chinese mirror),
`HF_HOME`, `HF_HUB_CACHE`, `ORT_DYLIB_PATH`, `TESSDATA_PREFIX`, `WEBVIEW2_USER_DATA_FOLDER`,
`NODE_EXTRA_CA_CERTS`, `DEEPGRAM_API_KEY`, `ANTHROPIC_API_KEY`, `DISABLE_MCP_CONFIG_FILTERING`,
`BASH_ENV`, `TAURI_ENV_DEBUG`.
Base vars not observed in the target exe (may live in the CLI binary or scripts):
`SCREENPIPE_IGNORE_DB_LOCK`, `SCREENPIPE_LOG`, `SCREENPIPE_NO_UPDATE_CHECK`, `SCREENPIPE_TRUSTED_HOSTS`,
`SCREENPIPE_REMOTE_*`, `SCREENPIPE_SYNC_*`, `SCREENPIPE_TEAM_API_TOKEN`, `SCREENPIPE_ENTERPRISE_LICENSE_KEY`
(replaced by registry/hash), `SCREENPIPE_FOCUS_URL`. Keep them in Freenpipe (harmless). The base
Windows `setx OLLAMA_ORIGINS *` step is still present in the target (0x49f51e8); unchanged.
Implementation: document in `docs/env.md`; wire the shell-owned ones as described in their items.

### shell-026 CLI changes (changed; CLI is a separate binary)

Evidence: embedded CLI skill doc 0x6a57a13..0x6a5865d (and copy at 0xa07d7e0); `npx -y screenpipe@latest
agent setup` + an unknown-target error (0x49f3109). Confidence: observed (commands, status fields),
inferred (flag defaults).

New/changed subcommands vs base `crates/screenpipe-engine/src/cli/mod.rs`:
- `service install [--mode <recorder|server>]` - installs a per-user background service that starts
  at login and restarts after failures. Default mode `recorder` (capture + indexing + API); `server` mode
  serves existing/synced data without capturing. Re-running switches mode and restarts immediately.
  Windows implementation suggestion: Task Scheduler logon task or HKCU Run + supervisor loop (no admin).
- `service uninstall` - stop and remove. `service status` - report service manager state only.
- `status` (changed) - verdict one of `recording normally`, `serving normally` (server mode, capture
  disabled on purpose), `not capturing`, `needs attention` (health degraded/unhealthy), `stopped` (no
  health endpoint on the port) (exact strings, client-visible); output also covers how fresh the last
  screen and audio captures are, which devices are active, history counts, storage used and the DB
  path. `status --json` fields: `running`, `health.status`, `health.frame_status`,
  `health.audio_status`, `last_capture`, `last_audio_capture`, `storage_size_bytes`, `database_path`
  (base JSON had `running port frames audio_transcriptions last_capture data_size_bytes data_size
  data_dir`; keep those too).
- `diagnose [--dry-run]` - builds a support bundle (logs, health, versions, redacted settings).
  `--dry-run` keeps the bundle on disk with no upload. Freenpipe: never upload; make local the only mode.
- `doctor`, `record`, `search` (`--start "30m ago"`, `--json`, `--content-type`, `--app`,
  `--focused`), `connection list|get|set|test|remove [--json]`, `pipe list|enable|disable|run|logs|
  install|delete|publish|models list`, `db cleanup [--apply]`, `db recover`, `db unlock --force`,
  `vault lock`, `export` - already in base.
- `agent setup <target>` - configure an external coding agent (targets resembling `claude-code`,
  `codex`, `cursor`, `gemini`, `openclaw`, `hermes`) to use screenpipe MCP/skills (MCP dim).
Base touchpoints: `crates/screenpipe-engine/src/cli/{mod.rs,status.rs}`, new `cli/service.rs`,
`cli/diagnose.rs`.

### shell-027 Deep link routes (changed)

Evidence: deep-link handler in `_next/static/chunks/app/layout-3ec03e167c097e07.js` (hosts compared
against `url.host`): `activity artifact auth chat database-recovery database-restart-verification
first-run-agent first-run-summary frame home install-pipe live-view localhost meeting oauth pipe
purchase-successful settings subscription-success timeline view`; query params seen: `api_key app_name
content_type conversation depth end_time error id include_artifacts limit live message name offset
purchase_token q section start_time subscription success target url`; Rust strings
`screenpipe://database-recovery`, `screenpipe://settings?section=storage`. Base hosts: `auth frame
install-pipe meeting subscription-success timeline view` (+ settings/login/join-team elsewhere).
Confidence: observed.

Shell-owned new routes: `database-recovery` -> `start_database_recovery()`;
`database-restart-verification` -> `restart_database_verification()`; `settings?section=<id>` -> open
Home at that section; `home` -> open Home. Others (`activity`, `artifact`, `chat/<id>`, `first-run-*`,
`live-view`, `oauth`, `pipe`) belong to their feature dimensions; the handler should route unknown
hosts to a no-op with a log line. Scheme unchanged (`screenpipe`, beta `screenpipe-beta`); Freenpipe
should consider registering `freenpipe` as an additional scheme while keeping `screenpipe` for
compatibility (open question). Decloud: `purchase-successful`, `subscription-success`, `auth` with
`api_key` are vendor-account flows; disable in Freenpipe.

### shell-028 Opener allowlist (changed)

Evidence: compiled capability regex 0x5a0e0c8: `https?://.*`, `mailto:.*`, `tel:.*`,
`x-apple-reminderkit://.*`, `claude://claude\.ai/new\?q=.*`, `cursor://anysphere\.cursor-deeplink/prompt\?text=.*`,
`codex://threads/new\?prompt=.*` (obsidian not in this regex; native action `obsidian://open?path=` at
0x4999575 is opened from Rust). Confidence: observed.
Implementation: add the three entries to the `opener:allow-open-url` allow list in `capabilities/main.json`
(keep obsidian). Used by flows that hand a prompt off to an external coding agent.

### shell-029 Install layout and bundled runtime libraries (changed, Windows)

Evidence: install directory listing (section 2); exe import table strings `vulkan-1.dll`,
`onnxruntime.dll`, `d3d12.dll`, `d2d1.dll`, `dwrite.dll`, `powrprof.dll`, `mmdevapi.dll`,
`cfgmgr32.dll`, `iphlpapi.dll`, `pdh.dll`, `netapi32.dll` (0xa6a9c6e..0xa6a9d34); ggml-vulkan sources in
whisper-rs-sys (0x6b9b013). Confidence: observed.

Changes vs base `tauri.windows.conf.json`:
- Add resources `DirectML.dll` (DirectML redistributable), `onnxruntime_providers_shared.dll`,
  `vulkan-1.dll` (Vulkan loader, linked at load time because whisper is built with ggml-vulkan) and
  `VulkanRT-License.txt`, `screenpipe_wer_dump_helper.dll` (shell-017).
- FFmpeg major versions in target: avcodec-62, avformat-62, avutil-60, avfilter-11, avdevice-62,
  swresample-6, swscale-9 (import libs `.lib`, `.def`, `.dll.a` are also shipped; unnecessary, do not copy).
- Windows release build should enable cargo features `vulkan` and `directml` (+ `redact-onnx-directml`).
  Because vulkan-1.dll is a load-time import, the app fails to start without it; alternatively delay-load
  it. Freenpipe must ensure the loader DLL is redistributed under its license.
- Runtime-written subfolders inside the install dir (`models\`, `vad\`, `frames\cache_index.bin`) are
  created by the engine when the cwd is the install dir (inferred); Freenpipe should keep models under
  `~/.screenpipe/models` and make sure the NSIS uninstaller removes stray folders.
- `hooks.nsh`: unchanged behaviour is fine; add removal of `$INSTDIR\crash-dumps` on uninstall.

### shell-030 DirectML adapter selection (new; audio dimension consumes it)

Evidence: parakeet_windows.rs strings 0x6b13d9c..0x6b13f20: DXGI adapter enumeration, skip software and
remote adapters, `D3D12CreateDevice` capability check per adapter ordinal, log of dedicated video memory
and UMA flag, a log falling back to CPU when DXGI enumeration fails, `SCREENPIPE_DIRECTML` explicit disable;
DirectML EP fallbacks to CPU with session recycling for parakeet and qwen3-asr (0x6b44f98..0x6b486b2).
Confidence: observed.
Shell part: env kill switch and the adapter probe helper (shared utility, e.g.
`crates/screenpipe-core/src/gpu/windows.rs`): returns the first hardware D3D12-capable adapter ordinal or
None. Audio dim uses it to pick the DirectML device id.

### shell-031 Telemetry (decloud)

Evidence: Sentry DSN at 0x52d98a0 and Sentry JS SDK with a second DSN (0x61e0b77); PostHog key
`phc_...` and `https://us.i.posthog.com` (0x5179e98, 0x517714c); `screenpipe_app::analytics` super
properties incl. disk sizes and UTM fields; engine `piggyback_telemetry` (`piggyback_meeting_summary`,
`piggyback_capture_transition`); crate `screenpipe-telemetry-wire` with device id prefix `sp_device_v1_`
and salt `screenpipe-device-id-v1`; `/api/...` internal telemetry route (`routes\internal_telemetry.rs`);
`SCREENPIPE_DISABLE_TELEMETRY`; command `is_telemetry_disabled_by_env`. Base also embeds Sentry + PostHog.
Confidence: observed.
Freenpipe: remove Sentry (Rust + JS), PostHog, piggyback forwarder and telemetry-wire uploads entirely or
compile them to no-ops. Keep `is_telemetry_disabled_by_env` returning true and `analyticsEnabled`
default false so the UI hides telemetry toggles. Keep local panic log and crash dumps. Base touchpoints:
`src/main.rs` (sentry init, posthog key), `src/analytics.rs`, `Cargo.toml` (`tauri-plugin-sentry`,
`sentry`), `capabilities/main.json` (`sentry:default`), frontend Sentry init.

### shell-032 Remote support logs (decloud)

Evidence: `remote_support_logs` 0x498fcc0..0x498fe97, 0x528a0c2..0x528a1a8: vendor endpoint
`https://screenpipe.com` + `/api/user/log-requests`, headers `X-Device-Id X-Device-Label X-Platform
X-OS-Version X-App-Version`, consent sync, request polling, upload of redacted support logs;
event `remote-support-log-status` with `state` in {`checking`, `syncing`, `ready`, `uploading`,
`sync_error`, `disabled`, ...}; settings `remoteLogCollectionEnabled` (default true in target),
`remoteLogCollectionUserId`; settings UI (@459133) toggle id `remote-log-collection-toggle`.
Confidence: observed.
Freenpipe: do not implement network behaviour. Accept and preserve the two keys (so screenpipe stores
round-trip) but force `remoteLogCollectionEnabled=false`, hide the toggle, and emit
`remote-support-log-status {state:"disabled"}` if the frontend asks.

### shell-033 Feedback / diagnostics upload -> local diagnostics bundle (decloud)

Evidence: `feedback_upload` 0x498c92c..0x498cb20: `/api/logs`, `/api/logs/confirm`, log/screenshot/video
upload slots, a timeout log for listing logs; event `feedback-upload-completed`; `feedback_redact` uses the
vendor privacy-filter enclave (`pii.screenpipe.containers.tinfoil.dev`, labels `person email phone
address company repo handle channel date secret sensitive`); commands `start_feedback_upload`,
`redact_pii_for_feedback`, `read_log_tail`; base `get_log_files` unchanged; `log_files.rs` tails files
with a leading truncation marker that states how many KB are shown (0x499bc81). Confidence: observed.
Freenpipe:
- Implement `read_log_tail(path, max_kb)` locally: return the last N KB of a log in the data dir
  (reject paths outside it; prefix truncation marker).
- Replace upload with `export_diagnostics_bundle` (zip of recent `screenpipe-app.*.log`, engine logs,
  `last-panic.log(.prev)`, `crash-dumps/*.dmp`, `update-*.log`, `recording-recovery.log`, health snapshot,
  settings with sensitive keys redacted) written to Downloads. The target's sensitive-key regex for
  redaction is: keys matching token/api key/secret/password/credential/auth/email/user/account/prompt/
  dir/path/url/host/blocklist/ignored/included, except `port`, `fontSize`, `lastSyncedAt`,
  `onboardingStep`, `windowSize`, `windowPosition` (from `378-...js`); reuse that rule (identifiers only).
- `redact_pii_for_feedback` -> local regex/ONNX redactor only (privacy dim).

### shell-034 Startup account/entitlement gate (decloud)

Evidence: `startup_auth` 0x52d7218..0x52d7d68 (a log for refreshing account access before the startup
gate runs, `/api/user`, refresh fields list, `consumer-startup-auth`, statuses `authenticated/logged_out/
not_required`); main.rs 0x5177520 (a log for skipping server auto-start until account access is granted),
event `app-entitlement-required`, a log for starting a capture-less local read server for trial activation;
0x5129812 (a log for resuming the deferred auto-start once access is verified); base `app_entitled_or_dev()`
gate. Confidence: observed.
Freenpipe: remove the gate. Capture always auto-starts after onboarding; never call `/api/user` at
startup; `SCREENPIPE_STARTUP_AUTHENTICATION_STATUS` is always `not_required` if exported to children.
Base touchpoints: `src/main.rs` ('start_server block), `src/store.rs` entitlement helpers.

### shell-035 Enterprise policy surface (ee-stub)

Evidence: registry `HKLM\SOFTWARE\screenpipe` value `EnterpriseLicenseKey` (32/64-bit views, 0x52b6330);
`enterprise::policy` (`app_ui`, `desktop_app`, `tray_open_app`, `hide_app`, `hide_app_ui`, `ui_mode`,
`enforce_auto_start`, recording authorization granted/revoked); `enterprise::host_identity`;
`enterprise_persistence` (`screenpipe-enterprise-<ver>-x64-persistent.exe`, staged.json, embedded updater
pubkey); env `SCREENPIPE_ENTERPRISE_*`; commands `apply_enterprise_ui_visibility`,
`get_enterprise_host_identity`, `set_enterprise_recording_authorized`, `save_enterprise_team_config`,
`set_enterprise_policy`; files `enterprise-sync-error.log`, `enterprise-takeover.log`,
`.screenpipe-enterprise-tmp`; setting `enterpriseManagedSettings`. Confidence: observed.
Freenpipe: keep the base consumer stubs (`is_app_ui_hidden()` false, etc.). Add no-op command stubs
returning "not available" so the frontend bindings compile; never read HKLM license keys. Preserve
`enterpriseManagedSettings` if present but do not apply it.

### shell-036 UI locale setting and native localization (new; i18n owned elsewhere)

Evidence: settings `uiLocale` (`"system"` or a locale code), `uiLocalizationEnabled`; settings UI
language selector (`data-testid` `language-selector`, id `ui-language`) shown only when localization is
enabled (`page-d9c69007346aaf6a.js` @12769); `screenpipe_app::localization` (0x51b3b60..0x51b3e00):
listens to store change `store://change` key `settings`, dispatches native locale, embedded manifest JSON
with `defaultLocale` `en`, `locales`, per-locale coverage (`ja`: frontend 5663 strings, native 347) and
hashed message ids (`$_hash` 16 hex) with fallbacks; a log for a failed app-menu update during localization.
Confidence: observed.
Shell part: on settings change re-resolve the effective locale (`system` -> OS UI language) and rebuild
tray/app menu and native notification strings from a message table keyed by stable ids. Implement with
English only initially; key strings by Freenpipe's own ids (do not reuse vendor hashes or translations).

### shell-037 `hideThinkingBlocks` removed (removed)

Evidence: key absent from the target struct field list (0x52c8878) and from all frontend chunks.
Confidence: observed (absence in shipped artifacts).
Freenpipe: harmless to keep (it is a UI preference); keep the field for backward compatibility but it may
be dropped from the settings UI if the chat dimension removed the feature.

### shell-038 Data-directory file layout additions (new)

Evidence: `shell_uniq.py` file-name diff (target minus base) plus contexts. Confidence: observed (names),
inferred (exact location when not printed).

Under `~/.screenpipe/` (or active data dir) new files seen in target:
| File | Purpose |
|---|---|
| `crash-dumps/*.dmp` | harvested minidumps (shell-017) |
| `recording-recovery.log` | recording/DB recovery diagnostics (shell-014) |
| `update-install.log`, `update-recovery.log`, `update-attempt.json` | updater diagnostics (shell-016) |
| `disk-pressure-notification-state.json` | low-disk incident state (shell-006) |
| `storage-migration-error.txt`, `storage-migration.json`, `storage.json`, `storage-init.json` | storage migration (shell-015, db) |
| `api-auth-recovery.json` | owner-only recovery copy of the local API auth key (0x5222018, 0x6a7eede; secrets dim) |
| `secrets.sqlite` (+`-journal`) | separate secret store DB counted in disk usage (0x49873c0; secrets dim) |
| `meeting-reminders.json`, `data_sync_cursor.json`, `favorites.json`, `chatgpt-oauth.json`, `.screenpipe-acp-sessions.json`, `.screenpipe-permissions.json`, `workflows-entry.bin` | other dimensions |
| `sqlite-planner-maintenance.json`, `database.quarantine.json`, `.quarantine.reserve.json`, `recovery-manifest.json`, `resolved-quarantine.json` | db dimension |
Still present from base: `store.bin`, `store.bin.last-good`, `store.bin.pre-restore-<ts>`, `.encrypt-store`,
`last-panic.log(.prev)`, `.db_recovery.lock`, `auth.json` (legacy), `vault.meta`, `.vault_locked`, `notifications.json`,
`disk_usage.json` (under `data`), `audio-exclusions.json`, `connections.json`, `pipes/`, `models/`.
Logging: app log remains daily-rolled `screenpipe-app.<YYYY-MM-DD>.log` in the data dir (prefix
`screenpipe-app` observed at 0x49f5118; rotation count not observable, keep base 5). Implementation: a
`paths.rs` constant per file in `screenpipe-core` so all modules agree.

### shell-039 Window inventory additions (changed)

Evidence: show.rs label/title table unchanged (0x514eeb8: `main home search onboarding chat
permission-recovery`, titles `screenpipe`, `ai chat`, `fix permissions`); extra windows
`notification-panel` (base), `notification-inbox` (new, command `show_notification_inbox`, 0x4999de8),
`starred-sessions` (new, `toggle_starred_sessions`/`hide_starred_sessions`, event
`starred-sessions-visibility`, 0x524e740), `shortcut-reminder` (base); suppression rules for enterprise
hidden UI and headless (0x514f83c, 0x514f97e); Home navigation URL `/home?section=<id>` (base);
`search-hidden`, `search-reset`, `originWindowLabel` (0x514f905); command `set_chat_always_on_top`
(new; base only read the setting at window creation). Next.js pages in bundle: home, chat, search,
settings, onboarding, overlay, viewer, notification-inbox, notification-panel, permission-recovery,
shortcut-reminder, starred-sessions, dev/workflows-ux. Confidence: observed.
Shell part: register labels `notification-inbox` and `starred-sessions` in the window helpers with
skip-taskbar, always-on-top, transparent, focusable-on-click (match notification-panel style); make every
`show()` consult headless/enterprise suppression; `set_chat_always_on_top(onTop)` applies
`set_always_on_top` to the live chat window and persists `chatAlwaysOnTop`.

---

## 5. Implementation order (suggested)

1. shell-031, shell-034, shell-016 (decloud) and shell-001/002/003 (settings schema) - foundation.
2. shell-013 (process exit), shell-019 (focus hardening), shell-020/021 (startup), shell-024 (argv
   dispatch), shell-012 (watchdog), shell-011 (health state).
3. shell-005, shell-004, shell-006, shell-007, shell-008, shell-009, shell-023, shell-039.
4. Windows extras: shell-017, shell-018, shell-022, shell-029, shell-030, shell-010.
5. Dependent on other dims: shell-014, shell-015, shell-026, shell-027, shell-036.

## 6. Open questions

- Exact low-disk `threshold_bytes` / `recovery_threshold_bytes`, watchdog check counts and respawn rate
  limits are numeric literals not visible as strings; a native decompile of `disk_pressure.rs` and
  `health.rs` (coordinator-owned REA session) would settle them.
- The predicate deciding new-install privacy defaults (shell-002) is an opaque frontend helper; only its
  effect is known.
- Location where the WER helper DLL writes dumps before harvest (install dir vs local app data) and
  whether HKCU or HKLM is used for `RuntimeExceptionHelperModules`.
- Bundle identifier of v2.7.104 (not recoverable as a standalone string); assumed unchanged
  (`screenpi.pe`). Freenpipe should use its own identifier but keep `~/.screenpipe` paths.
- CLI flag defaults for `service`/`diagnose` (separate binary not available locally).
- `SCREENPIPE_KEEP_NORMAL_PRIORITY` and `SCREENPIPE_MEETING_DETECTOR` exact semantics.
- Whether Freenpipe should also register a `freenpipe://` deep-link scheme.
- The bundled `CHANGELOG.md` in `frontend/` was intentionally not consulted (clean-room).


---

## Verification

Adversarial pass over shell-001..shell-039, checking (a) the BASE (`<repo>`, apps/crates/packages),
(b) the cited target evidence, and (c) duplicates, both inside this spec and against the other dimension specs
(`cloud.md`, `db.md`, `audio.md`, `frontend.md`).

Method:
- Every cited `app_strings.tsv` offset was resolved with `tools/verify_offs.py`. All of them exist and support the
  claim. One minor slip: the "non-local Origin" log line sits at the end of the packed line at 0x6185f70, not at
  its start.
- Frontend claims were checked by direct reads of `378-9f286ebe7c0f0713.js`, `8001-00ed120106af2449.js`,
  `app/layout-3ec03e167c097e07.js`, `app/shortcut-reminder/page-a5341a3196435d05.js` and
  `app/(main)/settings/page-d9c69007346aaf6a.js`, using `tools/shv_jsctx.py`.
- The base was checked with new helpers `tools/shv_base.py`, `tools/verify_shell_keys.py` and
  `tools/verify_shell_cmddiff.py` (310 target commands against 206 in the base). New sweep helpers:
  `tools/shv_appsrc.py` (app-crate module paths), `tools/shv_envdiff.py` (env vars) and `tools/shv_files.py`
  (data-file names).
- REA: the parent graph is `ev_57f23857f31e9207a9f71e1d612412e78f9c5dc0d29af8c68f97372786b6216d`.
  - String seed `webview_renderer_heartbeat` returned no match: `ev_e1b1c0ddf7b32246db47199e93e07b3f33584431d532e6ce7c7b770aa04d9bf6`. The graph does not index this literal, so direct file inspection was used.
  - Module trace of the shortcut-reminder page: retained as `ev_4655819f09955bc38f8cf5f6797edb3a5f531e00b82cae26c4c993ae7784fae6`. It was too large for the transport.

### Verdicts

| Id | Verdict | Note |
|---|---|---|
| shell-001 | confirmed | **Key diff:** 32 settings-struct keys are new: the 31 reported by `verify_shell_keys.py`, plus `headless`, whose base hits were only substrings in comments. **Defaults:** the target defaults in chunk 378 match the spec (`uiLocale:"system"`; `usePiiRemoval`, `autoUpdate`, `filterMusic`, `analyticsEnabled`, `remoteLogCollectionEnabled`, `hideAppInScreenShare` and `keepSearchAvailableAfterQuit` all true; `headless`, `headlessRecordOnly` and `keepComputerAwake` all false). **Base:** `updateChannel` already exists in the base TS (`use-settings.tsx:83,545`) but is missing from the Rust struct. The base TS defaults are `usePiiRemoval:false`, `autoUpdate:false`, `filterMusic:false`, `analyticsEnabled:true`. **Overlaps:** frontend-053 (JS defaults) and cloud-031 (`user.enterprise_account`). shell-001 should own the Rust struct and its field order. **Gap:** the `onboarding` object schema change was missed (see shell-M02). |
| shell-002 | partially_in_base | **Already in base** (`lib/hooks/use-settings.tsx:673-876`): <br>- provider `pi` -> `screenpipe-cloud` and preset id `pi-agent` -> `screenpipe-cloud`<br>- `_chatOpusAppliedForPro`<br>- back-fill of `showChatShortcut`, `startAudioShortcut` and `stopAudioShortcut`; platform override; `deviceId` generation<br>- `restartNotificationsDefaultedOff`, `coreaudioTapMigrationV2`, `_presetsSeededForUser`, `_proCloudMigrationDone`, `_parakeetDefaultMigrationDone`, `_proCloudAudioDefaultsAppliedV2`<br>**Actually new:** `coreaudioTapMigrationV3`, `filterMusicDefaultedOn`, folding of the semantic-context legacy keys into `remoteControlPolicy`/`remoteControlPreferences`, back-fill of `starSessionShortcut`, the enterprise-managed overlay (`enterpriseManagedSettings`), the new-install privacy-defaults predicate, and resolving the store path via `get_screenpipe_base_dir` (all observed in chunk 378). **Note:** frontend-053 reads the predicate as `isPro`. |
| shell-003 | partially_in_base | **Already in base:** `get_cloud_token` (`commands.rs:769`, reads `~/.screenpipe/auth.json`) and `set_cloud_token` (`commands.rs:799`, pushes in memory to the engine only). **New:** the token moves into secret key `cloud.auth_token` and is stripped from `store.bin` (0x49ecdb0); commands `get_screenpipe_base_dir` and `get_active_data_dir`; fail-closed store init (the base falls back to defaults at `main.rs:1099-1102`). **Overlaps:** cloud-013 (token) and db-023 (`get_active_data_dir`). |
| shell-004 | confirmed | **Evidence:** the full headless string set exists (0x499a7c0..0x499a828, 0x514f97e, 0x5177480, 0x5177770, 0x52b64c8..0x52b6aa7). **Base:** no `headless` or `headlessRecordOnly` setting or code. **Note:** the range end 0x52b6b2c actually lands on an unrelated panic-hook string. |
| shell-005 | confirmed | **Evidence:** module `search_only` (app 0x4987800, engine 0x49a7a48); settings switch `keep-search-after-quit` @379032 defaults on. **Base:** no equivalent. The base only has capture-only stop in `capture_session.rs`. |
| shell-006 | confirmed | **Evidence:** engine `disk_pressure.rs` events with `DiskSpaceLowEvent`/`DiskSpaceRecoveredEvent` structs (0x52cb687); app notifications and state file. **Base:** nothing. **Correction:** the config struct next to `thresholdBytes` also carries `checkIntervalSeconds` (0x52b2d38), so `get_low_disk_guard_config` should return both. |
| shell-007 | confirmed | **Evidence:** `set_keep_awake({enabled})` (8001 @43203), thread `screenpipe-keep-awake`, `SetThreadExecutionState`. **Base:** none. **Note:** an app-side module `src\power_awake.rs` (0x49877d0) also exists. |
| shell-008 | confirmed | **Base:** the WDA helper applies only to the overlay (`windows_overlay.rs`, `showOverlayInScreenRecording`). **New:** `hideAppInScreenShare` (all windows), `hideOverlayInScreenRecording` and `get/set_app_screen_capture_protection` (8001 @9808). |
| shell-009 | confirmed | **Evidence:** the anchor list and legacy corner mapping are in the shortcut-reminder page; snooze strings at 0x499a6e0; `star_session` id at 0x499c670. **Base:** none of the keys or commands. **Overlap:** frontend-032 covers the UI side. |
| shell-010 | confirmed | **Evidence:** crate `screenpipe-overlay-win` panic paths (0x5a51c42, 0x62338ba); module `native_overlay_win`; `/ws/metrics` and `/ws/meeting-overlay` (0x499a4ab). **Base:** only the macOS native reminder exists. |
| shell-011 | confirmed | **Evidence:** module `overlay_health`, event `recording-health-state` (also parsed in the shortcut-reminder page), and `BootPhaseSnapshot.cpuCompatMode` (0x61857a0). **Base:** none of the commands. |
| shell-012 | confirmed | **Base blind kill confirmed:** `recording.rs:1059-1095` and `server.rs:191-215` run `netstat -ano \| findstr :port`, then `taskkill` on every non-self PID. **Target:** module `port_conflict` plus the auto-respawn strings. |
| shell-013 | partially_in_base | **Already in base:** `RunEvent::Exit` (`main.rs:1901-1951`) stops capture, then the server, synchronously and spawns the Pi cleanup. There is also a `QUIT_REQUESTED` gate (`main.rs:1888-1899`) and an update restart gate (`updates.rs` `await_safe_restart`/`await_restart_gate`). **New:** module `process_exit` with a bounded budget and force-exit; safe relaunch with `SCREENPIPE_RELAUNCH_HOME_VISIBLE`/`SCREENPIPE_DATA_DIR`; the server-generation drain rule (`recording::server_shutdown`); quit-ignore rules; `restart_for_update(timeoutSecs)` and `restart_after_screen_recording_permission`. **Overlap:** cloud-041 (`restart_for_update`). |
| shell-014 | confirmed | **Evidence:** modules `db_relaunch` (0x5a61680), `db_recovery_notifications` (0x514cf40), `db_self_heal` (0x49ee5ae), `recording::db_wedge` and `recording::retry`; `recording-recovery.log`. **Base:** none. **Cross-dimension overlap:** db-019 already lists the app surface (notification actions, deep link, both commands, recovery log). Assign `db_relaunch` classification, `db_wedge`, retry and keep-UI-up to shell, and the marker formats and recovery engine to db. |
| shell-015 | duplicate | **Duplicate of db-023,** which already has the commands with exact arguments, the status fields, the block reasons and the disable-auto-retry rule. **Unique bits to merge into db-023:** event `storage-migration-activity` and its field list (0x5a63e84), `storage-migration-error.txt` (0x5a63f99), and settings key `pendingStorageMigration` (0x5a644c8). **Wrong claim:** the "needs 2x DB + 512 MiB free" rule is misattributed. The 2 x DB bytes + 0x20000000 check (settings page @540947) lives in the retention/cleanup component (`rA`) and gates the "Compact" button (`/data/compact`, @543897; its label states how much free space the compaction needs while it runs). It belongs to db-025, not to storage migration. |
| shell-016 | duplicate | **Duplicate of cloud-012 + cloud-041.** Between them they cover the endpoint replacement with GitHub Releases, the `stable`/`pre-release` channel mapping including legacy `beta`, the rollback, `restart_for_update`, the `update-attempt.json` marker, and `update-install.log`/`update-recovery.log` with symlink refusal. **Base:** the updater already exists (vendor endpoints in `tauri.prod/beta/enterprise.conf.json`; rollback at `updates.rs:37`). Merge the attempt-marker field list (`from_version to_version ts_epoch_secs ... relaunch_observed`) into cloud-041. |
| shell-017 | confirmed | **Evidence:** the DLL exports and imports were verified directly in `screenpipe_wer_dump_helper.dll` (`crash-dumps` @0x19100, the `OutOfProcessExceptionEvent*Callback` exports, `MiniDumpWriteDump`); app strings at 0x5a0d2d8..0x5a0d502. **Base:** no `crash-dumps`, `RuntimeExceptionHelperModules` or `WerRegister*`. |
| shell-018 | confirmed | **Frontend:** the 1 s non-overlapping heartbeat (layout @140350) and `setHistorySwipeNavigationEnabled(false)` (@139941) are verified. The log for a cancelled Windows swipe-setting update (0x5113020) shows a real Windows code path, which contradicts frontend.md's "no-op on Windows". **Base:** none. **Caveat:** the Rust reaction to a stale heartbeat (reload or recreate) is unobserved. The only Rust trace is the command name in the ACL list (0x5178f64), so treat the reaction as inferred. |
| shell-019 | partially_in_base | **Already in base:** `FocusPayload.target` (`server.rs:89-96`). **New:** `launch_exe`, `launchd_job_label`, the exe/scheme handoff rejection, `--autostart` dedupe, and rejection of non-local Origin/Host (the base uses CORS `*` at `server.rs:235-239`). |
| shell-020 | confirmed | **Base:** `tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None)` (`main.rs:868-871`), i.e. no args. **Target:** `--autostart` (0x5a65bd8), background launch (0x5177380), dev isolation (0x51776d5). **Detail:** dev isolation also skips machine-wide Pi orphan cleanup (0x528f08b). |
| shell-021 | partially_in_base | **Already in base:** `SCREENPIPE_DATA_DIR` is honoured by `screenpipe_core::paths::default_screenpipe_data_dir` (`paths.rs:10-16`), which `config::resolve_data_dir` uses. **Replaced:** `SCREENPIPE_SKIP_ONBOARDING` takes over the base E2E seed `onboarding` (`main.rs:1224-1232`). **New:** the env-driven onboarding skip, and the runtime fs/asset-protocol scope grant for a custom data dir (0x5177285..0x51772cc). |
| shell-022 | confirmed | **Evidence:** AVX2 compatibility strings at 0x52d96b9, 0x5176fc9 and 0x4993189; `cpuCompatMode` at 0x61857a0. **Base:** no `is_x86_feature_detected` and no `cpuCompatMode`. |
| shell-023 | confirmed | **Base tray** (`tray.rs:634-737`): read-only monitor rows; five plain status strings; `recreate_tray` is dead code except on macOS (`tray.rs:456`). **Target:** `toggle_vision_device_` (0x5a3f562), `/vision/device/start\|stop` (0x5a3ed70), the compound status strings (0x4994c0d, 0x5a3f924), the open-first-summary tray item (0x5a3f448), retrying `tray_menu_update` (0x5a3fce7), and recreation at the rightmost position (0x51794f7). **Note:** hidden-UI item suppression is base behaviour, correctly labelled. |
| shell-024 | confirmed | **Base:** the only early argv mode is `--check-arc-automation` (macOS). **Target:** three new helper flags (0x499143d, 0xa0cab68, 0xa1170fe/0xa117260). **Overlap:** feature semantics are already in agents_pipes.md, so the shell item rightly limits itself to argv dispatch. |
| shell-025 | confirmed | **Inventory is accurate, with corrections:** <br>- `SAVE_RESOURCE_USAGE` is already in the base (`crates/screenpipe-engine/src/resource_monitor.rs`).<br>- `SCREENPIPE_DATA_DIR` is already honoured by the base app through core paths, so it is not newly honoured.<br>- The inventory missed `SCREENPIPE_FFMPEG_PATH` (0xa0e4df0, startup health diagnostics; see shell-M01).<br>**Confirmed absent from the target:** the base-only vars (`SCREENPIPE_IGNORE_DB_LOCK`, `SCREENPIPE_NO_UPDATE_CHECK`, `SCREENPIPE_TRUSTED_HOSTS`, `SCREENPIPE_TEAM_API_TOKEN`, `SCREENPIPE_ENTERPRISE_LICENSE_KEY`). |
| shell-026 | confirmed | **Evidence:** the CLI skill document (0x6a57b17..0x6a584e1, with a copy at 0xa07dedd) has `service install [--mode server]`, `service uninstall\|status`, `diagnose --dry-run`, the status verdicts and the JSON fields; `agent setup` is at 0x49f313b. **Base:** `cli/mod.rs` has no Service, Diagnose or Agent subcommand. |
| shell-027 | duplicate | **Duplicate of frontend-045,** which has the same host switch plus `scheduled`, `workflows` and `calendar-connections` from other chunks; shell-027 missed those three. **Base:** the hosts are exactly `auth frame install-pipe meeting subscription-success timeline view` (`components/deeplink-handler.tsx`). **Shell-owned piece:** the Rust strings that generate `screenpipe://database-recovery` and `screenpipe://settings?section=storage`. Keep that note in shell-014/006. |
| shell-028 | confirmed | **Base:** `capabilities/main.json` allows http(s), mailto, tel, reminderkit and obsidian. **Target:** the compiled regex (0x5a0e0c8) adds `claude://`, `cursor://` and `codex://` and has no obsidian entry. |
| shell-029 | confirmed | **Verified against the install directory listing.** The base `tauri.windows.conf.json` ships ffmpeg, onnxruntime, openblas, vcredist, assets and bun. New files: `DirectML.dll`, `onnxruntime_providers_shared.dll`, `vulkan-1.dll`, `VulkanRT-License.txt` and `screenpipe_wer_dump_helper.dll`. **Overlap:** audio-019/020 for the Vulkan and DirectML runtime. **Base nuance:** the base already globs `ffmpeg\lib\*`, which is why the target ships `.lib`/`.def`/`.dll.a`. Dropping them is a Freenpipe choice. |
| shell-030 | duplicate | **Duplicate of audio-020** (DXGI adapter probe, `D3D12CreateDevice` check, `SCREENPIPE_DIRECTML` switch, CPU recovery; same offsets 0x6b13d30..0x6b13f5f). **Semantics differ:** audio-020 reads the env as "anything other than true/1 disables", while shell-030 reads it as "a false value disables". Settle this in audio-020. |
| shell-031 | duplicate | **Duplicate of cloud-010 (PostHog), cloud-011 (Sentry), cloud-040 (piggyback and other telemetry) and cloud-003 (telemetry-wire channel),** including `is_telemetry_disabled_by_env` -> true and `analyticsEnabled` defaulting to false. |
| shell-032 | duplicate | **Duplicate of cloud-034,** with the same evidence (0x498fcc0.., 0x528a0c2..) and the same Freenpipe decision (force off, hide the toggle). |
| shell-033 | duplicate | **Duplicate of cloud-036 and frontend-046.** Both already specify a local `read_log_tail` plus a local zip instead of an upload. **Correction:** the target signature is `read_log_tail(path, maxBytes)`, not `max_kb` (8001 @33128). **Unique to shell-033** (merge into cloud-036): the bundle contents list (crash dumps, update/recovery logs, health snapshot) and the sensitive-key redaction rule. **Conflict:** the export location differs (`<data_dir>/feedback/` in cloud-036, Downloads in shell-033). Pick one. |
| shell-034 | duplicate | **Duplicate of cloud-014 (base gate removal) + cloud-030 (startup auth stub; `SCREENPIPE_STARTUP_AUTHENTICATION_STATUS=not_required`) + cloud-031.** **Base:** the gate already exists (`main.rs:1431-1436` emits `app-entitlement-required`; `store.rs:1283` `app_entitled_or_dev`). |
| shell-035 | duplicate | **Duplicate of cloud-001/002/003/043.** **Factual error in shell-035:** "keep the base consumer stubs" is wrong. The base `enterprise_policy.rs`, `enterprise_install_metadata.rs` and `enterprise_sync.rs` are `#[path]` shims into the removed `ee/` directory, so no stub exists. Callers such as `is_app_ui_hidden`/`is_tray_item_hidden` (`tray.rs:6`, `window/show.rs:407`) need original stubs, which cloud-001 specifies. |
| shell-036 | confirmed | **Evidence:** `store://change` key `settings` listener (0x51b3b36), `native-localization` dispatch (0x51b3d00), manifest with `defaultLocale`/`ja` coverage (0x51b3e00), and the settings `language-selector` (@13143). **Base:** no localization layer and no `$_hash` message ids. |
| shell-037 | confirmed | **Base:** `hideThinkingBlocks` is present in the base TS (4 files) and in the Rust `store.rs` (`hide_thinking_blocks`). **Target:** zero hits in `app_strings.tsv` and in every frontend chunk. |
| shell-038 | confirmed | **Base:** none of the listed new names. **Overlap:** db-028 for the db and storage files. **Missing:** `health-startup.json` (shell-M01). |
| shell-039 | partially_in_base | **Already in base:** enterprise hidden-UI suppression inside `show()` (`window/show.rs:407`, `window/panel.rs:84`) and the `chatAlwaysOnTop` setting (read at window creation). **New:** windows `notification-inbox` (0x4999dd0) and `starred-sessions` (0x524e740), headless suppression (0x514f97e), and the live command `set_chat_always_on_top({onTop})` (8001 @41876). **Overlaps:** frontend-030 (inbox UI) and meetings_memory (starred sessions). |

**Totals:** 24 confirmed, 6 partially_in_base, 9 duplicate, 0 already_in_base, 0 unsupported.

### Missed items

**shell-M01 (new): startup health diagnostics report `health-startup.json`.**

Evidence: 0xa0e4df0..0xa0e5288, crate path `crates\screenpipe-core\src\health_diagnostics.rs`.

What the target records:
- Startup phase: `phase`, `phase_since`, `engine_started_at`.
- `preflight` checks, each with `check`, `result`, `detail`, `remedy`. Checks seen: `data_dir_writable`, `path_has_ffmpeg`, `env_overrides`.
- `last_failed_start` with `at`, `reason`, `exit_code`. One message reports that the previous startup never reached ready without an observed termination cause.
- `dependencies` for ffmpeg/ffprobe: `path`, `source`, `version`, `version_probe_error`, `install_in_progress`, `install_retry_after`, `last_spawn_error`, `last_spawn_error_at`. The paths are probed from the env vars `SCREENPIPE_FFMPEG_PATH`, `ORT_DYLIB_PATH`, `HF_HOME` and `HF_HUB_CACHE`.
- Runtime component states for `transcription`, `vad`, `speaker_segmentation` and `speaker_embedding`. Possible states: `ok`, `init_failed`, `init_timed_out`, `missing`, `not_required`, `unknown`, `degraded`. Also `degraded_features` and a `crash_dump_helper` status.
- A `ProcessSnapshot` with `hd_encoders_running`, `audio_encodes_running`, `audio_encodes_last_minute`, `audio_encode_failures_last_minute`, `extractions_running`, `extractions_queued`, `compaction_running`, `stalls_total`, `last_stall_at`.

Who owns it:
- audio-021 cites this file but defers it to a "health dimension" that does not exist.
- No base equivalent: no `health_diagnostics`, `ProcessSnapshot` or `version_probe_error`.
- The shell should own it, because it feeds `retry_screenpipe`, the diagnostics bundle and `cpuCompatMode`.

Size: M.

**shell-M02 (changed): `onboarding` object schema in `store.bin`.**

Evidence: the serde field list at 0x52ae2c1 and 0x52c9740. The target `OnboardingStore` keeps the base fields (`isCompleted`, `completedAt`, `currentStep`) and adds:
- `firstRunSummaryPhase`
- `firstRunSummaryStartedAt`
- `firstRunSummaryChatId`
- `firstRunSummaryNotificationSentAt`
- `firstRunSummaryNotificationId`
- `firstRunSummaryError`
- `firstRunSummaryTelemetryVersion`
- `trialActivationFreshInstall`

Coverage gaps:
- The base `store.rs:496-507` has only the three original fields.
- shell-001 says the other top-level store keys are unchanged, which is wrong for `onboarding`.
- frontend.md covers only the `get_onboarding_status` read side.
- Freenpipe should add these as optional serde fields so that screenpipe-written stores round-trip. `trialActivationFreshInstall` is accepted but ignored (cloud-032).

Size: S.

### Reconciliation actions for the coordinator

1. **Drop duplicates:** drop shell-015/016/027/030/031/032/033/034/035 in favour of db-023, cloud-012/041, frontend-045, audio-020, cloud-010/011/040, cloud-034, cloud-036/frontend-046, cloud-014/030 and cloud-001/043. First merge the unique details listed in the notes above into those items.
2. **Fix shell-006:** the low-disk config also carries `checkIntervalSeconds`.
3. **Move the free-space rule:** the 2x DB + 512 MiB check belongs to `/data/compact` (db-025), not to storage migration.
4. **Fix the shell-035 premise:** the base has no enterprise stubs. The `#[path]` shims point into the deleted `ee/` directory, so the stubs are a build blocker.
