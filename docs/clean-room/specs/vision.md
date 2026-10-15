# Freenpipe delta spec: screen / vision capture pipeline (`vision`)

Base: Freenpipe @ upstream `892199f` (MIT, ~v2.5.27). Target: installed screenpipe v2.7.104 (Windows, Tauri 2.11, engine embedded in `screenpipe-app.exe`).
Evidence is static only. Hex values like `0x49aa31c` are file offsets of string runs in `<re-workspace>/app_strings.tsv`
(column 1). Frontend paths are relative to `<re-workspace>/frontend/`. Everything here is paraphrased; only interop
identifiers (routes, JSON keys, SQL, settings keys, env vars, file names) are quoted exactly. SQL DDL is reproduced verbatim where it
is needed for on-disk compatibility.

REA evidence used:
- `ev_57f23857f31e9207a9f71e1d612412e78f9c5dc0d29af8c68f97372786b6216d` - parent JS application analysis of `frontend/` (partial graph; semantic budget 0, so feature traces returned no matches, see `ev_aaa3b42434357f23e0dee4c72719e1808051b8db8e144773b2ff659f642aae19`).
- `ev_d205fc1fda693bff953c410e8912bcbd3a6e1ff1e195914f6446ed80da215f84` - summary view (394 JS files, 2112 modules).
- `ev_5e2965250ccf21bf81f6c79a2ca8b022def4f7f7bbb9afa2fae0e236d783730f` - module `_next/static/chunks/3308-e676c712bfd27088.js` (thumbnail / preview-media URL helpers).
- `ev_df2f0e5751cdd069e53296885abe98dfbea61b17917792afacbbfacad43df260` - module `_next/static/chunks/app/(main)/home/page-c18385b042b3382e.js` (recording popover monitor toggles, activity preview samples).
- `ev_46bc6f414f9baaca64f92d089aee37621290580055d14430bae159403207e138` - module `_next/static/chunks/app/(main)/settings/page-d9c69007346aaf6a.js` (domain rules, scroll capture, PII column options, video quality).

Helper used to slice the strings file: `<re-workspace>/tools/vdump.py` (`range` / `grep` modes); intermediate dumps in `tools\out\v_*.txt`.

---

## 0. Important clarifications before implementing

1. **`capture/hdassets` is not a real identifier.** At `0x4995449` the run reads `/vision/device/status` + `user_disabled` + `/capture/hd` +
   `assets/screenpipe-logo-tray-white.png` ... Rust packed the route literal `/capture/hd` next to the tray icon asset paths. There is no
   `hdassets` route, table, directory or setting. Nothing to implement.
2. The target ships **no `record` CLI flags** for vision inside the app binary (no clap help for `--disable-vision`, `--monitor-id`,
   `--visual-change-threshold`, etc.; only `min_capture_interval_ms` appears, inside a log line at `0x49aa761`). Configuration flows
   from the settings store (`store.bin`, serde camelCase keys at `0x5a42a38`). Keep the base CLI flags (harmless), but every new knob
   in this spec must be reachable from `RecordingSettings` (`crates/screenpipe-config/src/recording.rs`).
3. The target's large **hybrid Parquet storage layer** (`frame_payloads`, `payload_files`, `storage_metadata`, bulk element store) is a
   storage-subsystem feature. This spec records what the vision pipeline must know about it (item `vision-hybrid-payload-storage`);
   the storage/db dimension owns the full migration machinery.
4. Telemetry (`vision_capture_wedge_restart`, `vision_pipeline_health` PostHog events) and the cloud PII enclave (`tinfoil`) must not
   be reproduced as network calls (CLAUDE.md "no phoning home").

---

## 1. Inventory: base vs target

| Area | Base (Freenpipe @ 892199f) | Target v2.7.104 (evidence) | Delta id |
|---|---|---|---|
| Capture trigger model | Event-driven loop `event_driven_capture.rs`: triggers `app_switch window_focus click typing_pause scroll_stop key_press clipboard visual_change idle manual`; defaults min 200 ms, idle 30 s, visual check 3 s, threshold 0.05, JPEG 80; poll 250 ms; 15 s capture timeout; warm cadence 5 s | Same trigger names (`0x49aa31c`), same loop messages; adds frozen-stream rebuild, soft-checkpoint deferral, a11y worker suspend, pause-before-focus-probe release, screenshots-disabled reasons | `vision-capture-loop-refinements` |
| Loop liveness | Heartbeat via `record_capture_attempt` only | Named loop stages + heartbeat; monitor-watcher "wedge watchdog" restarts VisionManager | `vision-capture-wedge-watchdog` |
| Visual-change probe / exclusion probe | Inline `capture_monitor_image` + `get_excluded_sck_window_ids` | Separate `visual_probe.rs`, `capture_exclusions.rs` with timeouts | `vision-probe-timeouts` |
| Per-monitor pause | Not present (audio has `is_user_disabled`) | `/vision/device/start|stop|status`, `VisionDeviceControlRequest`, `VisionDeviceStatusEntry`, user-paused monitors skipped by manager + watcher | `vision-device-control` |
| Display arrangement | Only per-monitor bounds in memory | `display_layout` table, persisted at startup and on change | `vision-display-layout` |
| Windows capture backend | `wgc_capture.rs` wraps `xcap::VideoRecorder` (166 lines); per-frame xcap fallback after 3 init failures | Native WinRT/D3D11 Graphics Capture (free-threaded frame pool, border/cursor off, `SetMinUpdateInterval`, device-removed recovery, WARP fallback, staging readback); `monitor/windows.rs` with `EnumDisplayMonitors` | `vision-wgc-native-capture` |
| Corrupt frames | none | `frames_corrupt_black`, `frames_corrupt_green` counters | `vision-corrupt-frame-guard` |
| Metrics / health | `PipelineMetrics` 20 fields; health `frame_status` | +`ocr_empty`, `ocr_gate_*`, `frames_dropped_timeout/error`, `silent_loss(_rate)`, `capture_loop_heartbeats`, `last_capture_loop_heartbeat_ts`, corrupt counters; health `vision_reason`, `loop_stage`, `loop_stage_age_secs`, `recording_coverage` | `vision-pipeline-metrics` |
| OCR scheduling | OCR whole frame when a11y empty/thin/terminal | Text-region detection gate (`text_regions.rs`), skip OCR when no regions, crop-OCR on region union / window crop | `vision-ocr-gate` |
| OCR storage | `ocr_text` table + elements | `ocr_text` dropped; `frames.text_json`; OCR elements rows | `vision-ocr-unify-frames` |
| Windows OCR | `perform_ocr_windows` async inline; language fallback exists | Dedicated `screenpipe-windows-ocr` worker with bounded queue; image-size validation; OCR-unavailable health reason + app notification; tesseract binary discovery / per-session disable | `vision-windows-ocr-worker` |
| A11y walk execution | `spawn_blocking` per capture | `tree_walker_worker.rs` per-monitor worker with timeout, restart-and-retry-once, suspend/resume | `vision-tree-walker-worker` |
| Windows UIA tree | Fresh traversal per capture (`tree/windows.rs`, 1026 lines) | "Retained capture": UIA event subscriptions + retained tree + bounded resync; `capture_diagnostics.rs`; fragile-provider skip | `vision-uia-retained-capture` |
| Window resolution / exclusion reasons | ignore/include patterns, incognito, hard-coded excluded apps | Structured skip reasons, UWP host resolution, transient shell-window skip, extension-popup skip | `vision-windows-window-resolution` |
| URL filters | `ignoredUrls: Vec<String>` domain-boundary match | `ignoredUrls: Vec<UrlRule>` (string or `DomainRule{domain,includeSubdomains,excludedSubdomains}`), new `includedUrls` allowlist, unverifiable URL = not recorded | `vision-url-domain-rules` |
| Screenshot-only toggles | `screenshot_disabled` only via power profile | `disableScreenshots` setting, managed policy, health reasons | `vision-disable-screenshots` |
| Capture width | `maxSnapshotWidth` + mac SCK width from `videoQuality` | adds `captureMaxWidth` | `vision-capture-max-width` |
| Scroll capture cadence | `captureScroll` only | `recordingDetail` = `auto|low_impact|balanced|more_detail` | `vision-recording-detail` |
| Thumbnails | none (full frame `/frames/:id` only) | `GET /frames/:frame_id/thumbnail` (JPEG, cache headers, nearby fallback) | `vision-thumbnail-endpoint` |
| Preview samples | none | `GET /frames/preview-samples` | `vision-preview-samples` |
| Preview media | none | `GET /frames/preview-media/:video_chunk_id` (byte-range MP4) | `vision-preview-media` |
| Element API | `/elements`, `/frames/:id/elements` JSON only | `format=json|csv|tsv|outline|automation|preferred`, `fields=`, `state`, automation snapshot refs | `vision-elements-formats` |
| Snapshot compaction | libx265 software, `ultrafast`, CRF by quality | Encoder probe: `hevc_videotoolbox`, `hevc_nvenc`, `hevc_qsv`, `hevc_amf`, `libx264`; env `SCREENPIPE_COMPACTION_ENCODER`; pause/defer handling; partial index | `vision-compaction-encoder` |
| Text/element payload storage | All text resident in SQLite | Hybrid: `frame_payloads`/`payload_files`/`storage_metadata`, Parquet `search.parquet`/`detail.parquet`, bulk elements virtual table | `vision-hybrid-payload-storage` |
| PII text redaction surfaces | `ocr_text`, `audio_transcriptions`, `frames.accessibility_text`, `ui_events.text_content` | + `elements.text/properties`, `frames.full_text`, `accessibility_tree_json`, `window_name`, `browser_url`, `text_json`, multi-column `ui_events`; `piiRedactionColumns`; partial indexes | `vision-redaction-surfaces` |
| PII extras | none | `piiRedactionPseudonyms` + `.redaction/pseudonym.key`; a11y-context secret detection; secret continuation; CPU-budgeted worker; AI failure circuit | `vision-redaction-pseudonyms-a11y` |
| PII models | `rfdetr_v12.onnx`, `v45_phase5_pruned` | `rfdetr_v38.onnx`, `v50_distilled6l` | `vision-redaction-models` |
| Cloud PII backend | `piiBackend=tinfoil` adapter present | still present (`pii.screenpipe.containers.tinfoil.dev`) | `vision-tinfoil-decloud` |
| Capture analytics | PostHog analytics module | `vision_capture_wedge_restart`, `vision_pipeline_health` | `vision-telemetry-decloud` |
| History gate | none | `history_access_limited` (24-hour) on frame/element routes | `vision-history-access-stub` |
| HD capture | meeting/timer/prewarm sessions (macOS recorder only) | + starred-session `hd_requested` binding | `vision-starred-hd` |
| Frame diff | `FrameComparer` downscale/4 + hash | grayscale thumbnails with fallible allocation, structured failure logging | `vision-frame-comparison-hardening` |
| Unchanged | HD routes `/capture/hd*`, `/vision/list`, `/vision/status`, `/vision/metrics` paths, focus-aware Active/Warm/Cold, frame linker, DRM pause, schedule pause, hot frame cache, video quality presets (Low 1280 / Balanced 1920 / High 3840 / Max native - settings page), HD recorder macOS-only (`0x49a7988`) | | - |

---

## 2. Delta items

### vision-device-control - per-monitor pause/resume API (new, M)

**Interop identifiers (observed):**
- Routes (route table at `0x5257ca8`): `POST /vision/device/start`, `POST /vision/device/stop`, `GET /vision/device/status` (also `0x5a3ed70`, `0x621d6c8`).
- Handler file `crates\screenpipe-engine\src\routes\vision.rs` (`0x59fb855`).
- Request type `VisionDeviceControlRequest` with one field `monitor_id` (`0x5155c68`: the serde descriptor reports a one-element struct; `0x59fb96f`).
- Response type `VisionDeviceControlResponse` with `message` (`0x6a726a0`); the success message says screen recording was paused or resumed on the given monitor; the failure messages say pausing or resuming the monitor failed (`0x59fb8c9..0x59fb948`).
- Status entry `VisionDeviceStatusEntry`, doc comment (paraphrased): the recording state of a single monitor, as the popover shows it (`0x6a72278`); fields observed: `active`, `user_disabled` (`0x59fb96f`); frontend reads `id`, `name`, `user_disabled` (home page, `ev_df2f0e57...`).
- Frontend use: home page recording popover builds one row per status entry with fields `name`, `fullName` (same as `name`), `kind` = `"monitor"`, `active` (the negation of `user_disabled`) and `id`, and toggles with `POST /vision/device/stop` when active else `/vision/device/start`, body `{"monitor_id": <id>}`. Tray menu item kind `monitor_toggle` (`0x5a3f748`) and tray health polling of `/vision/device/status` (`0x4995449`).

**Behaviour (target):**
- `VisionManager` keeps a set of user-paused monitor ids. Logs record each user pause and user resume of vision recording with the monitor id (`0x59fae21`, `0x59faee6`), and a skipped start for a monitor that is user-paused (`0x59fae94`).
- Stop = add to set, then stop that monitor's capture task (same 3 s abort wait as base). Start = remove from set, then `start_monitor` (fails with a monitor-not-found error if the monitor is gone, `0x59faed1`).
- `start_monitor` refuses (Ok, no-op) for a user-paused id.
- Monitor watcher skips user-paused monitors when reconnect/new-monitor logic would restart them, logging that the monitor was paused from the recording popover (`0x49a9323`).
- `VisionManager::start` success accounting: if every selected monitor is user-paused, start succeeds and logs that the manager is running with all N selected monitors deliberately paused by the user (`0x49a9c84`); same for the stale-allowlist fallback path (`0x49a9d76`). Zero-enumerated case has its own message saying the watcher will classify (display asleep, enumeration failure or permission revoked) (`0x49a9e41..0x49a9ed7`).
- Status lists every currently enumerated monitor (subject to allow-list? inferred: all connected monitors that pass `is_monitor_allowed`), `active` = capture task running and not user-paused, `user_disabled` = in the user-paused set. When vision is disabled return `[]` (frontend dev mock returns `[]`, `frontend/_next/static/chunks/app/layout-3ec03e167c097e07.js`).

**Data shapes:**
```
POST /vision/device/start|stop   body {"monitor_id": u32}
  200 {"success": true, "message": "..."}         (success field inferred from audio twin)
  4xx/500 {"success": false, "message": "..."}
GET /vision/device/status  -> [ {"id": u32, "name": string, "active": bool, "user_disabled": bool} ]
```
**Persistence:** none observed; treat as in-memory (inferred - mirrors base audio `user_disabled_devices`). A restart clears pauses.

**Base files to touch:** `crates/screenpipe-engine/src/vision_manager/manager.rs` (user-paused `DashSet<u32>` or `RwLock<HashSet>`, `pause_monitor`, `resume_monitor`, `is_user_paused`, status snapshot), `vision_manager/monitor_watcher.rs` (skip paused ids), new `crates/screenpipe-engine/src/routes/vision.rs`, `routes/mod.rs`, `server.rs` (register routes in the oasgen `Server` like `/audio/device/*`). Mirror shapes of `routes/audio.rs` `DeviceStatusEntry` / control handlers. App: home popover + tray (other dimensions) consume it.

**Notes:** keep ids as numeric `SafeMonitor::id()`. Consider also accepting `stable_id` later; not required for interop.

---

### vision-display-layout - persisted display arrangement (new, M)

**DDL (migration description "capture action fidelity", `0xa2a75f0`; verbatim, `0xa2a7b1b..0xa2a7d31`):**
```sql
ALTER TABLE ui_events ADD COLUMN element_ancestors TEXT;
CREATE TABLE IF NOT EXISTS display_layout (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- why this row exists: 'startup' | 'change'
    reason TEXT NOT NULL DEFAULT 'change',
    -- canonical JSON array, displays sorted by id:
    -- [{"id":1,"stable_id":"...","name":"Built-in","x":0,"y":0,
    --   "width":1512,"height":982,"is_primary":true}, ...]
    layout_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_display_layout_timestamp
    ON display_layout(timestamp);
```
(`element_ancestors` belongs to the ui-events dimension; add the column in the same migration for schema parity.)

**Queries (`crates\screenpipe-db\src\db\display_layout.rs`, `0x52435d0`):** `INSERT INTO display_layout (reason, layout_json) VALUES (?1, ?2)` and `SELECT layout_json FROM display_layout ORDER BY id DESC LIMIT 1`.

**Behaviour:**
- Purpose (paraphrased migration comment): click coordinates are global-desktop points while element bounds are per-monitor normalised; without the arrangement, joins on multi-monitor setups were poor. Each row snapshots every display: `id`, `stable_id`, `name`, `x`, `y` (global origin), `width`, `height` (the sizes the capture backend gives), `is_primary`.
- Written by the monitor watcher at startup (`reason='startup'`) and whenever the canonical JSON differs from the last row (`reason='change'`). Canonical = displays sorted by id, compact JSON, stable key order. Logs cover a successful snapshot write and a write that failed or timed out; both failure logs say the write is retried on the following watcher pass (`0x49a9770`, `0x49a97cd`). So: bounded timeout (inferred ~5 s), on failure keep the pending snapshot and retry at the next 5 s watcher poll.
- Retention / delete-range: `DELETE FROM display_layout WHERE timestamp BETWEEN ?1 AND ?2 AND id NOT IN (SELECT id FROM display_layout WHERE timestamp BETWEEN ?1 AND ?2 ORDER BY timestamp DESC, id DESC LIMIT 1)` i.e. delete the range but keep the newest layout row in that range so later data still has a layout (`0x5140a78..0x5140dcc`).

**Base files:** new migration in `crates/screenpipe-db/src/migrations/` (name e.g. `2026062x000000_capture_action_fidelity.sql`; exact version unknown, must sort after base migrations), `crates/screenpipe-db/src/db.rs` (insert/latest), `vision_manager/monitor_watcher.rs` (compute + persist), delete-range in `routes/data.rs`/`db.rs` and retention.

---

### vision-capture-wedge-watchdog - loop stages + wedge restart (new, M)

**Observed identifiers:**
- Stage names (packed run `0x49ad5c8`, repeated `0x52594ff`): `heartbeat`, `focus-gate`, `warm-wait`, `cold-wait`, `pause-gate`, `release-stream`, `invalidate-streams`, `exclusion-probe`, `visual-probe`, `trigger-wait`, `capture`, `hot-cache-push`, `trigger-drain`.
- Health fields: `loop_stage`, `loop_stage_age_secs` (`0x5256cd6`); metrics `capture_loop_heartbeats`, `last_capture_loop_heartbeat_ts` (`0x522e3cc`).
- Watchdog messages (`0x49a8fa0..0x49a91f0`): two stall classes, labelled `still-attempting wedge` and `gone-silent stall`; the stage part shows either a stage that was never entered or how many seconds it has been frozen in. The main diagnostic names the monitor and class, then reports `status=Running`, the stage info, the loop heartbeat age, the last-attempt age and how long there has been no terminal capture outcome, and says the VisionManager is being restarted. Follow-up watchdog logs cover a failed vision stop, a successful restart after a silent wedge, and a failed restart. Analytics event `vision_capture_wedge_restart` with `attempt_age_secs`, `loop_heartbeat_age_secs` (`0x49a9108`) - see decloud item.

**Behaviour:**
- Each capture loop publishes (per monitor) the current stage name and the instant it was entered, plus a loop heartbeat counter/timestamp bumped every iteration (`heartbeat` stage at loop top).
- Monitor watcher (5 s poll) inspects each running monitor while `VisionManagerStatus::Running`:
  - "still-attempting wedge": a capture attempt started (attempt timestamp recent or advancing) but no terminal outcome (persisted / dedup-skip / dropped) for longer than the wedge threshold.
  - "gone-silent stall": loop heartbeat itself is older than the threshold (loop stuck inside a stage).
  - When either holds and the monitor is not in a legitimate pause (locked, power pause, DRM, schedule, Cold focus wait, user-paused), log the diagnostic, `stop()` then `start()` the VisionManager.
- Thresholds are not in strings. Recommended (inferred): wedge if no terminal outcome for 120 s while attempts continue; stall if heartbeat age > 90 s and stage is not `cold-wait`/`pause-gate`/`trigger-wait`. Cool-down 5 min between restarts per monitor.
- `/health` exposes the oldest/most-stuck monitor's `loop_stage` and `loop_stage_age_secs`.

**Base files:** `event_driven_capture.rs` (stage setter calls around each block), `crates/screenpipe-screen/src/metrics.rs` (heartbeat counters; per-monitor stage cell can live in a small `LoopStageTracker` in engine), `vision_manager/monitor_watcher.rs` (watchdog), `routes/health.rs` (fields).

---

### vision-probe-timeouts - visual probe and exclusion probe isolation (new, S)

**Observed:** `crates\screenpipe-engine\src\visual_probe.rs` (`0x49a7f31`): a log for a visual-change probe that hit its timeout on a monitor, blaming an unresponsive ScreenCaptureKit; the check is skipped and the loop keeps running (`0x49a7fe8..0x49a8041`). `crates\screenpipe-engine\src\capture_exclusions.rs` (`0x49aa1da`): logs for a failed window-exclusion refresh task and for a refresh that overran its deadline, blaming an unresponsive WindowServer (`0x49aa2a0..0x49aa2ff`).

**Behaviour:** The visual-change screenshot+diff (Active and Warm paths) and the excluded-window enumeration each run under a timeout (spawned task). On timeout or join error: log, skip this check, keep the previous excluded-id cache, continue the loop. Wording is macOS-flavoured; implement the wrappers cross-platform. Suggested timeout: 3 s probe, 2 s exclusion refresh (inferred).

**Base files:** extract from `event_driven_capture.rs` (functions around `should_run_visual_check` and the Warm branch) into `visual_probe.rs` and `capture_exclusions.rs`.

---

### vision-capture-loop-refinements - event loop behaviour changes (changed, M)

All in `event_driven_capture.rs` region `0x49aa418..0x49aada5`. Base already has nearly every message; these are the differences:

1. **Frozen persistent stream:** a log reports that a monitor's capture stream has delivered no new frame for roughly N seconds and that the persistent stream is being invalidated so it gets rebuilt (`0x49aa9bc..0x49aa9f6`). Track the last time the persistent capture delivered a *new* frame (WGC FrameArrived / sequence number). If the backend keeps returning the same cached frame for longer than a threshold (inferred 30 s) while content should be changing (any capture attempted), call `release_capture_stream()` so the next capture recreates the session.
2. **Timeout wording/metrics:** the timeout log (with trigger and monitor) now says the frame was dropped and names the probable causes: a blocked DB write, an exhausted write pool, or slowness in the a11y/OCR stage (`0x49aab69..0x49aaba6`). Increment `frames_dropped_timeout`; errors increment `frames_dropped_error` (see metrics item). Keep 15 s timeout.
3. **Soft checkpoint deferral:** a log notes that a soft checkpoint was deferred, with `trigger` and `monitor` fields (`0x49aabfd`). Workflow-checkpoint triggers (clipboard/key/focus kinds that bypass debounce in base) that arrive while a capture is still in flight or within the debounce window are deferred (held) and fired on the next eligible tick rather than dropped or bypassing debounce unconditionally (inferred semantics: "soft" = non-urgent checkpoint). Keep the correlation ids attached to the deferred trigger.
4. **Pause entry order:** the pause-entry log lists the `locked`, `power_paused`, `drm` and `schedule` flags and states that the capture stream is released ahead of the focus probes (`0x49aaccc..0x49aacf1`). Release the OS stream at pause entry before any focus-tracker probing; also suspend the a11y worker, with a warning when that suspend fails on pause (`0x49aad2c`) and a matching warning when it fails on the Cold (focus-away) transition (`0x49aa643`). See `vision-tree-walker-worker`.
5. **Screenshots disabled reason:** a log says screenshots are off for the monitor, giving the power profile and the `managed/config` flag, and notes that the a11y walk keeps running (`0x49aa6d5..0x49aa723`). `screenshot_disabled` becomes `profile.screenshot_disabled || settings.disableScreenshots || managed policy`. See `vision-disable-screenshots`.
6. **Elements dedup log:** the DB layer logs when a frame reuses another frame's elements because their `content_hash` matches (`0xa2b7704`). Base behaviour retained.

Defaults to keep from base (target strings do not contradict them): `min_capture_interval_ms=200`, `idle_capture_interval_ms=30000`, `visual_check_interval_ms=3000`, `visual_change_threshold=0.05`, `jpeg_quality=80` (balanced), poll 250 ms, warm visual cadence 5 s, startup settle 500 ms, error backoff `min(errors/3, 30)` s after 3 consecutive errors, trigger channel 4096, stop-monitor wait 3 s, watcher poll 5 s.

---

### vision-wgc-native-capture - native Windows Graphics Capture (changed, L)

**Observed (`crates\screenpipe-screen\src\wgc_capture.rs`, `0x9f56cb7..0x9f587f1`; WinRT class names at `0x9f58a60..0x9f58b18`):**
- Classes used: `Windows.Graphics.Capture.GraphicsCaptureItem`, `Windows.Graphics.Capture.Direct3D11CaptureFramePool` (`CreateFreeThreaded`), `Windows.Graphics.Imaging.SoftwareBitmap`, `Windows.Media.Ocr.OcrEngine`.
- Session setup: `GraphicsCaptureItem` via interop `CreateForMonitor` (HMONITOR from `EnumDisplayMonitors`), `CreateCaptureSession`, `SetIsBorderRequired(false)` (non-fatal), `SetIsCursorCaptureEnabled(false)` (non-fatal), `SetMinUpdateInterval` (non-fatal, needs Win11 24H2+), `StartCapture`; handlers `FrameArrived` and item `Closed`.
- D3D11: hardware `D3D11CreateDevice`; on failure a log says it falls back to WARP; `ID3D11Multithread` protection enabled when available; `CreateDirect3D11DeviceFromDXGIDevice`; device-removed detection logs that the device was lost (GPU reset or driver update) and is being recreated, or is dropped so the next session recreates it.
- Readback: latest texture copied to a CPU staging texture (error contexts for creating and mapping the staging texture), converted to RGBA (with an error if the RGBA image cannot be built). `ContentSize` change => a log that the display size changed and the WGC frame pool is being recreated (`Recreate`).
- Lifecycle logs cover: the capture item closing (monitor unplugged or session invalidated); release of a monitor's persistent WGC session; a persistent-capture init failure with an attempt counter out of 3; persistent capture being turned off for a monitor after 3 init failures, with per-frame capture used for the rest of the session; no frame arriving before a deadline; the WGC session closing; and persistent WGC capture stopping.
- Monitor module split: `crates\screenpipe-screen\src\monitor\windows.rs` (`0x9f58ba0`), an `EnumDisplayMonitors` failure error (`0x9f57677`), and an error when the monitor requested for persistent capture cannot be found.

**Behaviour to implement:**
- Replace the xcap `VideoRecorder` wrapper with a direct `windows`-crate implementation (base already depends on `windows` 0.58/0.61): one session per monitor, free-threaded frame pool (2 buffers, B8G8R8A8), latest-frame slot updated in `FrameArrived`, and a frame sequence counter (feeds the frozen-stream check).
- Capture call returns the latest frame; first call waits up to the existing first-frame timeout. Keep base's policy (after 3 init failures, use per-frame xcap capture for the rest of the session) and `release_capture_stream()` semantics.
- Recreate the frame pool when `ContentSize` changes; recreate the whole D3D device on `DXGI_ERROR_DEVICE_REMOVED`/`RESET`.
- No yellow border, no cursor in captures, minimum update interval set to the capture cadence where supported (silently ignore on older builds).
- Keep `SafeMonitor` API (`id()`, `stable_id()` = `"{name}_{w}x{h}_{x},{y}"`, `is_primary()`, `x()`, `y()`); enumerate via `EnumDisplayMonitors` + `GetMonitorInfoW` (device name) so ids stay compatible with xcap's (xcap also derives ids from HMONITOR). Verify id compatibility on a real machine before switching (open question).

**Base files:** `crates/screenpipe-screen/src/wgc_capture.rs` (rewrite), `crates/screenpipe-screen/src/monitor.rs` (split into `monitor/mod.rs` + `monitor/windows.rs`), `crates/screenpipe-screen/Cargo.toml` (enable `Graphics_Capture`, `Graphics_DirectX_Direct3D11`, `Win32_Graphics_Direct3D11`, `Win32_Graphics_Dxgi`, `Win32_System_WinRT_Graphics_Capture` features).

---

### vision-corrupt-frame-guard - black/green frame rejection (new, S)

**Observed:** metric fields `frames_corrupt_black`, `frames_corrupt_green` in `MetricsSnapshot` (`0x522e3cc`) and health `VisionPipelineInfo` (`0x61856b0`, `0x52b87f8`).

**Behaviour (inferred, counters observed):** before snapshot/diff, sample the image on a coarse grid (e.g. 32x32 points). If essentially all samples are pure black (R,G,B all <= 2) => corrupt-black; if essentially all are saturated green with ~0 red/blue (typical D3D/NV12 zero-buffer artefact) => corrupt-green. Corrupt frames are not stored, not fed to the frame comparer, counted in the matching metric, and trigger a persistent-stream release after N (inferred 3) consecutive corrupt frames. Do not treat a locked screen as corrupt (pause gate already handles lock).

**Base files:** `crates/screenpipe-screen/src/utils.rs` or new `frame_validity.rs`; `metrics.rs`; call site in `event_driven_capture.rs` `do_capture`.

---

### vision-pipeline-metrics - metrics and health reasons (changed, M)

**Target `MetricsSnapshot` field list (`0x522e3cc`, order as packed):** `uptime_secs, frames_captured, frames_skipped, ocr_completed, ocr_cache_hits, ocr_cache_misses, ocr_empty, avg_ocr_latency_ms, ocr_gate_skips, ocr_gate_crop_ocr, ocr_gate_detects, avg_ocr_gate_detect_latency_ms, frames_video_written, frames_db_written, frames_dropped, frames_dropped_timeout, frames_dropped_error, silent_loss, silent_loss_rate, avg_db_latency_ms, frame_drop_rate, capture_fps_actual, time_to_first_frame_ms, ocr_queue_depth, video_queue_depth, pipeline_stall_count, last_db_write_ts, last_capture_attempt_ts, last_capture_loop_heartbeat_ts, capture_loop_heartbeats, capture_attempts, dedup_skips, frames_corrupt_black, frames_corrupt_green`.
New vs base: `ocr_empty, ocr_gate_skips, ocr_gate_crop_ocr, ocr_gate_detects, avg_ocr_gate_detect_latency_ms, frames_dropped_timeout, frames_dropped_error, silent_loss, silent_loss_rate, last_capture_loop_heartbeat_ts, capture_loop_heartbeats, frames_corrupt_black, frames_corrupt_green`.

**Health (`crates\screenpipe-engine\src\routes\health.rs`):**
- `vision_reason` enum values (`0x5256419`): `ok`, `disabled_by_setting`, `no_displays_expected`, `screenshots_disabled_by_config`, `screenshots_disabled_by_power_profile`, `permission_denied`, `capture_stalled`, `not_started`, `ocr_unavailable`.
- New top-level fields (`0x5256cd6`, `0x6a22da8`): `vision_reason`, `loop_stage`, `loop_stage_age_secs`, `recording_coverage` (object; `CoverageSnapshot` fields `recording_secs`, `active_secs`, `active_stalled_secs`, `active_paused_secs`, `idle_secs`, `coverage_ratio` - `0x522b693`, `0x6a0bff9`), plus storage/write-queue fields owned by other dimensions (`write_queue_degraded`, `persistent_failure_signals`, ...).
- Monitor info entries unchanged: `id, stable_id, name, width, height, is_default`.
- Diagnostic stall classification messages (`0x5256730..0x52568e5`) distinguish four cases: attempts continue but no frame gets to the writer; the loop has stopped attempting; the backend is paused or rejecting frames; frames get to the writer but are never committed. A summary log gives seconds since the last unique vision frame, the capture heartbeat age, lifetime counters (`attempts`, `persisted`, `dedup`, `silent_loss`), pool `read`/`write` figures and a suspected cause. Thresholds kept from base: drop rate > 50 %, avg DB latency > 10 s.

**Semantics (inferred where not stated):**
- `silent_loss = capture_attempts - (frames_db_written + dedup_skips + frames_dropped_timeout + frames_dropped_error + frames_skipped)` clamped at 0; `silent_loss_rate = silent_loss / max(capture_attempts,1)`.
- `ocr_empty`: OCR ran and returned no text. `ocr_gate_*`: see OCR gate item.
- `vision_reason` precedence: disabled_by_setting (disableVision) > no_displays_expected (0 monitors, e.g. clamshell) > permission_denied > screenshots_disabled_by_config > screenshots_disabled_by_power_profile > not_started > capture_stalled > ocr_unavailable > ok.

**Base files:** `crates/screenpipe-screen/src/metrics.rs`, `routes/health.rs`, `/vision/metrics` handler in `server.rs`. Recording coverage computation itself (`recording_coverage.rs`) belongs to the health dimension; vision must expose per-second state (active / stalled / paused) to it.

---

### vision-ocr-gate - text-region gate and crop OCR (new, L)

**Observed:** `crates\screenpipe-screen\src\text_regions.rs` (`0x9f5c808`, `0x9f5da2c`, type `TextRegion` with `x`,`y` at `0x527a668`). `paired_capture.rs` logs (`0x5259a09..0x5259ab6`): one for a gate decision to skip because detection found no text regions (with detect time and `app`, `window_crop`, `detect_dims` fields), and one giving the region count and union size (with `app`, `window_crop`, ...). Metrics `ocr_gate_skips`, `ocr_gate_crop_ocr`, `ocr_gate_detects`, `avg_ocr_gate_detect_latency_ms` (`0x522e3cc`, `0x49ac1d0`).

**Behaviour (algorithm inferred; interface observed):**
1. Only applies when OCR would run (no a11y text, thin a11y => hybrid, or terminal apps `wezterm alacritty kitty hyper warp` - list unchanged, `0x5259b0b`).
2. Optionally crop to the focused window rectangle (`window_crop`) when known from the a11y walk/focus tracker; else full monitor.
3. Run a cheap text-region detector on a downscaled grayscale copy (`detect_dims` = detector input size, e.g. long side 640-960 px): edge/gradient density + morphological closing + connected components; keep boxes with text-like aspect and density. Record detect latency.
4. No regions => skip OCR (`ocr_gate_skips`++), store frame without OCR text.
5. Regions found => OCR only the union bounding box of regions (padded) in full resolution (`ocr_gate_crop_ocr`++), then translate OCR word boxes back to full-frame normalised coordinates so `text_json` and OCR `elements` stay aligned with the screenshot.
6. Fail open: detector error => OCR the full image.

**Base files:** new `crates/screenpipe-screen/src/text_regions.rs` (pure Rust on `image`), `crates/screenpipe-capture/src/paired_capture.rs` (gate before `perform_ocr_*`), `metrics.rs`.

---

### vision-ocr-unify-frames - drop `ocr_text`, OCR onto frames (changed, M)

**Migration description "unify ocr text into frames" (`0xa2a3c32..0xa2a4d8d`), statements verbatim:**
```sql
ALTER TABLE frames ADD COLUMN text_json TEXT DEFAULT NULL;
UPDATE frames SET app_name = (
    SELECT ot.app_name FROM ocr_text ot
    WHERE ot.frame_id = frames.id AND ot.app_name IS NOT NULL AND ot.app_name != '' LIMIT 1
WHERE (app_name IS NULL OR app_name = '')
  AND EXISTS (SELECT 1 FROM ocr_text ot
              WHERE ot.frame_id = frames.id AND ot.app_name IS NOT NULL AND ot.app_name != '');
UPDATE frames SET window_name = (
    SELECT ot.window_name FROM ocr_text ot
    WHERE ot.frame_id = frames.id AND ot.window_name IS NOT NULL AND ot.window_name != '' LIMIT 1
WHERE window_name IS NULL
  AND EXISTS (SELECT 1 FROM ocr_text ot
              WHERE ot.frame_id = frames.id AND ot.window_name IS NOT NULL AND ot.window_name != '');
UPDATE frames SET focused = (
    SELECT ot.focused FROM ocr_text ot
    WHERE ot.frame_id = frames.id AND ot.focused IS NOT NULL LIMIT 1
WHERE focused IS NULL
  AND EXISTS (SELECT 1 FROM ocr_text ot
              WHERE ot.frame_id = frames.id AND ot.focused IS NOT NULL);
UPDATE frames SET full_text = (
    SELECT ot.text FROM ocr_text ot
    WHERE ot.frame_id = frames.id AND ot.text IS NOT NULL AND ot.text != '' LIMIT 1
WHERE (full_text IS NULL OR full_text = '')
  AND EXISTS (SELECT 1 FROM ocr_text ot
              WHERE ot.frame_id = frames.id AND ot.text IS NOT NULL AND ot.text != '');
UPDATE frames SET text_json = (
    SELECT ot.text_json FROM ocr_text ot
    WHERE ot.frame_id = frames.id AND ot.text_json IS NOT NULL AND ot.text_json != '' LIMIT 1
WHERE text_json IS NULL
  AND EXISTS (SELECT 1 FROM ocr_text ot
              WHERE ot.frame_id = frames.id AND ot.text_json IS NOT NULL AND ot.text_json != '');
DROP TRIGGER IF EXISTS ocr_text_ai;
DROP TRIGGER IF EXISTS ocr_text_au;
DROP TRIGGER IF EXISTS ocr_text_ad;
DROP TRIGGER IF EXISTS ocr_text_update;
DROP TRIGGER IF EXISTS ocr_text_delete;
DROP TABLE IF EXISTS ocr_text_fts;
DROP TABLE IF EXISTS ocr_text;
```
(Each `UPDATE ... = (SELECT ... LIMIT 1` is missing the closing `)` only because the strings tool split lines; close each subquery with `)` before `WHERE`.) The migration must sort after the `full_text_redacted_at` migration (version `20260613000001`) because it relies on that column existing and deliberately does not copy `ocr_text.redacted_at`. Followed in target by "frames app name timestamp index": `CREATE INDEX IF NOT EXISTS idx_frames_app_name_timestamp ON frames(app_name, timestamp);` (observed, `0xa2a539c..0xa2a53d6`).

**Insert path (`0xa2b6378..0xa2b6714`):** single frame insert with columns
`video_chunk_id, offset_index, timestamp, name, browser_url, app_name, window_name, focused, device_name, snapshot_path, capture_trigger, accessibility_text, text_source, accessibility_tree_json, content_hash, simhash, full_text, elements_ref_frame_id, document_path, text_json` (video_chunk_id NULL, offset 0 for snapshot frames). OCR text goes to `full_text` (when OCR is the text source or appended in hybrid) and OCR word boxes JSON to `text_json`. No `ocr_text` insert. `ocr_engine` no longer stored per frame.
**Elements:** accessibility rows inserted in bulk with pre-reserved ids from `sqlite_sequence` ("AX id reservation", `0xa2b86b8`): `INSERT INTO elements (id, frame_id, source, role, text, parent_id, depth, left_bound, top_bound, width_bound, height_bound, confidence, sort_order, properties, on_screen) VALUES (?,?,'accessibility',...)`; OCR rows `source='ocr'` with roles `page|block|paragraph|line|word` (`0xa2b8512`) or flat `block` (`0xa2b8368`).
**Readers:** `/frames/:id/text` and `/frames/:id/ocr` read `frames.text_json` (fallback to accessibility bounds unchanged); search/timeline/sync/archive must stop joining `ocr_text`.

**Base files:** new migration file, `crates/screenpipe-db/src/db.rs` (`insert_snapshot_frame_with_ocr`, all `ocr_text` reads), `paired_capture.rs`, `routes/frames.rs`, redact worker tables (`ocr_text` target removed). Coordinate with the db/search dimension.

---

### vision-windows-ocr-worker - WinRT OCR worker + OCR availability (changed, M)

**Observed:** worker errors for: the worker exiting before it took the request, a full queue, a poisoned lock, and failure to start the worker; thread name `screenpipe-windows-ocr` (`0x5a43240..0x5a432bf`); errors for a stopped worker and a closed OCR semaphore (`0x5259778`); image validation errors rejecting a null image buffer, an image (or its height or width) above the size limit, a BGRA buffer whose byte length disagrees with the dimensions, and dimension arithmetic overflow (`0x9f5ba7d`); language selection messages (user-profile engine, fallback to installed recognizer languages, install-language-pack guidance) `0x9f5b873..0x9f5be68` (base already implements language selection). The on-demand OCR route has its own failure error (`0x51523cd`) and a start log carrying `frame` and `inflight` fields (`0x6a1cad1`).
Tesseract (Linux, optional elsewhere): binary discovery `tesseract.exe`/`tessdata`/`eng.traineddata` next to the engine or system install, `TESSDATA_PREFIX`, `APPDIR`; when no binary is found (neither installed system-wide nor bundled beside the engine) a log says OCR is off for the rest of the session and suggests installing tesseract-ocr (`0x9f5cdc0`); invokes CLI with `tessedit_create_tsv` (`0x9f5adc0`).
App: a log that screen text capture is unavailable and the OCR capability notification is being shown (`0x4995388`), and a notification whose body explains screenshots are recorded without searchable text plus the install hint (`0x6185f70`).

**Behaviour:**
- A single long-lived OS thread (`screenpipe-windows-ocr`, MTA, owns the `OcrEngine` cache per language set) receives requests over a bounded channel (capacity inferred 4); `try_send` failure => "queue full" error => count as OCR skipped, not as capture failure. Replies via oneshot. Live capture and on-demand `POST /frames/:id/text` share it; on-demand concurrency is tracked (`inflight`).
- Validate dimensions before creating `SoftwareBitmap`: reject 0 sizes, overflow, and sizes above `OcrEngine::MaxImageDimension` (downscale to fit instead of failing - inferred), check BGRA length = w*h*4.
- OCR availability is probed once at startup: no recognizer languages / no tesseract => `vision_reason=ocr_unavailable` in health (only when otherwise ok) and metrics keep counting; app shows a one-time notification (app dimension).

**Base files:** `crates/screenpipe-screen/src/microsoft.rs`, `tesseract.rs`, `paired_capture.rs`, `routes/frames.rs` (`run_frame_ocr`), `routes/health.rs`.

---

### vision-tree-walker-worker - dedicated accessibility worker (new, M)

**Observed:** `crates\screenpipe-capture\src\tree_walker_worker.rs` (`0x49adcd8`, `0x5259b48`): per-worker messages for a failed walk that triggers a restart and one retry, a lost suspend acknowledgement, a worker refusing a suspend, a lost walk response, and a walk timeout with a `restarted` flag (`0x5259bc9..0x5259d1d`). Capture loop: warnings when suspending the accessibility worker fails on focus-away or on pause (`0x49aa643`, `0x49aad2c`).

**Behaviour:**
- One worker thread per monitor capture loop owns the platform `TreeWalker` (COM/UIA initialised once on that thread - base lazy-inits COM per walker thread already).
- API: `walk(config) -> Result<TreeWalkResult>` with a deadline (inferred = walker budget + 1 s grace, max 5 s); `suspend()` / `resume()` (drop retained subscriptions and caches while monitor is Cold or paused; acknowledge via response channel).
- On panic/error: recreate the worker thread and retry the walk once. On timeout: abandon the stuck thread (detach), start a fresh worker, report `restarted=true`, return timeout so the frame is captured pixel-only.
- Replaces `spawn_blocking(walk_accessibility_tree)` in `do_capture`.

**Base files:** new `crates/screenpipe-capture/src/tree_walker_worker.rs`, `paired_capture.rs::walk_accessibility_tree` callers, `event_driven_capture.rs` (suspend on Cold/pause, resume on Active).

---

### vision-uia-retained-capture - event-driven retained UIA tree on Windows (new, XL)

**Observed (`crates\screenpipe-a11y\src\platform\windows_uia.rs`, `platform\retained_tree.rs`, `tree\windows.rs`, `capture_diagnostics.rs`):**
- Subscription/strategy tokens (`0xa03d9f0`): `property_subscription`, `periodic_resync`, `partial`, `preserved`, `structure_subscription`, `text_subscription`, `property_events_and_periodic_resync`, `layout_subscription`.
- State/outcome tokens (`0xa03dc90`): `document`, `fresh_document_identity_mismatch`, `bounded_resync`, `invalidated_recovery_pending`, `preserved_when_privacy_allows`, `fresh_document_refresh`, `event_overflow`, `partial_recovery_pending`, a message for a UIA event whose identity differs from the retained tree, `unknown_event_identity`.
- Capture modes (`0xa0417fc`): `retained_advance`, `restart_retained_baseline`, `unavailable`, `preserved_when_privacy_allows`, `subscribe_or_root`, `pixel_only`.
- Slice metrics (`0xa040f70`): `provider_calls`, `retained_nodes`, `event_notices`, `pending`, `event_overflow`, `slice_ms`; logs for retained-capture initialisation and for each UIA retained-capture slice (`0xa041688`, `0xa0416d8`).
- Tracing target `screenpipe_a11y_capture` with fields `stage`, `count`, `fallback` (`0xa03d9f0`); diagnostics fields `view`, `request_scope`, `stage`, `elapsed_ms`, `budget_ms`, `nodes`, `max_nodes`, `reason`, `hresult`, `accessibility_outcome`, `pixel_outcome`, outcome values `element`, `root`, `traversal`, `partial`, `unavailable`, `fallback`; two user-facing log lines say UIA capture is degraded (or the retained capture hit a problem) while permitted screenshot recording keeps working (`0xa03b8c8..0xa03be78`).
- Walk result variants `None`, `Timeout`, `MaxNodes`, `Pending` (`0xa03f318`).
- App/role lists: Firefox/Zen document handling and control types `Document ProgressBar Text Edit Button MenuItem TabItem Hyperlink ComboBox CheckBox RadioButton ListItem TreeItem DataItem Header HeaderItem StatusBar TitleBar` (`0xa041c0d`); app list `edge chrome chromium brave slack discord claude obsidian code cursor windsurf notion` (`0xa041db8`, likely apps where retained/subscription capture is used because full traversals are expensive - inferred).
- Fragile providers: a log that tree capture is skipped for a named UIA provider known to crash (`0xa03efc1`), and a matching skip reason for crash-prone UIA providers (`0xa0418d8`).
- UIA init: 4 attempts then 30 s retry backoff; focus-change subscription with polling fallback (`0xa03e820..0xa03e98d`) - base has similar.

**Behaviour to implement (design paraphrase):**
1. For the focused top-level window, build a baseline tree once (bounded by `max_nodes`/time budget = base `TreeWalkerConfig` limits).
2. Register UIA event handlers on that window: structure-changed (subtree add/remove), property-changed (Name/Value/IsOffscreen/BoundingRectangle), text-changed, layout-invalidated. Each event becomes an "event notice" keyed by runtime id.
3. On each capture: if the window identity (hwnd + process + document runtime id) matches the retained baseline, apply pending notices by re-reading only affected subtrees within a time slice (`slice_ms`) => `retained_advance`. If notices overflowed a bounded queue (`event_overflow`) or identity mismatched => bounded resync / `restart_retained_baseline`. Apps without reliable events use `property_events_and_periodic_resync` (full resync every N captures, inferred N=10 or 30 s).
4. Privacy: if the window becomes ignored/incognito/URL-blocked, drop retained state (`preserved_when_privacy_allows` = state kept only while privacy allows).
5. When UIA cannot deliver (timeouts, HRESULT failures) produce a diagnostic and capture `pixel_only` (screenshot + OCR path) rather than failing the frame.
6. Emit the same `TreeSnapshot` (text, nodes, hashes) as base so downstream elements/dedup are unchanged.

**Base files:** `crates/screenpipe-a11y/src/tree/windows.rs`, `platform/windows_uia.rs`, new `platform/retained_tree.rs`, new `capture_diagnostics.rs`, `tree/mod.rs` (walk result variants), `budget.rs`. Feature-flag it (e.g. `TreeWalkerConfig.retained_capture: bool`, default on for listed heavy apps) so the classic walker remains the fallback.

---

### vision-windows-window-resolution - window identity and skip reasons (changed, S)

**Observed (`crates\screenpipe-a11y\src\platform\windows.rs`, `tree\windows.rs`, `events.rs`):**
- Skip reasons (`0xa03abd0`), six display texts: incognito or private browsing; excluded app; window ignored by user configuration; window absent from the included-windows list; blocked by the browser URL policy; browser URL privacy validation still pending.
- Tree logs: a skip log for a browser extension popup that matched an ignored window, with an `app` field (`0xa0417b1`); a11y skip logs naming the cause: a user ignore pattern, an incognito/private window, a fragile UIA provider, a hard-coded excluded app, or a transient shell window class (`0xa041879..0xa041922`).
- Hard-coded sensitive/system apps (`0xa041530`): `1password bitwarden lastpass dashlane keepassxc credential manager logonui lockapp searchhost shellexperiencehost pickerhost snippingtool` (base has a subset in `tree/windows.rs`).
- Transient shell windows (`0xa037990..0xa037a10`): `runtimebroker.exe`, `startmenuexperiencehost.exe`, `shellexperiencehost.exe`, `applicationframehost.exe`, `msedgewebview2.exe`, window classes `CiceroUIWndFrame`, `tooltips_class32`, `Windows.UI.Core.CoreWindow`, `TopLevelWindowForOverflowXamlIsland`, `Chrome_WidgetWin_1` (context).
- a11y logs for resolving the UWP app hosted inside the frame host, for normalising an app name, and for a foreground change that was captured, excluded, or skipped as a transient shell window, with fields `pid raw_process window_class effective hosted` (`0xa038d10..0xa0392cb`).

**Behaviour:** when the foreground window belongs to `ApplicationFrameHost.exe`, resolve the hosted UWP child's process for `app_name`; normalise exe names to display names; ignore transient shell/tooltip/IME windows for focus/app-switch triggers; carry a typed `SkipReason` through `TreeWalkResult::Skipped` so health/diagnostics can show it. The capture still writes a pixel frame unless the reason is a privacy reason (ignored/incognito/URL), in which case no frame is written (base behaviour).

**Base files:** `crates/screenpipe-a11y/src/platform/windows.rs`, `tree/windows.rs`, `events.rs`, `incognito/windows.rs`.

---

### vision-url-domain-rules - structured URL block/allow rules (changed, M)

**Observed:**
- Settings keys `ignoredUrls`, `includedUrls` (`0x5a42a38`); types `DomainRule{domain, includeSubdomains, excludedSubdomains}` (`0x5a430d0`; serde descriptor for a three-element struct at `0x52b8d38`) and untagged enum `UrlRule` (string | DomainRule) (`0x5a43138`).
- Validation (`crates\screenpipe-a11y\src\url_filter.rs`, `0xa039c71..0xa039fc1`): `excludedSubdomains` is rejected unless `includeSubdomains` is true; each excluded subdomain has to sit below the domain; an empty domain is rejected; a URL-form entry needs an http or https scheme; the domain needs at least one dot; malformed domains are rejected; so are URLs without a hostname.
- Browser list for URL policy (`0xa039d50`): `chrome chromium firefox safari edge brave arc vivaldi opera zen comet epiphany helium`.
- Frontend schema (`frontend/_next/static/chunks/7451-47878874a9162526.js`): `ignoredUrls: array(union(string, DomainRule))`, `includedUrls: array(DomainRule).optional()`, DomainRule `domain` min length 3, strict object.
- Settings page (`ev_46bc6f41...`): the allow list, when it has entries, limits recording to those websites; browser capture then only covers hosts that match, native apps are governed by the app filters instead, and a tab whose URL the engine cannot confirm is skipped. The block list wins over the allow list. Legacy string entries carry a label marking them as old-style broad matches. New rules default `includeSubdomains:false`; excluded subdomains offered from observed domains. Privacy category switches add ordinary rules and track ownership in `categoryOwnedFilters: {apps:[], domains:[]}` (app patterns use `App::` form, e.g. `1Password::`, domains like `bitwarden.com`).

**Matching semantics:**
- Legacy string rule: base `is_url_blocked` semantics (exact host, any subdomain, or no-TLD label match).
- DomainRule: host == domain matches; if `includeSubdomains`, any `*.domain` matches unless host equals or is below one of `excludedSubdomains`. Case-insensitive, trailing dot stripped.
- Decision for a browser window: blocked if any ignored rule matches. Else if `includedUrls` non-empty: record only if URL known AND matches an included rule; if the browser URL is not yet known/verified => use the URL-validation-pending skip reason => no frame (pixels not stored) for that capture. Non-browser apps unaffected by `includedUrls`.
- Applies to both screenshot capture (`WindowFilters`) and a11y walk (base already shares `url_filter`).

**Data compatibility:** store.bin may contain both string and object entries; serde must accept both (`#[serde(untagged)] enum UrlRule { Legacy(String), Domain(DomainRule) }`), serialise back unchanged.

**Base files:** `crates/screenpipe-config/src/recording.rs` (`ignored_urls: Vec<UrlRule>`, new `included_urls: Vec<DomainRule>`), `crates/screenpipe-a11y/src/url_filter.rs`, `crates/screenpipe-screen/src/capture_screenshot_by_window.rs` (`WindowFilters`), `tree/*` walkers, `vision_manager/manager.rs` config plumbing, app `lib/utils/validation.ts` + settings privacy UI (settings dimension).

---

### vision-disable-screenshots - accessibility-only recording toggle (new, S)

**Observed:** settings key `disableScreenshots` (`0x5a42a38`; default `false` in frontend defaults chunk `378-9f286ebe7c0f0713.js`); health reasons `screenshots_disabled_by_config`, `screenshots_disabled_by_power_profile` (`0x5256419`); loop log with `managed/config=` (`0x49aa6d5`); paired capture logs a skipped screenshot because `screenshot_disabled` is set, with the trigger (`0x525998d`, base same); frontend treats `disableVision || disableScreenshots` as screen recording being switched off for the last-5-minutes UI, with an enterprise-managed variant (`7737-fae548fffc70b86b.js`).

**Behaviour:** when true, capture loops keep running triggers and a11y walks and write frame rows (timestamp, app, window, url, full_text, tree) with empty `snapshot_path`; no JPEG, no OCR (no pixels), no visual-change probe (cannot diff without screenshots: disable visual checks), no compaction input. Power profile may also force it (base). Health reports `vision_reason=screenshots_disabled_by_config` (or `_by_power_profile`).

**Base files:** `recording.rs` (field), `vision_manager/manager.rs` (`VisionManagerConfig.disable_screenshots`), `event_driven_capture.rs`, `routes/health.rs`.

---

### vision-capture-max-width - capture-time resolution cap (new, S)

**Observed:** settings key `captureMaxWidth` adjacent to `maxSnapshotWidth` (`0x5a42a38`); not referenced by the frontend bundle (no UI). Base derives the macOS SCK stream width from `videoQuality` (`set_sck_capture_max_width`).

**Behaviour (inferred):** optional u32; 0/absent = derive from `videoQuality` (`video_quality_to_max_snapshot_width`: low 1280, balanced 1920, high 3840, max native). Applied at capture time (macOS SCK stream size; Windows: downscale right after readback before diff/OCR/encode) so all downstream work uses the smaller image. `maxSnapshotWidth` still caps the stored JPEG. OCR/thumbnail coordinates remain normalised so nothing else changes.

**Base files:** `recording.rs`, `vision_manager/manager.rs`, `crates/screenpipe-screen/src/monitor.rs` (Windows downscale hook), `utils.rs`.

---

### vision-recording-detail - scroll-capture cadence (new, S)

**Observed:** settings key `recordingDetail`, enum `RecordingDetail` = `auto|low_impact|balanced|more_detail` (`0x5a429c8`), default `auto` (`378-...js`). Settings page "Scroll capture" card (`ev_46bc6f41...`), described: `auto` varies the scroll snapshot interval within 1-5 s according to how quickly captures complete and the power profile; `low_impact` is roughly every 5 s; `balanced` roughly every 2 s; `more_detail` roughly every 1 s; the setting changes only how often scroll snapshots are taken, not the quality or extraction of each one; battery-related limits continue to apply. Hidden when `disableVision`.

**Behaviour:** while the user is actively scrolling (scroll events from the UI recorder; base `captureScroll` gate controls whether scroll rows are persisted, not whether scroll snapshots happen - keep that), the capture loop takes intermediate "scroll" snapshots no more often than the cadence (in addition to the existing `scroll_stop` trigger). Cadence: `low_impact`=5000 ms, `balanced`=2000 ms, `more_detail`=1000 ms, `auto` = clamp(observed average capture duration x 4, 1000, 5000) and raised to >= 5000 ms on battery-saver power profiles (inferred). Use trigger name `scroll_stop` for the final snapshot and keep existing trigger vocabulary (no new trigger string observed - intermediate snapshots may reuse `scroll_stop` or `visual_change`; open question).

**Base files:** `recording.rs`, `event_driven_capture.rs` (scroll-burst timer), `ui_recorder.rs` (forward scroll-activity signal even when rows are not persisted), `power/profile.rs`.

---

### vision-thumbnail-endpoint - `GET /frames/:frame_id/thumbnail` (new, M)

**Observed:** route `/frames/:frame_id/thumbnail` (`0x5257ca8`), handler `screenpipe_engine::routes::frames::get_frame_thumbnail` (`0x6a1d878`), query type `FrameThumbnailQuery` (`0x6a0bf30`). Validation errors reject `width` and `quality` outside their allowed ranges (`0x5152741`, `0x5152763`). Errors: a remote-device message with `error_type: remote_device` (`0x51527b8`), a message that the exact frame's thumbnail cannot be produced, a message that neither the frame nor any nearby frame can supply a thumbnail, `frame_unavailable`, `snapshot_missing` (`0x5152810`). Fallback logs cover: the exact source failing with fallback turned off; the source failing and nearby frames being tried; a thumbnail served from a nearby frame instead of the requested one; a source path that is not a regular file; and a legacy video source that is missing or damaged. Cache values `hit`/`miss` (`0x52562b0..0x5256401`). Response headers (`0x6a1cfa0`): `content-type: image/jpeg`, `cache-control: private, no-cache`, `etag` (value prefixed `"frame-thumbnail-`, `0x6a1c9d1`), `x-screenpipe-thumbnail-cache`, `x-screenpipe-thumbnail-width`, `x-screenpipe-thumbnail-height`, `content-length`.
Frontend callers (`3308-e676c712bfd27088.js`, `ev_5e296525...`): srcset widths 384 and 768 at quality 75 with optional `retry=<n>` cache-buster and `fallback=false`; activity preview 320 px quality 60 `fallback=false`; workflows 1280 px quality 80 `fallback=false` (`0x49f2994`).

**Query:** `width` (u32, required or default 320; valid range not observed - use 32..=3840, inferred), `quality` (u8, default 75; range 1..=100 inferred), `fallback` (bool, default true), `retry` (ignored by server).

**Behaviour:**
1. Look up frame (same lookup as `get_frame_data`). Remote-device frames (frame `machine_id` not local / cloud path) => 404/409 JSON `{"error": <message saying the frame lives on another device>, "error_type": "remote_device"}`.
2. Source: snapshot JPEG if `snapshot_path` exists and is a regular file; else compacted MP4 at `video_chunk_id/offset_index` via ffmpeg extraction (base extraction helpers); legacy video likewise.
3. Resize keeping aspect ratio to `width` (never upscale), encode JPEG at `quality`.
4. Cache encoded thumbnails in memory (LRU keyed by frame id + width + quality + source generation; moka is already a dependency); `x-screenpipe-thumbnail-cache: hit|miss`; ETag `"frame-thumbnail-<frame_id>-<w>-<q>-<generation>"`; honour `If-None-Match` with 304 (inferred).
5. On failure: if `fallback=false` => 404 `{"error": <message saying the exact frame's thumbnail is unavailable>, "error_type": "frame_unavailable"|"snapshot_missing"}`; else search nearby frames (same device, backward then forward, base `try_nearest_frame` logic) and serve that thumbnail (header width/height reflect the served image); none => 404 with a message saying no thumbnail is available from the frame or its neighbours.
6. Requires the same Bearer auth as other frame routes (`0x5115d83`).

**Base files:** `crates/screenpipe-engine/src/routes/frames.rs` (new handler + query/response types with `OaSchema`), `server.rs` route registration, reuse `video_utils.rs`/`video_cache.rs`.

---

### vision-preview-samples - `GET /frames/preview-samples` (new, M)

**Observed:** route (`0x5257ca8`), handler `get_frame_preview_samples` (`0x6a1d748`); types `FramePreviewSamplesQuery`, `FramePreviewSamplesResponse`, `FramePreviewSample`, `FramePreviewSource`, `TimeRange` (`0x6a0bf30..0x6a0c4b0`); doc (paraphrased): hands back a capped list of frames to drive a brief preview in the UI; snapshot entries are served through the thumbnail route, while compacted entries name the MP4 that already exists and an offset into it, leaving decoding to the browser; the endpoint does no frame extraction and creates no media (`0x6a1df5b..0x6a1e06d`). Validation (`0x6a1e951`, `0x6a1e96f`): `limit` in 1..=N; `app_name` 1 to 256 characters; range no longer than 24 hours; `start_time` earlier than `end_time`; `browser_domain` a valid host. A separate error covers a failed sample query (`0x5152b40`).
Frontend (`home/page-...js`, `ev_df2f0e57...`): query `start_time`, `end_time`, `app_name`, `limit=6`, optional `browser_domain`; response `{frames:[...]}`; client keeps entries with safe-integer `frame_id > 0`, parseable `timestamp`, and when `source == "video"` a safe-integer `video_chunk_id` and finite `video_offset_seconds >= 0`; at most 6. Workflows app sends `limit=8` (`0x5223330`). Dev mock returns `{frames:[{frame_id, timestamp}]}` x6.

**Response shape:**
```
{"frames": [
  {"frame_id": i64, "timestamp": RFC3339,
   "source": "snapshot" | "video",
   "video_chunk_id": i64?,            // present when source=video
   "video_offset_seconds": f64? }     // present when source=video
]}
```
**Behaviour:** select frames in `[start_time, end_time)` (max span 24 h) with `app_name` exact (case-insensitive, inferred) and, if given, `browser_domain` matching the frame's `browser_url` host (domain or subdomain). Spread samples evenly over the range (bucket the range into `limit` slices, pick the first frame with an available source in each). `source=snapshot` if `snapshot_path` set; else if compacted (`video_chunk_id` with mp4 file) `source=video`, `video_offset_seconds = offset_index / video_chunks.fps`. Skip frames with no media. Limit default 6, max 12 (inferred).

**Base files:** `routes/frames.rs`, `crates/screenpipe-db/src/db.rs` (query), `server.rs`.

---

### vision-preview-media - `GET /frames/preview-media/:video_chunk_id` (new, S)

**Observed:** route (`0x5257ca8`), handler `get_frame_preview_media` (`0x6a1dad8`); doc (paraphrased): streams a compacted MP4 as-is, honouring byte ranges so the preview plays through the browser's own video decoder, without extracting or re-encoding anything (`0x6a1dde0`); errors for media not found, a failed history lookup and a failed media lookup (`0x5152a20..0x5152b00`). Frontend helper returns `<api>/frames/preview-media/<id>` (`3308-...js`).

**Behaviour:** look up `video_chunks.file_path` by id; must be a local regular `.mp4` (compacted snapshot chunk; reject `cloud://` and missing => 404). Serve `video/mp4` with `Accept-Ranges: bytes`, honour single `Range` header (206 + `Content-Range`), `cache-control: private`. Use `tower-http` `ServeFile` (already a dependency family) or a small range reader. Auth as other routes.

**Base files:** `routes/frames.rs`, `server.rs`.

---

### vision-elements-formats - element API projections (changed, L)

**Observed:** `ElementsQuery` fields `q, frame_id, source, role, start_time, end_time, app_name, limit, offset, format, fields` (doc `0x5118d9a`; struct names `0x6a0bba0`); `FrameElementsQuery`; `ElementState{focused, selected, expanded}` with `ElementResponse.state` (`0x59fbc06`); formats documented (paraphrased): `json` (default), `csv`/`tsv` (column-oriented, header row emitted once), `outline` (compact text meant for cheap reading), `automation` (only interactive controls, carrying a snapshot revision, refs valid only within the response, stable keys on a best-effort basis, state, bounds and permitted actions), `preferred` (driven by the desktop AI-context setting; falls back to outline for reading) (`0x5118e2d..0x511900a`); `fields=a,b,c` dotted paths (`0x5116243`).
Outline/automation text markers (`0x6a1f4c9..0x6a1f6da`): a header line giving the frame id and the text-element count; state words `off-screen disabled selected focused collapsed expanded`; a truncation line giving shown vs total counts and suggesting `?q=`, `?role=`, `?on_screen=true` or `?limit=` to narrow; an empty-result line for frames with no text-bearing elements that points to `format=json` for the raw tree; an empty-result line for frames with no automation targets that points to `format=outline`; an automation header carrying the frame id and `revision=r_<hash>`; guidance that refs are response-local and must be re-verified (key + role + name + bounds); a count line for automation targets that advises narrowing to a single frame or app before taking any action; key confidence `derived|strong|ambiguous`; target line markers `[ref=eN]`, `key=k_<hash>`.
Role vocabulary for automation (`0x6a1eb98..0x6a1ed10`): clickable `button link checkbox check_box radio switch tab menuitem menu_item disclosure cell row listitem list_item treeitem tree_item`; typeable `textfield text_field textbox text_box textarea edit entry input password text password_text` (actions `type`, `set_value`); selectable `combobox combo_box listbox list_box popupbutton popup_button select slider spinbutton spin_button spinner` (`press_key`); scrollable `scrollarea scroll_area scrollbar scroll_bar webarea web_area` (`scroll`); properties consulted `is_enabled is_keyboard_focusable is_focused is_selected is_expanded is_password placeholder help_text role_description`.
MCP mapping (`0xa0d6447..0xa0d658d`): purpose `automation`/`computer-use` => `automation`, `read` => `outline`, else `preferred`. `preferred` maps from settings `semanticContextMode` (`memory` => outline, `computerUse` => automation, `both` => outline; inferred).

**Behaviour:**
- `format=csv|tsv`: flatten each `ElementResponse` to columns (dotted names like `bounds.left`), header once; `fields` selects/ordering columns; content-type `text/csv` / `text/tab-separated-values`.
- `format=outline`: plain text, one line per text-bearing element in `sort_order`, indented by depth, role + text + state words; capped; trailing narrowing hint.
- `format=automation`: only interactive roles (above) that are enabled; assign `ref=e1..eN` per response; `key=k_` + short hash of (app, window, role path, name, automation_id); `revision=r_` + hash of the frame's element set; include bounds (normalised) and allowed actions; mark key confidence `strong` (automation_id present), `derived` (role path + name), `ambiguous` (duplicates).
- `state` object derived from `properties` (`is_focused`, `is_selected`, `is_expanded`).
- Keep JSON default and pagination shape `{data, pagination:{limit, offset, total}}` (base).

**Base files:** `crates/screenpipe-engine/src/routes/elements.rs`, new `routes/response_format.rs` (shared csv/tsv + fields, also used by `/search` - coordinate with the API dimension), `crates/screenpipe-db` element queries.

---

### vision-compaction-encoder - hardware-aware snapshot compaction (changed, M)

**Observed (`crates\screenpipe-engine\src\compaction_encoder.rs`, `0x6a78750..0x6a78ab8`, `0x6a9dbf8`):** probe command `-hide_banner -loglevel error -f lavfi -i color=black:s=640x360:r=10:d=0.3 -frames:v 2 -vf scale=trunc(iw/2)*2:trunc(ih/2)*2 ... -pix_fmt yuv420p`; candidate order `hevc_videotoolbox`, `hevc_nvenc`, `hevc_qsv`, `hevc_amf`, fallback `libx264` (software args include `veryfast`, `-crf`, `-threads`); env `SCREENPIPE_COMPACTION_ENCODER` with values `software` (force software), `hardware`, or a specific encoder name (which is still probed before use); unrecognised value => default order; logs name the chosen snapshot compaction encoder, flag when it is a hardware one, and report a probe that could not be spawned, could not be waited on, or ran past its timeout.
Compaction worker (`0x6a79390..0x6a79b5a`, `0x6ab6e40..0x6ab6f9f`): a start log with `min_age`, plus skipping / throttled / paused / deferred states; a log for clearing N invalid snapshot pointers on a device; a log for postponing N snapshots that are unreadable for now; source JPEGs retained for a later retry when ffmpeg fails; an error when ffmpeg writes an empty MP4; ffmpeg pause handling during power/capture pause (ffmpeg had already exited, or the wait for the paused ffmpeg to exit timed out); and a log that compaction is turned off by the recording settings (`0x498c096`). Input pipeline unchanged: `-f image2pipe -vcodec mjpeg -r <fps> -i - ... -movflags frag_keyframe+empty_moov+default_base_moof`.
Partial index (migration "add frames snapshot pending index", `0xa2a756b`):
```sql
CREATE INDEX IF NOT EXISTS idx_frames_snapshot_pending
    ON frames(device_name, timestamp)
    WHERE snapshot_path IS NOT NULL;
```
DB updates (`0xa2b6968..0xa2b6bb8`): `UPDATE frames SET video_chunk_id = ?, offset_index = CASE id ... ELSE offset_index END, snapshot_path = NULL WHERE id IN (...) AND snapshot_path IS NOT NULL` and `UPDATE frames SET snapshot_path = NULL WHERE id IN (...)` for missing files.

**Behaviour:** at worker start, probe encoders once (timeout ~10 s each, inferred) in order, skipping ones not built into the bundled ffmpeg; cache the winner. Hardware HEVC encoders get quality via their CQ/QP knob mapped from `video_quality_to_crf`; `libx264` uses `-preset veryfast -crf <crf> -threads 1`. Keep base `libx265` path as an explicit `software` option? Target's software fallback is `libx264`; for Freenpipe keep libx265 if available else libx264 (open question: output codec mixture is fine for readers since ffmpeg decodes both). Missing snapshot files: clear DB pointer; unreadable-but-present (locked by AV) files: defer to next cycle. On pause (power/lock), stop feeding and let ffmpeg finish or kill after timeout; keep source JPEGs for retry.
Defaults kept from base: min age 600 s, poll 300 s, 100 frames/chunk (50 throttled), fps clamp 0.1..30, query LIMIT 5000.

**Base files:** new `crates/screenpipe-engine/src/compaction_encoder.rs`, `snapshot_compaction.rs`, new migration for the partial index.

---

### vision-hybrid-payload-storage - payload staging and Parquet sealing (new, XL; storage-dimension dependency)

**Why vision cares:** the capture writer still inserts full text/tree/OCR JSON into `frames`, but in hybrid mode each new frame also gets a `frame_payloads` staging row via triggers; later the sealer moves payload text into Parquet files and NULLs the columns. All vision readers (`/frames/:id/context`, `/text`, thumbnails do not care) must read through the payload reader. Elements likewise become a virtual table over a bulk store.

**DDL (verbatim, `crates\screenpipe-db\src\storage\schema.rs`, `0x49e7699..0x49e8220`):**
```sql
CREATE TABLE _hybrid_migrations(version INTEGER PRIMARY KEY, checksum TEXT NOT NULL);
CREATE TABLE storage_metadata (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1), descriptor TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0, maintenance INTEGER NOT NULL DEFAULT 0,
    staging_bytes INTEGER NOT NULL DEFAULT 0, staging_limit INTEGER NOT NULL,
    record_limit INTEGER NOT NULL, policy TEXT NOT NULL, required_surfaces INTEGER NOT NULL,
    writer_version TEXT NOT NULL
);
CREATE TABLE upload_bindings (
    destination TEXT PRIMARY KEY, checkpoint TEXT NOT NULL
);
CREATE TABLE payload_files (
    id TEXT PRIMARY KEY, search_path TEXT NOT NULL UNIQUE, detail_path TEXT NOT NULL UNIQUE,
    search_checksum TEXT, detail_checksum TEXT, schema_version INTEGER NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('encoding','published','dirty','retired')),
    row_count INTEGER NOT NULL, created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE frame_payloads (
    frame_id INTEGER PRIMARY KEY REFERENCES frames(id) ON DELETE CASCADE,
    generation INTEGER NOT NULL CHECK(generation > 0),
    state TEXT NOT NULL CHECK(state IN ('staged','sealed')),
    file_id TEXT REFERENCES payload_files(id), bytes INTEGER NOT NULL,
    policy TEXT NOT NULL, completed_surfaces INTEGER NOT NULL DEFAULT 0,
    capture_version TEXT, archive_writer_version TEXT,
    attempts INTEGER NOT NULL DEFAULT 0, retry_at INTEGER, last_error TEXT,
    CHECK((state='staged' AND file_id IS NULL) OR (state='sealed' AND file_id IS NOT NULL))
);
CREATE INDEX frame_payloads_file ON frame_payloads(file_id);
CREATE INDEX frame_payloads_staging ON frame_payloads(state,frame_id);
ALTER TABLE frames ADD COLUMN payload_full_text_length INTEGER NOT NULL DEFAULT 0;
ALTER TABLE frames ADD COLUMN payload_accessibility_length INTEGER NOT NULL DEFAULT 0;
ALTER TABLE frames ADD COLUMN payload_full_text_present INTEGER NOT NULL DEFAULT 0;
ALTER TABLE frames ADD COLUMN payload_accessibility_present INTEGER NOT NULL DEFAULT 0;
ALTER TABLE frames ADD COLUMN payload_detail_present INTEGER NOT NULL DEFAULT 0;
DROP TRIGGER IF EXISTS frames_ai;
DROP TRIGGER IF EXISTS frames_au;
DROP TRIGGER IF EXISTS frames_ad;
DROP TABLE frames_fts;
CREATE VIRTUAL TABLE frames_fts USING fts5(full_text,app_name,window_name,browser_url,
    content='',contentless_delete=1,tokenize='unicode61');
```
Revocation (`0x49e1fd7`): `CREATE TABLE IF NOT EXISTS _storage_revocation(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL);` + `INSERT OR IGNORE INTO _storage_revocation VALUES(1,0);` and triggers `hybrid_read_revoke_policy`, `hybrid_read_revoke_payload`, `hybrid_read_revoke_element`, `hybrid_read_revoke_delete_<table>`, `hybrid_read_revoke_redaction_<table>` bumping `revision`; per-table `hybrid_revision_<table>_INSERT|UPDATE|DELETE` triggers bumping `storage_metadata.revision`. Frame triggers named `hybrid_frame_insert`, `hybrid_frame_update`, `hybrid_frame_delete`, `hybrid_frame_sealed_guard`, `hybrid_frame_metadata` (`0x49e8cd0`).
Surface bitmask (from staging insert CASE and redaction UPDATE, `0x49e8368`, `0x5149310`): 1=`full_text`, 2=`accessibility_text`, 4=`accessibility_tree_json`, 8=`text_json`, 16=`window_name`, 32=`browser_url`. Empty surfaces count as completed at insert.
Staging insert on frame insert: `INSERT INTO frame_payloads(frame_id,generation,state,bytes,policy,completed_surfaces) SELECT NEW.id,1,'staged',<sum of byte lengths of the four payload columns>,<storage_metadata.policy>,<empty-surface mask>`.
Parquet layout: per day `%Y-%m-%d` directory with `search.parquet` and `detail.parquet` under the descriptor's `payloads` root (`0x49e9bc0`, `0xa27c3ad`); schemas (`0xa2c46c8`):
```
message screenpipe_search_v1 { REQUIRED INT64 id; REQUIRED INT64 generation; OPTIONAL BYTE_ARRAY full_text (UTF8); OPTIONAL BYTE_ARRAY accessibility_text (UTF8); }
message screenpipe_detail_v1 { REQUIRED INT64 id; REQUIRED INT64 generation; OPTIONAL BYTE_ARRAY accessibility_tree_json (UTF8); OPTIONAL BYTE_ARRAY text_json (UTF8); }
```
Sealing (`0x49e9760..0x49e9cd8`): select staged frames where `bytes <= record_limit`, policy matches and `(completed_surfaces & required_surfaces) = required_surfaces`; encode files (state `encoding`), fsync, publish with checksums, set `frame_payloads.state='sealed', file_id`, NULL the four text columns on `frames`, retire unused files. Privacy retry: `retry_at = now + MIN(3600, 30*(attempts+1))`, `last_error='configured surface processing failed'` (`0x626d0d8`). Reader: `SELECT f.id,p.generation,p.state,p.file_id,p.bytes,...,pf.search_path,pf.detail_path,pf.search_checksum,pf.detail_checksum FROM frames f JOIN frame_payloads p ON p.frame_id=f.id LEFT JOIN payload_files pf ON pf.id=p.file_id WHERE f.id IN (SELECT value FROM json_each(?))`, retry if `_storage_revocation.revision` changed mid-query (`0x626ace8`).
Descriptor `storage.json` fields (`0xa27c3ad..0xa27c658`): `format` (`sqlite` | `hybrid-parquet-v1`), `database_id`, `generation`, `source_continuity`, `index` (`index.sqlite`), `capabilities` (`parquet-frame-v1`, `contentless-delete-fts5`, `durable-wal`, `parquet-bulk-v1`), `budget` {`row_group_rows, file_rows, file_bytes, record_bytes, decode_bytes, response_bytes, concurrent_decodes, staging_bytes, disk_reserve_bytes, operation_timeout_secs, lifecycle_timeout_secs, read_attempts`}, `privacy` {`identity?`, `required_surfaces`}.
Bulk elements (`0x49e3586..0x49e50a1`): original `elements` renamed and copied into `_bulk_element_rows(id, <element columns>, _archive_generation, _archive_deleted, _archive_file)`, helper tables `_bulk_files`, `_bulk_element_kinds(source,role)`, `_bulk_element_ranges`, `_bulk_element_frames`, `_bulk_element_groups`, `_bulk_element_parent_refs`, `_bulk_element_checks`, `_bulk_element_state`, `_bulk_element_lookup`; `CREATE VIRTUAL TABLE elements USING screenpipe_elements;` (custom vtab over SQLite-staged rows + Parquet archives under `bulk-v1/elements/*.parquet`); `elements_fts` becomes `fts5(text,role,frame_id UNINDEXED,content='_bulk_element_search_content',content_rowid='id',columnsize=0,tokenize='unicode61')`. Full verbatim statements are in `tools\out\v_storage.txt` lines 15-120.

**Vision-side requirements:** (a) insert path must work in both `sqlite` and `hybrid-parquet-v1` modes (triggers do the staging; the writer must not write payload columns after sealing); (b) frame/element readers must go through the payload reader when `payload_*_present` is set and the column is NULL; (c) redaction worker must operate on staged payloads before sealing (`completed_surfaces`); (d) `required_surfaces` comes from the PII policy (`piiRedactionColumns` + async redaction toggles).
**Recommendation:** Freenpipe should first ship the `sqlite` format only (all items above work with resident columns) and treat hybrid storage as a separate, later storage-dimension project; but it must be able to *open* a hybrid database written by screenpipe if interop with existing data dirs is a goal (open question).

**Base files:** new `crates/screenpipe-db/src/storage/*` (owned by storage dimension); vision touch points `db.rs` frame readers, `routes/frames.rs`, `routes/elements.rs`, redact worker.

---

### vision-redaction-surfaces - more text surfaces under async PII redaction (changed, L)

**Migrations (descriptions + verbatim DDL; versions from comments where given):**
- "add elements redacted at" (`20260613000000`, `0xa2a322d`): `ALTER TABLE elements ADD COLUMN redacted_at INTEGER;` `CREATE INDEX IF NOT EXISTS idx_elements_redacted_at ON elements(redacted_at);`
- "add frames full text redacted at" (`20260613000001`, `0xa2a39e8`): `ALTER TABLE frames ADD COLUMN full_text_redacted_at INTEGER;` `CREATE INDEX IF NOT EXISTS idx_frames_full_text_redacted_at ON frames(full_text_redacted_at);`
- "add frames derived text redacted at" (`20260619000000`, `0xa2a5b58`): `ALTER TABLE frames ADD COLUMN accessibility_tree_redacted_at INTEGER;` `ALTER TABLE frames ADD COLUMN window_name_redacted_at INTEGER;` + indexes `idx_frames_accessibility_tree_redacted_at`, `idx_frames_window_name_redacted_at`.
- "add frames browser url redacted at" (`20260619120000`, `0xa2a614c`): `ALTER TABLE frames ADD COLUMN browser_url_redacted_at INTEGER;` `CREATE INDEX IF NOT EXISTS idx_frames_browser_url_redacted_at ON frames(browser_url_redacted_at);`
- "add frames text json redacted at" (`20260627000000`, `0xa2a6ddb`): `ALTER TABLE frames ADD COLUMN text_json_redacted_at INTEGER;` `CREATE INDEX IF NOT EXISTS idx_frames_text_json_redacted_at ON frames(text_json_redacted_at);`
- "make redacted at indexes partial" (`0xa2a82f7..0xa2a87b9`): drop and recreate the six frame indexes as `... ON frames(<col>) WHERE <col> IS NULL;` for `accessibility_redacted_at`, `full_text_redacted_at`, `accessibility_tree_redacted_at`, `window_name_redacted_at`, `browser_url_redacted_at`, `text_json_redacted_at`.
(Exact file names: `<version>_<description with underscores>.sql`; exact versions for the last two are not in strings - pick versions after `20260627000000`.)

**Worker SQL (`crates\screenpipe-redact\src\worker\tables.rs`, `0x626d360..0x626db18`):**
- Frames batch: `SELECT id, full_text, accessibility_text, accessibility_redacted_at, accessibility_tree_json, accessibility_tree_redacted_at, window_name, window_name_redacted_at, browser_url, browser_url_redacted_at, text_json, text_json_redacted_at FROM frames WHERE full_text IS NOT NULL AND full_text != '' AND full_text_redacted_at IS NULL ORDER BY id DESC LIMIT ?`.
- Elements batch: `SELECT id, text, properties FROM elements WHERE redacted_at IS NULL AND ( (text IS NOT NULL AND text != '') OR (properties IS NOT NULL AND ( properties LIKE '%"value"%' OR properties LIKE '%"placeholder"%' OR properties LIKE '%"help_text"%' OR properties LIKE '%"role_description"%' OR properties LIKE '%"url"%' )) ) ORDER BY id DESC LIMIT ?`; update `UPDATE elements SET text = ?, properties = ?, redacted_at = strftime('%s', 'now') WHERE id = ?`.
- Derived copies: `UPDATE frames SET accessibility_tree_json = ?, accessibility_tree_redacted_at = strftime('%s', 'now') WHERE id = ?`, same pattern for `text_json`, `window_name`, `browser_url`.
- ui_events multi-column (`text_content`, `element_value`, `window_title`, `element_name`, `element_description`, `element_ancestors`), one `redacted_at`.
- `ocr_text` surface removed (table dropped). Audio unchanged (`DELETE FROM audio_transcriptions WHERE id = ?` path exists for empty results).

**Column selection setting `piiRedactionColumns`** (`0x5a42a38`; option ids `0x626db18`): `accessibility_text, accessibility_tree, audio_transcription, ui_text_content, ui_element_value, ui_window_title, ui_element_name, ui_element_description, ui_element_ancestors, element_text, element_properties, a11y_url_field` (+ `window_name`, `browser_url` from frontend). Frontend (`ev_46bc6f41...`): always-on set `accessibility_text, accessibility_tree, window_name, audio_transcription, ui_text_content, ui_element_value, ui_window_title, element_text`; recommended-on optional `element_properties`; optional off `browser_url`, `ui_element_name`, `ui_element_description`, `a11y_url_field`. Default when unset = always-on + `element_properties`. `full_text` and `text_json` are always processed (search source).

**Propagation (redaction map, `crates\screenpipe-redact\src\redaction_map.rs`, `0x627c280`):** detect once on `full_text` (union of accessibility + OCR text), build a map of detected spans -> replacement, then replace occurrences in `window_name`, `browser_url`, `accessibility_tree_json` string values and `text_json` word texts (leaving box geometry) using Aho-Corasick multi-pattern replacement - no extra model pass. Malformed JSON => leave un-stamped and log that the malformed `accessibility_tree_json` / `text_json` was skipped without a redaction stamp.
FTS: `frames_au` trigger re-syncs `frames_fts` after updates (base trigger fires on `full_text, app_name, window_name, browser_url`); in contentless-FTS hybrid mode the worker deletes + reinserts the FTS row (`0x5149310`).

**Base files:** migrations, `crates/screenpipe-redact/src/worker/{mod.rs,tables.rs}`, new `redaction_map.rs`, `crates/screenpipe-config/src/recording.rs` (`pii_redaction_columns: Option<Vec<String>>`), engine wiring in `privacy_filter.rs`/`core` startup.

---

### vision-redaction-pseudonyms-a11y - pseudonyms, a11y-aware secrets, worker budgeting (new, L)

**Observed:**
- Pseudonyms (`crates\screenpipe-redact\src\pseudonym.rs`, `0x6266a48..0x6266ba6`): key file `.redaction/pseudonym.key` under the data dir; created atomically on first use; if the file exists but has the wrong length, the worker will not replace it (replacing it would invalidate tokens already issued) and tells the user to remove the file manually to rotate the key. Setting `piiRedactionPseudonyms` (bool, default false; exposed as a settings checkbox for stable pseudonyms). Pipeline log field `pseudonyms=` (`0x62768ae`). Placeholder labels `[PERSON] [EMAIL] [PHONE] [ADDRESS] [URL] [COMPANY] [REPO] [HANDLE] [CHANNEL] [ID] [DATE] [SECRET] [SENSITIVE]` (`0x6266640`).
- A11y context secrets (`crates\screenpipe-redact\src\a11y_context.rs`, `0x627bf88..0x627c280`): reads node `role`, `on_screen`, `is_password`, `input_value`, `password`, `bounds` (`left/x, top/y, width/w, height/h`), `placeholder`, `help_text`, `role_description`, `automation_id`, `class_name`, `is_focused`, `is_keyboard_focusable`, secure text field roles (`AXSecureTextField`, `securetext`); label hints `passwd passphrase api key api_key access token auth token refresh token client secret private key credential`; token prefixes `sk- sk_ github_pat_ ghp_ gho_ ghu_ ghs_ ghr_ glpat- xoxb- xoxp- xoxa- xoxr- xapp- AIza whsec_ hf_ npm_ pypi- age-secret-key- tskey- flw...` and PEM `-----BEGIN ... PRIVATE KEY` / `SECRET KEY`.
- Image worker context query (`0x626c3d1`): picks newest frame with `name IS NOT NULL AND image_redacted_at IS NULL` older than a minimum age, joining `frames ctx ON ctx.id = f.elements_ref_frame_id` and using the ref frame's `accessibility_tree_json` when the two frames are within `?2` seconds and share `app_name` and `window_name`. Non-still-image names (mp4 chunks) and missing JPEGs are stamped redacted to skip (`0x626c850..0x626c8fc`).
- Secret continuation (`crates\screenpipe-redact\src\image\secret_continuation.rs`, `0x627c4b0`) with detection fields `bbox label score image_b64 labels detections` (`0x627c5c0`).
- Worker CPU governor (`0x6265360`): fields `work_units, batch_limit, next_batch_limit, target_slice_ms, active_cpu_percent, idle_cpu_percent, worked_ms, cooldown_ms, target_cpu_percent`; crate `screenpipe-resource` provides a shared low-priority CPU lane for background work (`0x626df58`).
- AI adapter failure circuit (`0x6276590..0x6276910`): `cooldown_seconds`; logs for an AI redactor failure that switches output to regex-only, for recovery that closes the failure circuit, and for the configured detector being in back-off. Pipeline modes `pipeline+regex`, `onnx`, `pipeline+ai`, `pipeline+onnx`, `pipeline+tinfoil` (`0x6276970`). Policy type `TextRedactionPolicy{allow, allow_subtypes}` (`0x6276470`, base has it).

**Behaviour:**
- Pseudonyms: when enabled, replacement for a detected span is `[LABEL_<token>]`-style stable token = HMAC-SHA256(key, label + normalised value) truncated (format inferred: e.g. `[PERSON_3f9a1c]`), so the same entity maps to the same token everywhere; disabled => plain `[LABEL]`. 32-byte key (inferred), file perms owner-only.
- A11y-aware secrets: before/alongside model detection, mark as `SECRET` any text of nodes that are password fields, secure text roles, or whose label/placeholder/help/automation id contains a hint word, and any token matching the prefixes; for image redaction, convert those nodes' normalised bounds into pixel boxes to black out even if the image model misses them.
- Secret continuation: when a secret region was blacked out in frame N, also black out the same region in the following frames of the same app/window while the a11y context shows the field still present (inferred).
- CPU governor: adapt batch size so the worker stays below a target CPU percentage (inferred default 15 % active, 5 % on battery), sleeping `cooldown_ms` between slices.
- Failure circuit: after an AI adapter error, use regex-only output for `cooldown_seconds` (inferred 300 s) then retry.

**Base files:** new `crates/screenpipe-redact/src/{pseudonym.rs,a11y_context.rs,redaction_map.rs,image/secret_continuation.rs}`, `pipeline.rs`, `image/worker.rs`, `worker/mod.rs`; config `pii_redaction_pseudonyms`.

---

### vision-redaction-models - model artefact versions (changed, S)

**Observed:** image detector `rfdetr_v38.onnx` (~60 MB) from `https://huggingface.co/screenpipe/pii-image-redactor/resolve/main/rfdetr_v38.onnx`, sha256 `0e968657aafcfb2c5d61ece264d5959ddeeec53091171fae226b45ecd3a8a880` (`0x49ad3f8..0x49ad4bc`); text ONNX dir `.screenpipe/models/v50_distilled6l` from `https://huggingface.co/screenpipe/pii-redactor/resolve/main/v50_distilled6l` with `model_quantized.onnx` (`e67efa3a511108b6864db3cb19be37853524e6ed4f0f26cc041ecb822ba12e6e`), `tokenizer.json` (`14c7e8bf7d9b58ca061fcda93bc8d0eedd1a51ffc3af01a1ba1ef54e2154887e`), a third file hashed `ab11d9f79693e29c0d9f11de185e48d80462e95799030c37bfa08c14379a9244` (likely `config.json`), `remap.json` (`334df93eb6843ecc7ea0939ab6450df4df203e21d5227bbcb9f7c2af19f98827`) (`0x49ad0b0`); OPF `opf-v6` from `.../pii-text-redactor/resolve/v6` (`config.json` `48cd9b76...`, `model.safetensors` `247779eb...`) - base already uses v6. Base: `rfdetr_v12`, `v45_phase5_pruned`.

**Recommendation:** these weights were published after the relicense; their licence is unknown. Keep base models as defaults. Make model id/URL/sha256 configurable (settings or env) so a user can opt into newer artefacts they obtain themselves. Record the target names/hashes only as interop data. Partial downloads use `.partial` suffix and sha256 verification before rename (base behaviour).

**Base files:** `crates/screenpipe-redact/src/adapters/{rfdetr.rs,onnx.rs}`.

---

### vision-tinfoil-decloud - remove vendor PII enclave (decloud, S)

**Observed:** env `SCREENPIPE_PRIVACY_FILTER_ENCLAVE`, `SCREENPIPE_PRIVACY_FILTER_REPO` (default `screenpipe/privacy-filter`), `SCREENPIPE_PRIVACY_FILTER_URL`, `SCREENPIPE_PRIVACY_FILTER_API_KEY`, host `pii.screenpipe.containers.tinfoil.dev`, endpoint path `/image/detect`, attestation drift handling (`0x627b996..0x627ca3a`); falls back to `SCREENPIPE_API_AUTH_KEY` / `SCREENPIPE_LOCAL_API_KEY`. Base contains the same adapter (`adapters/tinfoil.rs`, `tinfoil_image.rs`) and `piiBackend: "tinfoil"`.

**Action:** default `piiBackend="local"`; remove the hard-coded vendor host and default repo. Either drop the `tinfoil` backend or keep it only as "custom attested endpoint" requiring an explicit user-provided URL + key (no default). Never forward screenpipe cloud tokens.

**Base files:** `crates/screenpipe-redact/src/adapters/tinfoil*.rs`, `pipeline.rs`, `recording.rs` doc/defaults.

---

### vision-telemetry-decloud - capture analytics stay local (decloud, S)

**Observed:** PostHog events `vision_capture_wedge_restart` (`0x49a9108`) and `vision_pipeline_health` (`0x49ac1d0`, fields `ocr_cache_misses frames_video_written frames_db_written time_to_first_frame_ms video_queue_depth ocr_gate_crop_ocr ocr_gate_detects avg_ocr_gate_detect_latency_ms`), resource posting to `https://us.i.posthog.com/capture/` (`0x6a7826d`).
**Action:** emit these as local `tracing` events only (and in `/health`, `/vision/metrics`); no network sink. Keep `analytics.rs` disabled by default.

---

### vision-history-access-stub - no 24-hour history gate (ee-stub, S)

**Observed:** frame/element/export/meeting routes return `{"error": <message saying the content falls outside the permitted 24-hour window>, "code": "history_access_limited"}` (`0x59fbb50`, `0x6a1cbd8`, `0x620a240`); raw SQL is refused with a message tying it to the 24-hour local history limit (`0x5152f2d`); module `history_access.rs` (`0x6a64d61`); Tauri command `is_history_access_restricted` (`0x5177a68`). This is a plan/licence gate.
**Action:** implement `history_access` as an always-unrestricted stub (function returns "allowed"); do not add the error path to `/frames/*`, thumbnails, preview samples/media or `/elements`. If the code identifier is needed for client compatibility, define the error variant but never produce it.

---

### vision-starred-hd - HD capture bound to starred sessions (new, S; depends on starred-sessions dimension)

**Observed:** `high_fps_controller.rs` has a "starred HD" mutex (`0x6a69290`); table `starred_sessions(id, start, end, hd_requested INTEGER NOT NULL DEFAULT 0, revision, ...)` (`0xa2ae2e5`, `0x516d1d0`); API doc (paraphrased): starring uses `POST /starred-sessions` with `hd_requested`; asking for HD is opt-in, and HD capture still honours pauses and exclusions (`0x511e39f`). HD session kinds and logs otherwise identical to base (meeting / timer / prewarm-pending, `0x6a69360..0x6a69d20`).

**Behaviour:** while `now` falls inside any starred session with `hd_requested=1`, the HD controller reports active with the configured `hdRecordingIntervalMs` (default 100 ms, floor 33 ms), exactly like a timer-bound session ending at the session `end`; pauses (lock, DRM, schedule, user-paused monitor, ignored windows) still suppress capture. Session edits (revision bump) re-evaluate.

**Base files:** `crates/screenpipe-engine/src/high_fps_controller.rs` (add `starred` source), starred-session store hook.

---

### vision-frame-comparison-hardening - diff robustness (changed, S)

**Observed (`crates\screenpipe-screen\src\frame_comparison.rs`, `0x9f59961..0x9f59dd5`):** a log for skipping the comparison when the thumbnail buffer could not be allocated; a hash-match fast-path log with a hit counter; two size invariants (the two grayscale thumbnails must be the same size, and the images must be the same size once resized); structured failure fields `source_width`, `source_height`, `failures`.
**Behaviour:** compute the downscaled grayscale thumbnail with fallible allocation (`Vec::try_reserve`); on failure return "no change" (0.0) and count failures, logging dimensions once per N failures; when previous and current thumbnails differ in size (monitor resolution change) reset baseline instead of panicking; keep hash fast path. Defaults unchanged (downscale factor 4).

**Base files:** `crates/screenpipe-screen/src/frame_comparison.rs`.

---

## 3. Settings keys summary (serde names, `0x5a42a38`)

Vision-relevant keys in target `RecordingSettings` and status vs base:
`disableVision` (base), `disableScreenshots` (new), `enableSemanticContext` / `semanticContextMode` = `memory|computerUse|both` (new, semantic dimension; affects `format=preferred`), `disableTimeline` (base), `monitorIds` (base), `useAllMonitors` (base), `videoQuality` (base), `captureMaxWidth` (new), `maxSnapshotWidth` (base), `disableSnapshotCompaction` (base), `idleCaptureIntervalMs` / `visualCheckIntervalMs` / `visualChangeThreshold` / `minCaptureIntervalMs` (base), `hdRecordingDefault` / `hdRecordingIntervalMs` (base), `captureOnKeystroke` / `captureOnClipboard` / `captureScroll` (base), `prioritizeInputLatency` / `extractionThreadPriority` (`normal|below_normal|lowest|idle`) / `pauseExtractionOnInputMs` (base), `ignoredWindows` / `includedWindows` (base), `ignoredUrls` (type changed), `includedUrls` (new), `ignoreIncognitoWindows` (base), `enhancedIncognitoDetection` (new, macOS browser automation; no Windows effect), `pauseOnDrmContent` (base), `disableClickCapture` (new, ui-events dimension), `recordWhileLocked` (base), `languages` (base), `usePiiRemoval` / `asyncPiiRedaction` / `asyncImagePiiRedaction` / `piiBackend` / `piiRedactionLabels` (base), `piiRedactionColumns` (new), `piiRedactionPseudonyms` (new), `redactAgentSessionSecrets` (new, agents dimension), `powerMode` (base), `recordingDetail` (new), `keepComputerAwake` (new, power dimension), `deviceTier` (base). Frontend also keeps legacy `ocrEngine` and `fps` (0.1-60) fields in its zod schema; engine ignores them.

## 4. Implementation order (suggested)

1. Schema: `vision-ocr-unify-frames`, `vision-redaction-surfaces` migrations, `vision-display-layout`, `vision-compaction-encoder` index.
2. Engine API: `vision-device-control`, `vision-thumbnail-endpoint`, `vision-preview-samples`, `vision-preview-media`, `vision-elements-formats`.
3. Capture robustness: `vision-capture-wedge-watchdog`, `vision-probe-timeouts`, `vision-capture-loop-refinements`, `vision-pipeline-metrics`, `vision-corrupt-frame-guard`, `vision-frame-comparison-hardening`.
4. Windows backends: `vision-wgc-native-capture`, `vision-windows-ocr-worker`, `vision-tree-walker-worker`, `vision-windows-window-resolution`, then `vision-uia-retained-capture`.
5. Settings: `vision-url-domain-rules`, `vision-disable-screenshots`, `vision-capture-max-width`, `vision-recording-detail`.
6. OCR gate, redaction extras, starred HD, compaction encoder probing.
7. Hybrid storage only after the storage dimension lands.

## 5. Open questions

- Exact numeric ranges for thumbnail `width`/`quality`, preview `limit` max, wedge thresholds, frozen-stream threshold, OCR worker queue size, text-region detector parameters are not present as strings (compiled constants). Values above marked "inferred" are proposals.
- Whether xcap monitor ids equal the target's `EnumDisplayMonitors`-derived ids (affects `monitorIds` persistence and `/vision/device/*` ids).
- Whether user-paused monitors persist across restarts (no store key observed; assumed in-memory).
- Migration version numbers for "unify ocr text into frames", "frames app name timestamp index", "add frames snapshot pending index", "capture action fidelity", "make redacted at indexes partial" are not stored as strings; only ordering constraints are known.
- Whether Freenpipe must read screenpipe hybrid (`hybrid-parquet-v1`) data directories, which would pull the whole storage layer forward.
- Licence of `rfdetr_v38` / `v50_distilled6l` weights.
- Which trigger string intermediate scroll snapshots use (`recordingDetail`).

---

## Verification

Adversarial pass, 2026-10-14. Method: (a) grepped the BASE (`<repo>` @ `0e2397a`, only the import + notice commits) for every identifier, message and table; (b) re-read every cited offset with `tools\vv_check.py` (does the substring occur at or just after the offset?) and `tools\verify_offs.py`; (c) checked overlap with the sibling specs `db.md`, `api.md`, `cloud.md`, `frontend.md`. For the independent sweep I used `tools\vv_newsrc.py` (target `.rs` paths absent from base) and `tools\vv_newmsg.py` (target strings with no 24-char window in base sources) over the capture-loop, watcher, paired-capture, screen-crate and a11y-Windows string regions. REA: `trace_application_feature` on `/vision/device/stop` returned no match on the partial graph (`ev_6eb0cefa9aed2cad5137a032934b1a8d01b6ff70347b5c3ef7539d8bb787c974`, parent `ev_57f23857...`), so I confirmed frontend claims by grepping the extracted chunks directly.

Every cited offset resolved to the claimed text. A few point at the DDL rather than the migration description (descriptions sit at `0xa2a3b29`, `0xa2a4eff`, `0xa2a6f24`, `0xa2a7d54`), and a few are off by one line. None of that is material.

### Verdict table

| id | verdict | notes |
|---|---|---|
| vision-device-control | confirmed | Routes, `VisionDeviceControlRequest{monitor_id}`, messages and watcher skip all verified. Base has only `/audio/device/*`. The frontend maps status rows using `e.id`, `e.name` and `e.user_disabled`, so the status key is `id`. **api-016 is wrong** to call it `monitor_id`, and its claim that pauses are "persisted across restarts" has no evidence (no store key in strings or frontend). Treat this spec as canonical and fix api-016. |
| vision-display-layout | confirmed | DDL, insert/select and the delete-range "keep newest row" verified (`0x5140d50..0x5140dcc`). Schema and queries overlap db-007 / db-m02; the watcher-side persistence (startup/change, timeout, retry) is vision's. Correction: the migration version is **20260702130000** (db-001), not `2026062x`. |
| vision-capture-wedge-watchdog | confirmed | Stage list, watchdog classes and restart messages verified (`0x49a90c9`, `0x49a9172`). Base has no `loop_stage` or watchdog. Thresholds are inferred, as stated. |
| vision-probe-timeouts | confirmed | `visual_probe.rs` and `capture_exclusions.rs` are new; base calls `capture_monitor_image` / `get_excluded_sck_window_ids` inline without timeouts (`event_driven_capture.rs:1186-1193`). **Correction:** the spec says to keep the cached exclusions on timeout. The target instead fails closed: when the exclusion state is unknown it logs that frame pixels are withheld until a later refresh works (`0x525502a`). See missed item vision-exclusion-unknown-fail-closed. |
| vision-capture-loop-refinements | confirmed | Frozen-stream, soft-checkpoint deferral and the new timeout wording are absent from base. Base already releases the stream at pause entry (`event_driven_capture.rs:907`) and logs power-profile screenshot disable (`:1001`). The delta is the release-ahead-of-focus-probes ordering, the a11y-worker suspend and the `managed/config` reason. Point 5 overlaps vision-disable-screenshots. |
| vision-wgc-native-capture | confirmed | Base `wgc_capture.rs` is 166 lines wrapping `xcap::VideoRecorder`. Target events reach `wgc_capture.rs:879`, with WARP fallback, device-removed recovery, `SetMinUpdateInterval`, `CreateFreeThreaded` and staging-texture readback all observed. |
| vision-corrupt-frame-guard | partially_in_base | Base already drops mostly-black frames before the DB write (`is_frame_mostly_black`, `event_driven_capture.rs:1779-1784`, with the same skip log as the target). New in target: the "green decode-garbage **band**" check (`0x525513d`, a band rather than a whole green frame as the spec infers) and the two counters. "Release persistent stream after N corrupt frames" has no evidence; drop it or mark it as a proposal. |
| vision-pipeline-metrics | confirmed | All 13 new `MetricsSnapshot` fields verified against base `metrics.rs:324-352`. The `vision_reason` enum and `loop_stage*` are new. Base already computes `silent_loss` inside the stall-summary log line (`health.rs:509-523`). The new parts are the four classification phrases (`0x5256730..0x52567f6`) and exposing `silent_loss` as a metric. Overlaps api-033 (health v2). |
| vision-ocr-gate | confirmed | `text_regions.rs`, gate logs and metrics verified; nothing in base. The detector is inferred, as stated. |
| vision-ocr-unify-frames | duplicate | Fully covered by **db-005** (migration, backfill, insert path, readers, OCR element roles) plus **db-006** (`idx_frames_app_name_timestamp`). The AX id reservation is in db-022. Also, base already writes OCR elements with `source='ocr'` and page/block/paragraph/line/word roles (`db.rs:252`, `2628-2651`). Versions: 20260613130000 and 20260618120000. |
| vision-windows-ocr-worker | confirmed | Worker thread, queue and validation strings are new (base `microsoft.rs` has no thread or semaphore). Partly in base: tesseract `TESSDATA_PREFIX` candidates and `tessedit_create_tsv` (`tesseract.rs:8-50`). New: bundled-binary discovery, `APPDIR`, per-session disable and the `ocr_unavailable` reason. |
| vision-tree-walker-worker | confirmed | `tree_walker_worker.rs` is new. Base runs walks with `spawn_blocking` (`event_driven_capture.rs:1869`). |
| vision-uia-retained-capture | confirmed | `retained_tree.rs` and `capture_diagnostics.rs` are new. Base registers only a focus-changed UIA handler (`windows_uia.rs:508-531`). Small fix: `None/Timeout/MaxNodes/Pending` is base's `TruncationReason` (`tree/mod.rs:305`) plus a new `Pending` variant, not a new walk-result enum. |
| vision-windows-window-resolution | partially_in_base | Already in base: the identical 12-entry `EXCLUDED_APPS` list (`tree/windows.rs:34-48`; not a "subset"), `TRANSIENT_SHELL_WINDOW_CLASSES` incl. `CiceroUIWndFrame`/`tooltips_class32`/`TopLevelWindowForOverflowXamlIsland` (`platform/windows.rs:1558`), `SHELL_HOST_PROCESSES` + `normalize_app_name`, typed `SkipReason` (Incognito/ExcludedApp/UserIgnored/NotInIncludeList/BlockedUrl, `tree/mod.rs:509`), and the extension-popup skip with the same log (`tree/windows.rs:311`). New: UWP-hosted app resolution behind ApplicationFrameHost (`0xa038e38`), new `BlockedUrl` display text that names the browser URL policy, and the new URL-validation-pending reason (part of vision-url-domain-rules). |
| vision-url-domain-rules | confirmed | Base `url_filter.rs` is only `is_url_blocked(&str, &[String])`. `DomainRule`/`UrlRule`, `includedUrls`, validation messages and frontend zod schema all verified. |
| vision-disable-screenshots | confirmed | Key, default `false`, zod field and health reasons verified; nothing in base. |
| vision-capture-max-width | partially_in_base | Base already caps capture resolution at capture time on macOS (`set_sck_capture_max_width` from `videoQuality`, `monitor.rs:85`, `manager.rs:443`). New: the `captureMaxWidth` override key (key only; no UI, no log) and any Windows application, which is inferred. |
| vision-recording-detail | confirmed | Enum, default `auto` and the settings card cadences (1-5 s / 5 s / 2 s / 1 s) verified in `page-d9c69007346aaf6a.js`. No engine-side scroll-snapshot trigger string exists, so the open question stands. |
| vision-thumbnail-endpoint | confirmed | Handler, headers, ETag prefix and errors verified; base has no thumbnail route. Overlaps api-010; this spec has more behavioural detail. |
| vision-preview-samples | confirmed | Verified; absent from base. Overlaps api-010. |
| vision-preview-media | confirmed | Verified; absent from base. Overlaps api-010. |
| vision-elements-formats | confirmed | Base `ElementsQuery` has no `format`/`fields`. `ElementState` is exactly `focused, selected, expanded` (`0x59fbc06`); `collapsed`, `disabled` and `off-screen` are outline text markers only. Overlaps api-004. **api-004's `table` format is unsupported**: the only `table` hit is inside "stable keys", and `response_format.rs` emits just `text/csv` and `text/tab-separated-values`. |
| vision-compaction-encoder | confirmed | Base hard-codes `libx265` (`snapshot_compaction.rs:414`). Encoder list, per-encoder args and env var verified (`0x6a78a00`, `0x6a9dbf8`). The partial index is db-006 (version 20260702000000). |
| vision-hybrid-payload-storage | duplicate | The spec itself defers this to storage. Fully specified by **db-021** (hybrid format), **db-022** (bulk elements vtab), db-023/024. Keep only the four "vision-side requirements" as cross-references. |
| vision-redaction-surfaces | confirmed | Worker SQL, `piiRedactionColumns` and `redaction_map.rs` are new. Base redacts via `*_redacted` sibling columns (`20260501000000_add_redacted_columns.sql`). The migrations themselves are db-003/db-004; `make_redacted_at_indexes_partial` = **20260709000000** (db-001). |
| vision-redaction-pseudonyms-a11y | confirmed | `pseudonym.rs`, `a11y_context.rs`, `secret_continuation.rs`, CPU governor and failure circuit are all absent from base. |
| vision-redaction-models | confirmed | Base pins `rfdetr_v12.onnx` (`rfdetr.rs:121-129`) and `v45_phase5_pruned` (`onnx.rs:112`); OPF v6 is already base (`opf.rs:100`). Target hashes verified. |
| vision-tinfoil-decloud | duplicate | Same action as **cloud-020**. This is base code, not a target delta: base already defaults `piiBackend` to `"local"` (`recording.rs:637`) but hard-codes `DEFAULT_ENCLAVE = pii.screenpipe.containers.tinfoil.dev` (`tinfoil.rs:76`, `tinfoil_image.rs:58`). |
| vision-telemetry-decloud | duplicate | Covered by **cloud-010** (removes engine `analytics.rs`/PostHog entirely) and cloud-040. Base **already** posts `vision_pipeline_health` to PostHog every 60 s (`server.rs:420-453`); only `vision_capture_wedge_restart` is new. |
| vision-history-access-stub | duplicate | Covered by **api-035** (engine routes; same offsets) and frontend-052 (`is_history_access_restricted -> false`). |
| vision-starred-hd | confirmed | "starred HD mutex" in `high_fps_controller.rs` (`0x6a69290`) and `hd_requested` verified. Base `SessionKind` has only Meeting/Timer/PrewarmPending. Depends on db-015 / api-007. |
| vision-frame-comparison-hardening | partially_in_base | Base already has the hash fast path with the same log (`frame_comparison.rs:212`), resize-on-dimension-mismatch and the post-resize equal-size invariant (`:316-348`). New: fallible thumbnail allocation that skips comparison on failure, structured `source_width/source_height/failures` logging, and the grayscale-thumbnail invariant. The spec's "reset baseline on dimension change" has no evidence; base resizes instead, so keep that. |

Tally: 23 confirmed, 4 partially_in_base, 5 duplicate, 0 unsupported.

### Missed items

1. **vision-capture-identity-gate (new, M).** Strings with no base equivalent in `event_driven_capture.rs` (described, not quoted):
   - a frame rejected because the foreground identity changed during the capture or could not be determined (`0x5255354`)
   - AX text kept but flagged as not pixel-aligned because the focused process differed between the screenshot and the AX walk (`0x52553c0`)
   - the focused-metadata lookup task failing or timing out (`0x52551c8`, `0x52551f0`)
   - a capture skipped because the resolved app or window hits the privacy filter (`0x5255580..0x52555bf`)
   - a capture skipped because the resolved browser URL fails the URL policy (`0x5255546`)

   Behaviour (paraphrase):
   - Snapshot the foreground identity (hwnd/pid) before and after the screenshot. If it changed or is unknown, drop the frame.
   - If only the AX-walked process differs from the screenshot's, keep the AX text but mark it not pixel-aligned (no bounds-to-pixel joins).
   - Run the focused-window metadata lookup as a task with a timeout.
   - After resolving the real app, window and URL, re-apply the ignore/privacy filters and URL policy before storing anything.
2. **vision-exclusion-unknown-fail-closed (changed, S).** When the excluded-window refresh fails or times out and exclusion state is unknown, the target keeps frame pixels out of storage until a subsequent refresh works (`0x525502a`; a11y-only capture can continue). This replaces the "keep cached exclusions" wording in vision-probe-timeouts.
3. **a11y-input-hook-watchdog (new, M; UI-events dimension, not vision).** `platform\windows.rs`:
   - a dedicated `a11y-input-worker` thread (`0xa037af8`)
   - a raw input queue with a soft cap that drops events when the worker falls behind (`0xa037dc8`)
   - a watchdog that reinstalls the LL keyboard/mouse hooks when system input is active but hook callbacks are silent, since Windows `LowLevelHooksTimeout` silently removes them (`0xa037ff0`, `0xa038020`)
   - the hook thread raised to HIGHEST priority (`0xa0394d0`)

   Base installs the LL hooks (`platform/windows.rs:446-456`) but has none of this. No spec covers it yet; route it to the UI-events owner.

Checked and **not** missed (already in base): multilingual incognito title keywords (`incognito/titles.rs`), VS Code/Obsidian document-path resolution (`tree/electron_docs.rs`), `/frames/:id` 7-day cache header. `search_only.rs` and `live_views.rs` are new, but api-034 and api-020 own them.

### Corrections to carry into implementation
- Migration versions are known (db-001), so drop them from the open questions: unify ocr text into frames `20260613130000`, frames app name timestamp index `20260618120000`, add frames snapshot pending index `20260702000000`, capture action fidelity `20260702130000`, make redacted at indexes partial `20260709000000`.
- `/vision/device/status` entries use `id` (not `monitor_id`); no evidence of persistence.

---

## Verification (pass 2, 2026-10-15)

This is an independent second adversarial pass. I re-checked every item against the base and the target myself and only then compared with pass 1 above.

**Method**
- **Base (a).** Grepped `<repo>` (crates and app, excluding `target/`) for every identifier, route, table, settings key and log message. Read the base code that the claims depend on:
  - `event_driven_capture.rs`: pause gate at :896-943, power-profile screenshot disable at :998-1003, `is_frame_mostly_black` at :2170
  - `screenpipe-screen/src/metrics.rs` `MetricsSnapshot` (21 fields)
  - `routes/elements.rs` `ElementsQuery` (no `format`/`fields`)
  - `routes/frames.rs` error paths at :122-222
  - `tree/mod.rs` `SkipReason` + `Display` (:509-531) and `TruncationReason` (:305)
  - `platform/windows.rs` `TRANSIENT_SHELL_WINDOW_CLASSES`, `SHELL_HOST_PROCESSES`, `normalize_app_name` (:1558-1650)
  - `frame_comparison.rs`, `high_fps_controller.rs` `SessionKind`, `paired_capture.rs` `OCR_SEMAPHORE`, `microsoft.rs`, `tesseract.rs`, the redact adapters and `worker/tables.rs`
- **Target (b).** Re-read every cited offset with `tools\vv_check.py` and with a new `tools\vv2_at.py`, which prints the strings text around a hex offset. Grepped the frontend claims directly in the extracted chunks:
  - home page `/vision/device/*` toggling and status mapping
  - `3308-*.js` thumbnail and preview-media helpers
  - `378-*.js` defaults
  - `7451-*.js` zod schema
  - `7737-*.js` `disableScreenshots`
  - the settings-page Scroll capture card
- **Duplicates (c).** Compared each item against `db.md`, `api.md`, `cloud.md`, `meetings_memory.md` and `frontend.md`.
- **Independent sweep.**
  - New `tools\vv2_newsrc.py`: target source paths with no base file, over the screen, capture, a11y, redact, resource, engine, core and db crates.
  - New `tools\vv2_newmsg.py`: target strings with no 20-char window in the base `.rs`/`.sql` sources, over `0x49a7000-0x49ae200`, `0x5254c00-0x525a400`, `0x59fa400-0x59fbd00`, `0x9f56000-0x9f5e200` and `0xa036000-0xa042400`.
  - Outputs are in `tools\out\vv2_engine_new.txt`, `vv2_paired_new.txt`, `vv2_screen_new.txt` and `vv2_a11y_new.txt`.
- **REA.**
  - `trace_application_feature` with string seed `/frames/preview-media/` on parent `ev_57f23857...` returned no match (`ev_e2772b716b031484d35bf71730c8b58746ed27f6dfe83b89438ced8b196c86a6`; the graph is partial).
  - The module item view for `3308-e676c712bfd27088.js` (`ev_5e2965250ccf21bf81f6c79a2ca8b022def4f7f7bbb9afa2fae0e236d783730f`, the ID this spec cites) holds module metadata but **none** of the URL literals. On its own it does not support the helper-URL claims, so I confirmed them by grepping the chunk: the thumbnail helper sets `fallback=false` and builds `/frames/<id>/thumbnail?...`, and the preview helper builds `/frames/preview-media/<id>`.

**Offsets.** Every cited offset resolves to the claimed text, sometimes a few runs later inside the cited range. Examples: the `hevc_*` list is at `0x6a78a00`, the elements-worker SELECT at `0x626d678`, the `frame_payloads` DDL at `0x49e7a51` and the ref-frame join at `0x626c64c`. Nothing is unsupported.

### Verdict table (pass 2)

| id | verdict | notes |
|---|---|---|
| vision-device-control | confirmed | No `/vision/device` or user-paused state anywhere in the base. The home page maps status rows with `e.id`, `e.name` and `!e.user_disabled`, and posts `{monitor_id}` (frontend `home/page-c18385b042b3382e.js`). The status key is `id`, not api-016's `monitor_id`. No settings/store key for paused monitors exists in the strings, so treat the state as in-memory. |
| vision-display-layout | confirmed | The DDL and the insert/latest/keep-newest delete are verified. The schema duplicates db-007 (migration `20260702130000`). The watcher-side persistence is vision's own: startup/change rows, timeout, retry next pass (`0x49a9770`, `0x49a97cd`). |
| vision-capture-wedge-watchdog | confirmed | Stage list `0x49ad5c8`; classes and restart messages `0x49a8fa0..0x49a91f0`. The base watcher only restarts for DRM and schedule resume. |
| vision-probe-timeouts | confirmed | `visual_probe.rs` / `capture_exclusions.rs` do not exist in base, and base calls `capture_monitor_image` / `get_excluded_sck_window_ids` inline (:837, :1186). **Correction:** when exclusion state is unknown the target stores no frame pixels (`0x525502a`), so "keep cached exclusions" is wrong for pixels. See missed item vision-exclusion-unknown-fail-closed. |
| vision-capture-loop-refinements | confirmed | New: frozen-stream rebuild, the timeout drop wording, soft-checkpoint deferral, and pause gate ordered **before** focus probes. In base the pause gate (:896) runs after the focus gate (:797). Also new: a11y-worker suspend and the `managed/config` reason. Pause-entry stream release itself and the power-profile screenshot disable already exist in base. Point 5 overlaps vision-disable-screenshots. |
| vision-wgc-native-capture | confirmed | Base `wgc_capture.rs` is 166 lines over `xcap::VideoRecorder`. The target has WARP fallback (`0x9f57545`), device-removed recovery (`0x9f57638`), `SetMinUpdateInterval` (`0x9f57df2`), `CreateFreeThreaded` (`0x9f580d7`), staging readback (`0x9f585e1`) and `EnumDisplayMonitors` (`0x9f57677`). `xcap-0.9.6` is still linked (crates.txt), which fits the per-frame fallback claim. |
| vision-corrupt-frame-guard | partially_in_base | Base already drops mostly-black frames before the DB write (`is_frame_mostly_black`, 15x15 grid, sum<15, >95%), with the same log as target `0x52550f9`. New: a green decode-garbage **band** check (`0x525513d`; a band, not a whole green frame) and the two counters. Releasing the stream after N corrupt frames has no evidence; mark it as a proposal. |
| vision-pipeline-metrics | confirmed | The packed field list at `0x522e3cc` adds exactly the 13 claimed fields to base's 21. `vision_reason`, `loop_stage*` and `recording_coverage` are absent from base. Base already computes `silent_loss` inside a health log line only (health.rs:509-520). The health surface overlaps api-033. |
| vision-ocr-gate | confirmed | `text_regions.rs`, gate logs `0x5259a09..0x5259aa0` and gate metrics all verified; nothing comparable in base. The detector algorithm is inferred, as the item states. |
| vision-ocr-unify-frames | duplicate | db-005 (migration `20260613130000`, backfill, `text_json` on frames, removal of `ocr_text` readers/writers) plus db-006 (`idx_frames_app_name_timestamp`, `20260618120000`). Base already inserts OCR `elements` rows with `source='ocr'` and page/block/paragraph/line/word roles (db.rs:252, :2628-2651); only `frames.text_json` and dropping `ocr_text` (40 refs in db.rs) are new. |
| vision-windows-ocr-worker | confirmed | New: worker thread, queue and image validation (`0x5a4327b`, `0x9f5ba7d`), the `ocr_unavailable` reason, and bundled/`APPDIR` tesseract discovery with per-session disable. Already in base: an OCR `Semaphore(1)` (paired_capture.rs:48), the OCR-unavailable language-selection messages (microsoft.rs:111-163), and `TESSDATA_PREFIX` / `tessedit_create_tsv`. |
| vision-tree-walker-worker | confirmed | `tree_walker_worker.rs` is a new file. Base walks run in `spawn_blocking` (:1869). All five worker messages verified at `0x5259bc9..0x5259d1d`. |
| vision-uia-retained-capture | confirmed | `retained_tree.rs` and `capture_diagnostics.rs` are new files. Base registers only a focus-changed UIA handler (windows_uia.rs:508-531). `Pending` is a fourth variant on base's `TruncationReason{None,Timeout,MaxNodes}` (`0xa03abd0`), not a new enum. |
| vision-windows-window-resolution | partially_in_base | Already in base: the identical 12-entry excluded-app list (tree/windows.rs:34-47), transient shell classes, `SHELL_HOST_PROCESSES` + `normalize_app_name` with the same app-name normalisation log, the transient-shell-window skip logs, the extension-popup skip log (tree/windows.rs:311), and typed `SkipReason` with four of the six display strings. New: UWP-hosted app resolution (`0xa038e38`), `BlockedUrl` display text changing from a user-ignored-URL wording to one that names the browser URL policy, and the URL-validation-pending reason (which belongs with vision-url-domain-rules). |
| vision-url-domain-rules | confirmed | Base has only `is_url_blocked(&str, &[String])` and `ignored_urls: Vec<String>` (recording.rs:300). `DomainRule`/`UrlRule`, `includedUrls`, the validation messages (`0xa039c71..0xa039fc1`) and the zod schema in `7451-*.js` are verified. |
| vision-disable-screenshots | confirmed | Key `0x5a42bd2`, default `!1` in `378-*.js`, and the enterprise-managed variant in `7737-*.js` are verified; nothing in base. |
| vision-capture-max-width | partially_in_base | Base already caps capture width at capture time on macOS from `videoQuality` (`set_sck_capture_max_width`, monitor.rs:85; manager.rs:443). New: only the `captureMaxWidth` key (`0x5a42c3f`), with no frontend reference and no log. Windows application and override semantics are inferred. |
| vision-recording-detail | confirmed | Enum and default verified; the settings card cadences (auto 1-5 s, low_impact ~5 s, balanced ~2 s, more_detail ~1 s) were read in `page-d9c69007346aaf6a.js`. Nothing in base. The scroll-snapshot trigger string is still unknown. |
| vision-thumbnail-endpoint | confirmed | No thumbnail route in base `server.rs` (:640-691). Handler, query type, headers and ETag prefix verified. Note: `remote_device` and `snapshot_missing` already exist as `/frames/:id` error types in base (frames.rs:123, :167), so reuse them. Overlaps api-010; this item is the more detailed spec. |
| vision-preview-samples | confirmed | Route, types, validation (`0x6a1e951`) and the frontend query builder (`/frames/preview-samples?`, home page) verified; absent from base. Overlaps api-010. |
| vision-preview-media | confirmed | Route, doc and errors verified; absent from base. Overlaps api-010. |
| vision-elements-formats | confirmed | Base `ElementsQuery` has no `format` or `fields`, and no `response_format.rs`. The `ElementState` serde struct is exactly `focused, selected, expanded` (`0x59fbc06`). Overlaps api-004/api-002, but this item is more accurate. api-004's extra state flags are only outline text markers (`0x6a1f4fb`). Its `table` alias has no evidence: the format error lists `json|csv|tsv` (`0x6a954cb`), and the word only appears in the doc phrase "columnar table" (`0x5116335`). |
| vision-compaction-encoder | confirmed | Base hard-codes `libx265 ultrafast` (snapshot_compaction.rs:414-418). The encoder list (`0x6a78a00`), per-encoder args (`0x6a9dbf8`), env var (`0x6a78910`) and worker messages are verified. The partial index is db-006 (`20260702000000`). |
| vision-hybrid-payload-storage | duplicate | The item defers to storage itself. db-021 (hybrid format, DDL, sealing, descriptor) and db-022 (bulk elements vtab) specify it fully. Keep only the four vision-side requirements as cross-references. |
| vision-redaction-surfaces | confirmed | The base worker surfaces are `ocr_text`, audio, `frames.accessibility_text` and `ui_events.text_content` (worker/tables.rs). The elements, full_text, derived-copy and multi-column ui_events SQL (`0x626d678`, `0x62b8fb8`), `piiRedactionColumns` and `redaction_map.rs` are all new. The migrations themselves duplicate db-003/db-004. |
| vision-redaction-pseudonyms-a11y | confirmed | `pseudonym.rs`, `a11y_context.rs`, `secret_continuation.rs`, `ocr_json.rs` and `tree_json.rs` are all new files. The `screenpipe-resource` crate (CPU lane, `0x626df58`) does not exist in base, and the failure circuit is new. |
| vision-redaction-models | confirmed | Base pins `rfdetr_v12.onnx` (rfdetr.rs:121-129) and `v45_phase5_pruned`. Target `rfdetr_v38` with sha256 at `0x49ad4bc` and `v50_distilled6l` at `0x49ad2c4` are verified. |
| vision-tinfoil-decloud | duplicate | Same action as cloud-020. This is base code rather than a target delta: base already defaults `piiBackend` to `local` (recording.rs:637) and hard-codes `DEFAULT_ENCLAVE` (tinfoil.rs:76, tinfoil_image.rs:58). |
| vision-telemetry-decloud | duplicate | Covered by cloud-010/cloud-040. Base already posts `vision_pipeline_health` to PostHog (server.rs:430); only `vision_capture_wedge_restart` (`0x49a912f`) is new. |
| vision-history-access-stub | duplicate | Covered by api-035 (same offsets) and the frontend `is_history_access_restricted` stub. |
| vision-starred-hd | confirmed | Base `SessionKind` has only Meeting, Timer and PrewarmPending. Stronger evidence than the cited mutex: target `SessionKind` variant list `MeetingTimerStarredPrewarmPending` at `0x6a707a0`. The route side is in api-007 and meetings_memory.md; this item owns the controller binding. |
| vision-frame-comparison-hardening | partially_in_base | Already in base: the hash fast path with the same log (frame_comparison.rs:212) and the post-resize equal-size invariant (:348). New: fallible thumbnail allocation that skips the comparison (`0x9f59cb8`), the grayscale-thumbnail invariant (`0x9f59d68`) and structured `source_width/source_height/failures` fields (`0x9f59a10`). "Reset baseline on dimension change" has no evidence, so keep base's resize. |

Tally: 23 confirmed, 4 partially_in_base, 5 duplicate, 0 unsupported. This matches pass 1 item for item. New in pass 2:
- The REA module view does not hold the frontend URL literals.
- The starred-HD `SessionKind::Starred` evidence.
- Exact base text for `SkipReason::BlockedUrl`.
- The source of api-004's spurious `table` alias.
- One additional missed item (no. 3 below).

### Missed items (pass 2)

1. **vision-capture-identity-gate (new, M)**. Independently re-found; nothing in base matches. Strings (described, not quoted):
   - frame rejected when the foreground identity changed mid-capture or is unknown (`0x5255354`)
   - AX kept but marked non-pixel-aligned when the focused process differs between screenshot and AX walk (`0x52553c0`)
   - focused-metadata lookup task failed / timed out (`0x52551c8`, `0x52551f0`)
   - post-resolution privacy re-checks: the resolved browser URL fails the policy, or the resolved app/window hits the privacy filter (`0x5255546`, `0x525558f..0x52555bf`)

   Behaviour (paraphrase):
   - Compare the foreground identity before and after the screenshot; drop the frame on a change or an unknown identity.
   - Keep AX text but mark it as not pixel-aligned when only the walked process differs.
   - Run the metadata lookup under a timeout.
   - Re-apply the app, window and URL filters after resolution.
2. **vision-exclusion-unknown-fail-closed (changed, S)**. When the window-exclusion refresh fails or times out, frame pixels are not stored until a refresh succeeds (`0x525502a`). This replaces the "keep cached exclusions" wording in vision-probe-timeouts.
3. **vision-frame-media-errors (changed, S; new in pass 2)**. `GET /frames/:frame_id` gains typed failure codes plus a recheck before declaring a snapshot missing:
   - `frame_media_unavailable`: frame has no media location (`0x5255e18`)
   - `snapshot_read_failed`: reading the snapshot file failed (`0x5255ea8`, `0x5255ed8`)
   - `snapshot_missing` only after a second exact-frame lookup rechecks the location (`0x5255da7`, `0x5255f00`), which covers a concurrent compaction moving the frame from JPEG to MP4

   Base (frames.rs:122-222) has `remote_device`, `snapshot_missing`, `ffprobe_not_found` and `video_corrupted`. On any snapshot read error it falls back to the nearest frame without distinguishing unreadable from missing, and it never re-looks-up the frame. No other spec covers these codes.
4. **a11y-input-hook-watchdog (new, M; UI-events or a11y owner)**. Re-confirmed:
   - `a11y-input-worker` thread (`0xa037af8`)
   - raw-input queue soft cap with drops (`0xa037dc8`, `0xa037def`)
   - LL-hook reinstall watchdog for `LowLevelHooksTimeout` removals (`0xa037ff0`, `0xa038020`)
   - hook thread raised to HIGHEST priority (`0xa0394d0`)

   None of this is in base, and no sibling spec mentions it. There is no UI-events spec in `specs\`, so it needs an owner.

Checked and not missed:
- Already in base: multilingual incognito titles, VS Code-family and Obsidian document paths (`tree/electron_docs.rs`, incl. Trae/VSCodium), `extractionThreadPriority` enum (`screenpipe-a11y/src/config.rs:159`), `LineSpan{char_start,char_count,bounds}` (target `db/text_positions.rs` is a db.rs split), hot-frame-cache day rollover.
- Owned by other dimensions: `screenpipe-resource` (shared CPU lane for redaction, storage and secrets), the video-export ffmpeg strings (`0x49a8600..0x49a88c2`), `disk_pressure.rs` (shell.md), `search_only.rs` / `live_views.rs` (api-034 / api-020).
