# Gap dimension spec: health and lifecycle internals

Covers recording-coverage accounting, resource monitor v2 (memory-leak trend, process breakdown) and the Windows suspend/resume monitor.

Target: screenpipe v2.7.104 Windows build (`<screenpipe-install>/screenpipe-app.exe`, engine embedded). The exe was never executed.
Base: Freenpipe @ upstream 892199f (`<repo>`).
Delta id prefix: `gap_health-lifecycle-internals-` (shortened below to `HLI-`).

**Clean-room note.** Everything below paraphrases static evidence about the shipped binary. Interop identifiers are quoted exactly: JSON fields, env vars, log-target names, file names, Win32 API names and enum values. Log message wording is summarised, not copied. No post-relicense upstream material was consulted.

**How the evidence was gathered.** The string table (`<re-workspace>/app_strings.tsv`) did not explain the logic: `recording_coverage.rs` emits no log lines at all. To get past that, I statically disassembled the installed exe, read-only, with MSVC `dumpbin /DISASM /RANGE`. Code VAs are quoted as `VA 0x14…`; string offsets are hex file offsets into `app_strings.tsv` / the exe. Reading the disassembly was for understanding behaviour only. No decompiled code is reproduced here.

New helper scripts, all in `<re-workspace>/tools/`:

| Script | Purpose |
|---|---|
| `hl_near.py`, `hl_range.py` | String-table windows |
| `hl_ptrscan.py`, `hl_locs.py` | Decode `&str` slices and Rust panic `Location {file,line,col}` records. These recover source line numbers. |
| `hl_xref.py` | numpy RIP-relative/rel32 reference finder |
| `hl_va.py` | VA ↔ file offset and constant reader |
| `hl_dis.py` | dumpbin wrapper that annotates operands with the literal, Location or f64 they point at |

Work files are in `<re-workspace>/work/health/`.

**REA evidence:**

| Evidence id | What it is |
|---|---|
| `ev_57f23857f31e9207a9f71e1d612412e78f9c5dc0d29af8c68f97372786b6216d` | Parent JS application analysis |
| `ev_10465921dacfc0c42c36c935bc278fb8b20aaccaee84e81a3e6004185f9d036a` | Seed trace `recording_coverage`: no frontend match |
| `ev_7c63fb564212c03f2ca25cb90fcffeac4a600dde91c9e9c0201a11f4183aa9e1` | Seed trace `screen_locked`: no frontend match |

A plain Grep of `frontend/` also found none of `recording_coverage`, `coverage_ratio`, `process_breakdown`, `RESOURCE_BREAKDOWN`, `mcp_external`, `phys_footprint`, `load_average`, `leak_suspected` or `suspend/resume`. This whole dimension is engine-internal: it surfaces only through `/health`, logs, a local file and (in the target) vendor telemetry.

---

## 0. Key structural facts

1. **`recording_coverage.rs` exists in the target and has no tracing events.**
   - Its only panic Locations are `recording_coverage.rs:202:5` (a `tokio::spawn`) and `:203:28` (a `tokio::time::interval`). Records are at 0x6a13260 and 0x6ab66e8.
   - The logic was recovered from code: VA 0x142605660 (classifier), 0x142605A90 (accumulate), 0x142605830 (snapshot), 0x1429F3460 (5 s ticker task) and 0x142605BE0 (spawn helper).
   - The doc comment on the `CoverageSnapshot` schema (0x6a0bf30..0x6a0c03c) states that it carries no private data, only wall-clock second counts and a derived ratio.
2. **"User active" means the UI recorder flushed input events recently.** It does not come from `GetLastInputInfo`.
   - The classifier inlines base `ui_recorder::ui_recorder_status_snapshot()`, i.e. `UI_RECORDER_LAST_EVENT_UNIX`, which `record_ui_event_flush` sets after each batch flush of UI events, i.e. the flush-count log (VA 0x141A2CD0D → 0x14259FB90).
   - `GetLastInputInfo` is imported (0xa6a7fa8), but its only caller is the a11y low-level-hook watchdog (VA 0x14344D4D1, the log reporting that the watchdog reinstalled the low-level input hooks). It is not used for coverage.
3. **Resource monitor v2 grew in size.**
   - Base `resource_monitor.rs` is 586 lines.
   - Target tracing callsites run up to `:760`, and panic Locations up to `:826`.
   - The process-breakdown logic sits beside the new `screenpipe-resource` crate's string pool (`crates\screenpipe-resource\src\cpu.rs`, 0xa2d06f0; code VA 0x143A15E70..0x143A17D20). The serde types `ProcessBreakdown` / `ProcessGroupUsage` / `ProcessUsage` (0x6a3e638) are serialised by engine code (VA 0x1426CAA80..).
4. **The Windows sleep monitor now uses a native power notification as well as polling.**
   - It registers `PowerRegisterSuspendResumeNotification` (import 0xa6a99c8; call at VA 0x1425B0DE3) in callback mode.
   - The callback (VA 0x1425A1140) only maintains a new global "system suspended" flag.
   - The base `OpenInputDesktop` 5 s lock poll and the clock-gap wake detector are kept.
   - The new flag is read only by snapshot compaction (VA 0x1427C86F3, 0x1429FBDAC).
5. **Every piece of this dimension also feeds vendor telemetry in the target.**
   - The engine's `resource_usage` and `vision_pipeline_health` PostHog events, and the app's `app_still_running` event.
   - Sentry receives the leak report.
   - Freenpipe implements the local parts only (CLAUDE.md "No phoning home"; cloud.md cloud-010 / cloud-011).

---

## 1. Base vs target inventory

| Area | Base (892199f) | Target (v2.7.104) | Delta |
|---|---|---|---|
| Recording coverage | none: no `recording_coverage.rs`, no `CoverageSnapshot`; `grep` of base for `recording_coverage`, `coverage_ratio`, `CoverageSnapshot` = 0 hits | process-lifetime accumulator: 4 buckets fed by a 5 s ticker; `CoverageSnapshot {recording_secs, active_stalled_secs, active_paused_secs, idle_secs, active_secs, coverage_ratio}` | HLI-001 |
| `/health.recording_coverage` | absent from `HealthCheckResponse` (`routes/health.rs:97-140`) | always-present object; built from the snapshot inside the health handler (VA 0x14108F2EE) | HLI-002 |
| Coverage in telemetry | n/a | `vision_pipeline_health` (engine, 60 s) and `app_still_running` (app) carry flattened coverage keys | HLI-003 (decloud) |
| Inputs to coverage | `ui_recorder_status_snapshot()` (`ui_recorder.rs:349`), `drm_detector::drm_content_paused()` (`:59`), `schedule_monitor::schedule_paused()` (`:20`), `PipelineMetrics.last_db_write_ts` + `started_at` (`screenpipe-screen/src/metrics.rs:95,79`) | same signals, read by the classifier | reused, no change |
| Process accounting | own process + **direct** children: memory sum, max virtual, cpu sum (`resource_monitor.rs:353-393`) | own process + **all descendants** (BFS over a parent→children map); adds available memory, swap, load averages, per-CPU load, footprint | HLI-004 |
| Status log line | DEBUG: runtime, memory % (used/total), virtual, CPU | DEBUG: runtime, memory % (used/total, available), swap used/total, footprint, virtual, CPU, load 1/5/15 | HLI-004 |
| Process breakdown | none (`ProcessBreakdown` etc. = 0 base hits) | groups `screenpipe_app`, `screenpipe_app_child`, `screenpipe_mcp_child`, `screenpipe_mcp_external`, `screenpipe_other`; top-12 by memory and by CPU | HLI-005 |
| Breakdown exposure | none | WARN log on "pressure"; INFO log when `SCREENPIPE_RESOURCE_BREAKDOWN` or `SAVE_RESOURCE_USAGE` is set; `process_breakdown` field in the JSONL file; per-group PostHog keys | HLI-006 |
| Memory-leak detection | none | linear regression over 6 h of memory samples, WARN once, plus a Sentry report | HLI-007 |
| Resource JSONL file | `SAVE_RESOURCE_USAGE` → `resource_usage_<ts>.json`, 7 keys, 10 MB truncate | same file and trigger, 18 keys incl. `process_breakdown` | HLI-008 |
| Startup hardware line | DEBUG "hardware: cpu= arch= gpus=" | INFO "host: os=<name> <version> kernel= cpu= arch= cores= gpus=" | HLI-009 |
| Vendor sinks | PostHog `resource_usage` with an embedded key (`resource_monitor.rs:335`) | key retained (0x6a78218) plus ~20 new properties and a Sentry leak message | HLI-010 (decloud) |
| Windows sleep/wake | `OpenInputDesktop` poll every 5 s + clock gap > poll+15 s → `mark_recently_woke` (`sleep_monitor.rs:604-650`) | same, plus `PowerRegisterSuspendResumeNotification` callback maintaining a "system suspended" flag; the clock-gap path clears it | HLI-011 |
| Compaction pause decisions | skips only when `!on_ac`; thermal throttle (`snapshot_compaction.rs:58-82`) | 6 ordered deferral reasons: system suspended, screen locked, on battery, OS low-power mode, capture power-paused, battery-saver mode; mid-cycle pause on suspend/lock | HLI-012 |
| Meeting detection while locked | not lock-aware | meeting-v2 scan loop and audio-process meeting detector skip iterations while the screen is locked | HLI-013 (cross-dimension) |
| Capture/audio lock handling | `event_driven_capture.rs:684,896,1989`, `screenpipe-audio run_record_and_transcribe.rs:191,234`, `windows_uia.rs:71` | unchanged (same messages at 0x52555e6, 0x49aa4e0, 0x512c950, 0x512c904) | none (owned by vision/audio) |

---

## 2. Delta items

### HLI-001: `recording_coverage.rs` accumulator and classifier (new, M)

**Interop identifiers:**
- Module `screenpipe_engine::recording_coverage` (path `crates\screenpipe-engine\src\recording_coverage.rs`, 0x6a13200 and 0x6ab6688).
- Type `CoverageSnapshot` (0x6a0bf30).
- Fields `recording_secs`, `active_stalled_secs`, `active_paused_secs`, `idle_secs`, `active_secs`, `coverage_ratio` (0x522b6ce..0x522b715).

**State.** One process-global accumulator: a lazily initialised `Mutex` around four `u64` second counters, indexed by bucket.
- It is never persisted.
- It is never reset; only the record and snapshot paths touch it (VA 0x142605A90, 0x142605830).
- So the window is **process lifetime**. It is neither rolling nor per day.

| Bucket index | Meaning | Snapshot field |
|---|---|---|
| 0 | user active and vision recording is landing | `recording_secs` |
| 1 | user active but vision stalled | `active_stalled_secs` |
| 2 | user active but recording intentionally paused | `active_paused_secs` |
| 3 | user idle (or activity unknown) | `idle_secs` |

**Ticker task** (spawn helper VA 0x142605BE0; task VA 0x1429F3460):
1. It is spawned once at HTTP-server start, in `server.rs` just before the server's other periodic spawns (target Locations `server.rs:636/651/706`). The helper is called from VA 0x14109EC7F.
2. It receives the vision `Arc<PipelineMetrics>` and the current `Instant`.
3. It is unconditional, even when vision is disabled.
4. It creates `tokio::time::interval(5 s)`, default missed-tick behaviour (`recording_coverage.rs:203:28`), and remembers the `Instant` of the previous tick.
5. Each tick:
   1. Compute whole seconds elapsed since the previous tick (`Instant::elapsed().as_secs()`, truncated). Then move the previous-tick marker to now.
   2. If elapsed is **0, or greater than 15**, do nothing for this tick. Elapsed is accepted only in the range 1..=15.
      - This drops time across sleep/hibernate, debugger stops and runtime stalls.
      - It also drops the zero-length burst ticks that tokio fires after a stall.
   3. Otherwise classify "now" (below) and add the elapsed seconds to that bucket.
6. If the mutex is poisoned, the add is skipped silently.

**Classifier** (VA 0x142605660). Input: the PipelineMetrics handle and `now` as Unix seconds (SystemTime; 0 if before the epoch). Rules, first match wins:
1. **Idle (3).**
   - The UI recorder's last flushed-event time is unset (0), or is not a valid timestamp.
   - Or `now − last_event` (saturating) is **greater than 300 s**.
   - The source is base `UI_RECORDER_LAST_EVENT_UNIX`, read through `ui_recorder_status_snapshot().last_event_at`.
2. **Active-paused (2).** Either `schedule_monitor::schedule_paused()` (global at bss 0x14A716189) or `drm_detector::drm_content_paused()` (bss 0x14A716160) is true.
3. **Recording (0) or active-stalled (1)**, from vision `last_db_write_ts` (Unix secs). Base advances this field on every frame DB write and on every dedup skip (`metrics.rs:180-215`).
   - If non-zero: age = `now − ts` (saturating). Age **≤ 120 s** → recording; age ≥ 121 s → active-stalled.
   - If zero (nothing written yet): pipeline uptime (`started_at.elapsed()` as f64 seconds) **< 120.0** → recording (startup grace); otherwise active-stalled.

The constants are observed: 300 (`cmp 0x12C`), 120 (`cmp 0x79`, ≥121), 120.0 (f64 at VA 0x1452417F0) and 15 (the elapsed window).

**Snapshot** (VA 0x142605830):
- `recording_secs = b0`, `active_stalled_secs = b1`, `active_paused_secs = b2`, `idle_secs = b3`.
- `active_secs = b0 + b1 + b2`.
- `coverage_ratio = b0 / active_secs` as f64, or `0.0` when `active_secs == 0`.
- A poisoned mutex yields all zeros.

**Inputs the target does *not* use for coverage:**
- Screen lock, system suspend, power profile `capture_paused`, battery, `disableVision`, audio state, and `GetLastInputInfo`.
- Locked or suspended periods still reach "idle": no UI events arrive, so after 300 s the time counts as idle.
- Sleep gaps are discarded by the 1..=15 s window.
- A user with the UI recorder disabled accrues only `idle_secs`, so `coverage_ratio` stays 0.
- A user with vision disabled but the UI recorder enabled accrues `active_stalled_secs` after 120 s.
- Freenpipe should reproduce this behaviour for parity. Interpreting `coverage_ratio` is the reader's job.

**Privacy.** The counters and ratio only. No app names, window titles, timestamps or content.

**Evidence.** All observed:
- Strings 0x6a0bf30..0x6a0c03c, 0x522b693.
- Locations 0x6a13260 (`:202:5`) and 0x6ab66e8 (`:203:28`).
- Code VAs above.
- Writer of the UI timestamp: VA 0x14259FB90, called after the UI-event batch flush and its flush-count log (VA 0x141A2CD0D).
- DRM flag writers: VA 0x1427B4BEE, next to the logs for protected content appearing and losing focus. Schedule flag writers: VA 0x1429EF7D5, next to the logs for leaving and entering the configured work hours.

**Base files to touch:**
- new `crates/screenpipe-engine/src/recording_coverage.rs`, with the Freenpipe file header;
- `crates/screenpipe-engine/src/lib.rs` (`pub mod recording_coverage;`);
- `crates/screenpipe-engine/src/server.rs`, to spawn next to the existing vision metrics task (base `server.rs:420-452`) and give it `self.vision_metrics.clone()`;
- optionally a cheap accessor in `ui_recorder.rs` (e.g. `last_ui_event_unix()`), to avoid building the whole status struct every 5 s.

**Implementation notes:**
- Use a plain `std::sync::Mutex<[u64; 4]>` in a `LazyLock`. Contention is negligible: one writer every 5 s, readers on `/health`.
- Keep the 1..=15 s acceptance window and `as_secs()` truncation for parity. Truncation slightly under-counts sub-second remainders.
  - A Freenpipe improvement could carry the remainder. If you do, document it as a deviation; it does not change the field set.
- `Instant` on Windows may or may not advance during suspend. The 15 s cap makes the result independent of that.
- Unit-test the classifier as a pure function. Inputs: last-UI-event secs, the two pause flags, last-write secs, uptime, now. Cover the boundaries 300/301, 120/121 and the 120.0 grace.

---

### HLI-002: `CoverageSnapshot` exposed on `/health` (changed, S)

**Interop.**
- `HealthCheckResponse.recording_coverage` is an object with the fields serialised in this order (two serializer instances, VA 0x140EEB458 and 0x140EEB63F): `recording_secs` (u64), `active_stalled_secs` (u64), `active_paused_secs` (u64), `idle_secs` (u64), `active_secs` (u64), `coverage_ratio` (f64).
- In the health response it sits between `ui_recorder` and `pool_stats` (0x5256cd6, 0x6a22da8).
- Correction to api-033: api.md listed the fields as `recording_secs, active_secs, idle_secs, …`. The real declaration and serialisation order is the one given here. JSON consumers are order-insensitive, but the OpenAPI schema should follow this order.

**Behaviour.**
- The health handler always calls the snapshot (VA 0x14108F2EE, inside the health builder that also emits the `healthy` status and its everything-OK message).
- It always fills the field, so emit it unconditionally (no `skip_serializing_if`).
- It is cheap enough to compute inside the api-033 health budget. If the budget is exceeded, the cached snapshot path (api-033) serves the stale value.
- Schema doc: a point-in-time snapshot of recording coverage; counts only.

**Evidence.** Observed: 0x5256cd6+930, 0x6a22da8+572, 0x6a0bf30.

**Base files.** `crates/screenpipe-engine/src/routes/health.rs`. Add `pub recording_coverage: CoverageSnapshot` with `#[derive(Serialize, Deserialize, Clone, OaSchema)]` and a doc comment, and fill it in `health_check_inner`. The cached default should be all-zero.

**Notes.**
- The app reads `/health` over HTTP. Deserializers of older payloads must tolerate the field's absence, so use `#[serde(default)]` on the response struct field.
- No frontend consumer exists (REA ev_10465921…).

---

### HLI-003: coverage telemetry sinks (decloud, S)

**Observed:**
- **Engine.** The 60 s `vision_pipeline_health` PostHog event (`server.rs:652` Location, VA 0x1403B8099; key string 0x49ac1d0) now adds `recording_coverage_ratio`, `recording_secs`, `active_secs`, `recording_active_stalled_secs`, `recording_active_paused_secs`, `recording_idle_secs`. It also adds the `ocr_gate_*` keys owned by vision. These keys were decoded from code immediates in VA 0x1403B7E00..0x1403BA300.
- **App.** `src\analytics.rs` `app_still_running` (0x499ad40..0x499b6a0) flattens `/health` into `recording_coverage_ratio` (from `recording_coverage.coverage_ratio`), `recording_secs`, `recording_active_secs`, `recording_active_stalled_secs`, `recording_active_paused_secs` and `recording_idle_secs`. It does the same for write-queue fields (`recording_write_queue_degraded`, …) and the pipeline loss counters.

**Freenpipe.**
- Do not add these keys to any network event.
- Base still sends `vision_pipeline_health` and `app_still_running` to PostHog. cloud-010 removes them; this item just records that the coverage data has no other sink.
- Optional local substitute: a DEBUG log every 60 s, e.g. "recording coverage: ratio=…, recording=…s, stalled=…s, paused=…s, idle=…s".

**Base files.** `server.rs:420-452` (vision reporter); `apps/screenpipe-app-tauri/src-tauri/src/analytics.rs:380-447`. Both are handled under cloud-010.

---

### HLI-004: process-tree sample and extended system metrics (changed, M)

**Observed behaviour** (sample builder VA 0x143A15E70; monitor loop VA 0x1429EEA80..; log_status VA 0x14275F2A0):

**1. Process set.**
- Start from the engine's own PID (`std::process::id()`).
- Build a map from parent PID to child PIDs over every process `sysinfo` knows. Walk it breadth-first to collect **all descendants**. Base only took direct children (`resource_monitor.rs:368`).
- Sum over the own process plus descendants:
  - RSS bytes → `total_memory_gb`;
  - CPU % (f32 sum) → `total_cpu_percent`;
  - maximum virtual memory → `total_virtual_memory_gb` (NaN-safe max).
- GB conversion is `bytes × 2⁻³⁰` (constant 9.313225746154785e-10).

**2. System figures.** From `sysinfo::System` (crate unchanged, `sysinfo-0.29.0`):

| Field | Source |
|---|---|
| `system_total_memory_gb` | total memory |
| `available_memory_gb` | available memory |
| `used_swap_gb` | used swap (decoded immediates) |
| `total_swap_gb` | total swap (code immediates at 0x27b90a1) |
| `load_average_1m`, `load_average_5m`, `load_average_15m` | `System::load_average()`. On Windows sysinfo emulates this via PDH; the PDH imports are at 0xa6a9ab0. |
| `load_average_*_per_cpu` | the matching load divided by the CPU count (inferred: the count is the `cores=` value from HLI-009) |
| `memory_usage_percent` | `total_memory_gb / system_total_memory_gb × 100`, or 0 when total is 0 |

**3. `phys_footprint_gb`.**
- In the Windows build this is the same value as `total_memory_gb`: the builder stores the same register into both slots.
- On macOS the name implies `task_info` phys_footprint. That is outside the Windows scope; keep a cfg hook.

**4. Status line** (DEBUG, same level as base; template 0x6a670a0). In order, it prints:
runtime seconds, memory % with used and total GB plus available GB, swap used/total GB, footprint GB, virtual GB, CPU %, and load as 1m/5m/15m.

**5. Loop.** Each tick:
1. refresh CPU, processes and memory;
2. build the sample;
3. run `log_status` (status line plus breakdown logging, HLI-006);
4. append to the JSONL file (HLI-008);
5. run the leak-trend check (HLI-007).

The tick interval and PostHog cadence are inferred unchanged from base: 30 s and 60 s (`screenpipe-engine.rs:883-884`, `server_core.rs:299-300`).

**Evidence.**
- Observed: format pieces 0x6a670a0..0x6a6716b; Location sequence `resource_monitor.rs:539..628` (0x6a77f50..0x6a78200); key immediates `used_swap_gb`, `total_swap_gb`, `load_average_1m`/`_5m`; sample field stores at VA 0x143A17410..0x143A17483.
- Inferred: the per-CPU divisor and the 30 s tick.

**Base files.** `crates/screenpipe-engine/src/resource_monitor.rs`. Replace the `(f64, f64, f64, f32, f64, Duration)` tuple with a named `ResourceSample` struct carrying all of the fields above plus `breakdown: ProcessBreakdown`.

**Notes.**
- The descendant walk is cheap: one pass over `sys.processes()` per tick.
- Guard against PID reuse loops by keeping a visited set; the target uses a HashSet.

---

### HLI-005: process breakdown model (new, M)

**Interop serde shapes** (field order from the serializer at VA 0x1426CAA80..0x1426CAFB9):
- `ProcessBreakdown { groups: [ProcessGroupUsage], top_related_by_memory: [ProcessUsage], top_related_by_cpu: [ProcessUsage] }`
- `ProcessGroupUsage { group: String, process_count: u64, rss_gb: f64, cpu_percent: f32 }`
- `ProcessUsage { pid: u32, parent_pid: u32|null, parent_name: String, name: String, group: String, rss_mb: f64, cpu_percent: f32 }`

**Group labels** (0xa2d0870 pool, 0x6a67f0d):

| Long label (used in `group`) | Short key (used only for telemetry key names, HLI-010) |
|---|---|
| `screenpipe_app` | `app` |
| `screenpipe_app_child` | `app_child` |
| `screenpipe_mcp_child` | `mcp_child` |
| `screenpipe_mcp_external` | `mcp_external` |
| `screenpipe_other` | `other` |

**Classification**, for every process `sysinfo` reports (observed, VA 0x143A16700..0x143A16A70):
1. Haystack = the process name, plus (when the process has a command line) a space and the argv joined with spaces, all ASCII-lowercased.
2. PID equals own PID → `screenpipe_app`.
3. Else if the haystack contains `screenpipe-mcp`: → `screenpipe_mcp_child` if the PID is in the descendant set (HLI-004), otherwise `screenpipe_mcp_external`.
4. Else if the PID is a descendant → `screenpipe_app_child`. This covers bun/pipes, ffmpeg, the ACP agents and so on.
5. Else if the haystack contains `screenpipe` → `screenpipe_other`. That is another screenpipe/Freenpipe instance or a CLI.
6. Else the process is unrelated and skipped.

**Per-process record:**
- `rss_mb = rss_bytes × 2⁻³⁰ × 1024`.
- `cpu_percent` is sysinfo's per-process value, which can exceed 100 on multi-core machines.
- `parent_pid` comes from sysinfo. `parent_name` is the parent process's name, or the literal `unknown` when the parent is not found (0xa2d08f2).

**Groups.** Accumulate `process_count`, `rss_gb` and `cpu_percent` per label. Emit one entry per non-empty label. The order is not recovered; sorting by `rss_gb` descending is suggested.

**Top lists.**
- All classified processes, sorted descending by `rss_mb` (memory list) and by `cpu_percent` (CPU list).
- Each list is truncated to **12** entries (`cmp …,0Ch`, VA 0x143A17659 and 0x143A17717).

**Display form**, used in the logs:
- Groups are rendered as `<group>=<count>p/<rss_gb>GB/<cpu>%cpu` and joined with `, `. The template bytes are at 0xa2d0b04; the precision is inferred as 2 decimals for GB and 0 for CPU.
- With no related processes, it renders a fixed placeholder stating that no related processes were found (0xa2d08cd).
- The top lists are not part of the Display string.

**Evidence.** Observed: 0x6a3e638, 0xa2d0870, 0x6a67f0d, VA ranges above. The `group` and `name` field names are recovered from shared literals (string offsets 0x6a305af, 0x6a305ec).

**Base files.**
- New `crates/screenpipe-engine/src/resource_breakdown.rs`. If the `screenpipe-resource` crate is introduced for the CPU lane (vision.md:615, db.md:805), the module can live there instead (`src/process_tree.rs`).
- Then call it from `resource_monitor.rs`.

**Notes.**
- Command lines can contain secrets: API keys in args, tokens. They are used only for substring matching and are never stored or logged. Only `name` is serialised.
- Freenpipe naming: the target matches `screenpipe` / `screenpipe-mcp`. Freenpipe binaries keep the upstream binary names, so the same substrings work. Optionally also match `freenpipe` / `freenpipe-mcp`, mapped to the same labels.

---

### HLI-006: breakdown exposure, pressure predicate and env toggles (new, S)

**Pressure predicate** (VA 0x143A17880). It is true if any of these hold, checked in this order:
1. `screenpipe_mcp_child.process_count` **> 2**;
2. `mcp_child.process_count + mcp_external.process_count` **> 9**;
3. `mcp_child.rss_gb + mcp_external.rss_gb` **≥ 1.0**;
4. sum of `rss_gb` over all groups **≥ 8.0**;
5. sum of `cpu_percent` over all groups **≥ 250.0**.

It is false when there are no groups. The constants are observed: 2, 9, 1.0 (VA 0x144980F10), 8.0 (VA 0x146AE3240), 250.0f32 (VA 0x149F4D66C).

**Logging, every monitor tick** (log target `resource_monitor`, 0x6a67250):
- If pressure is true: WARN, the "process breakdown pressure:" prefix followed by the Display form (0x6a67857).
- Else, if env `SCREENPIPE_RESOURCE_BREAKDOWN` **or** `SAVE_RESOURCE_USAGE` is set to any valid-Unicode value: INFO, the "process breakdown:" prefix with the Display form (0x6a67800).
  - Read with `std::env::var`, so a non-Unicode value counts as unset.
  - Decoded from the `Result<String, VarError>` niche handling at VA 0x14275F8C0..0x14275FAE5.
- Otherwise, nothing is logged.

**Other exposure:**
- The full `ProcessBreakdown` is serialised into the JSONL file as `process_breakdown` (HLI-008).
- The target also flattens per-group `process_<short>_count`, `process_<short>_rss_gb` and `process_<short>_cpu_percent` into PostHog (`resource_monitor.rs:45/49`, 0x6a68031..0x6a68083). Freenpipe drops that (HLI-010).
- **Not** exposed on `/health` or in the UI (no frontend or health-serializer reference).

**Base files.** `resource_monitor.rs` (`log_status` / `log_status_local`, merged into one function). shell.md:811 already lists both env vars; this item defines their semantics.

**Notes.** Read the env vars once at `ResourceMonitor::new` and cache them. The target re-reads them every tick, which is harmless but unnecessary; the observable behaviour is the same.

---

### HLI-007: memory-leak trend detector (new, M)

**State.** A `Mutex<VecDeque<(t_secs: f64, mem_gb: f64)>>` and a one-shot "already reported" `AtomicBool` inside the monitor (VA 0x142760160).

**Per tick**, called with `runtime_secs` (monitor uptime as f64) and `total_memory_gb` from the HLI-004 tree-RSS sample:
1. If `runtime_secs` < **1800** (30 min warm-up): return None and do not record a sample.
2. Push `(runtime_secs, mem_gb)`. Then pop from the front while length > **4096**, or while the newest t minus the oldest t > **21600** s (6 h window).
3. If there are < **30** samples, or the span (last t − first t) < **10800** s (3 h): return None.
4. Least squares over the window:
   - `slope_gb_per_s = Sxy / Sxx`. Return None if `Sxx ≤ 0`.
   - `r2 = Sxy² / (Sxx·Syy)` when `Syy > f64::EPSILON`, else 0.
   - `slope_mb_per_h = slope × 1024 × 3600`.
   - `window_h = span / 3600`.
   - `start_gb` = mem of the oldest sample; `end_gb` = mem of the newest sample.
   - `fitted_growth_gb = window_h × slope_mb_per_h / 1024`.
5. Return `Some(trend)`. The target uses it for telemetry (`mem_slope_mb_per_h`, `mem_slope_r2`, `mem_trend_window_h`).
6. **Leak verdict** when all of the following hold:
   - `slope_mb_per_h ≥ 150`;
   - `r2 ≥ 0.8`;
   - `end_gb ≥ 1.5`;
   - `fitted_growth_gb ≥ 0.5`;
   - the one-shot flag was false. It is atomically swapped to true, so the verdict fires **at most once per process**.
7. On a verdict:
   - WARN on target `resource_monitor` (0x6a67499). The message gives the slope in MB/h over the window in hours, with r², start GB and end GB.
   - The target additionally sends a Sentry message with fingerprint `memory-leak-suspected` and attributes `leak.slope_mb_per_h`, `leak.r2`, `leak.window_h`, `leak.start_gb`, `leak.end_gb`, `leak.fitted_growth_gb` (0x6a9d530..0x6a9d5c0; builder VA 0x1428D2A30). Freenpipe drops the Sentry part (HLI-010).

**Constants.** All observed f64s at VA 0x146A64380..0x146A64398, 0x14513D870, 0x144980F18, 0x144980CD8: 1800, 21600, 10800, 3600, 1024, 150, 0.8, 1.5, 1/1024, 0.5, plus the counts 30 and 4096.

**Freenpipe local exposure.**
- WARN log only, in parity with the local part of the target.
- Optional: record the verdict as a field in the JSONL file (`leak_suspected`) and in api-033's `unhealthy_reasons`, as a Freenpipe-only extension. Not in target; mark it as a deviation if added.

**Base files.** `resource_monitor.rs`, with a new `MemoryTrend` helper. Unit-test it as a pure function over synthetic series: flat, slow growth below 150 MB/h, noisy growth with r² < 0.8, a true leak, and the < 3 h span case.

---

### HLI-008: resource usage JSONL file v2 (changed, S)

**Trigger:**
- Unchanged: env `SAVE_RESOURCE_USAGE` present at monitor creation.
- Filename `resource_usage_<YYYYmmdd_HHMMSS>.json` in the working directory (0x6a67dc8, 0x6a67dd6).
- The target still first writes `[]` into the file (the JSON-file initialisation failure log, 0x6a67e1b), as in base.

**Record.** One JSON object per line. Key order, observed (VA 0x1429EC930..0x1429ED800):
`timestamp` (RFC 3339 local), `runtime_seconds`, `total_memory_gb`, `system_total_memory_gb`, `memory_usage_percent`, `available_memory_gb`, `used_swap_gb`, `total_swap_gb`, `total_cpu_percent`, `total_virtual_memory_gb`, `load_average_1m`, `load_average_5m`, `load_average_15m`, `load_average_1m_per_cpu`, `load_average_5m_per_cpu`, `load_average_15m_per_cpu`, `phys_footprint_gb`, `process_breakdown` (object, HLI-005).

The packed literal list at 0x6ab6030 holds the keys unique to this function.

**Size cap.** Truncate when the file exceeds 10 MB. Inferred unchanged from base (`MAX_RESOURCE_LOG_BYTES`); the Location of the resource-log write-failure message moved to `:650`/`:826`.

**Base files.** `resource_monitor.rs::log_to_file`.

**Notes.** The leading `[]` followed by JSONL lines is a pre-existing base quirk; keep it for parity. Readers should skip a first line equal to `[]`.

---

### HLI-009: startup host line (changed, S)

Observed (0x6a67ed4; level INFO at VA 0x1427621CB). Base logged "hardware: cpu=… arch=… gpus=…" at DEBUG. The target logs once at monitor creation:
`host: os=<os name> <os version> kernel=<kernel> cpu=<brand> arch=<arch> cores=<logical cpu count> gpus=<list>`.

GPU detection is unchanged: PowerShell `Get-CimInstance Win32_VideoController`, with `wmic` as the fallback (0x6a66c58, 0x6a66d08).

**Base files.** `resource_monitor.rs::new`.

**Note.** This is local logging only; nothing about it is cloud.

---

### HLI-010: resource-monitor vendor sinks (decloud, S)

**Target, observed:**
- **PostHog.** The `resource_usage` event still uses the embedded key `phc_…` (0x6a78218) and `https://us.i.posthog.com/capture/`. It now carries:
  - `distinct_id`, `$lib`, `total_memory_gb`, `phys_footprint_gb`, `system_total_memory_gb`, `memory_usage_percent`, `available_memory_gb`, `used_swap_gb`, `total_swap_gb`, `total_cpu_percent`;
  - `load_average_{1m,5m,15m}` and their `_per_cpu` versions, `runtime_seconds`;
  - `os_name`, `os_version`, `kernel_version`, `cpu_count`, `cpu_brand`, `cpu_arch`, `gpu_count`, `gpu_names`, `release`;
  - `mem_slope_mb_per_h`, `mem_slope_r2`, `mem_trend_window_h`, `leak_suspected`;
  - per-group `process_<short>_{count,rss_gb,cpu_percent}`;
  - the telemetry-context IDs `SCREENPIPE_SUPPORT_ID`, `SCREENPIPE_CUSTOMER_ID`, `SCREENPIPE_DEPLOYMENT_ID`, `SCREENPIPE_EMBEDDER(_VERSION)`, `SCREENPIPE_ENTERPRISE_LICENSE_HASH`, `SCREENPIPE_ENTERPRISE_DEVICE_ID`.
  - Keys were decoded from VA 0x1427B9000..0x1427BD200.
- **Sentry.** The leak message (HLI-007).

**Freenpipe:**
- Remove `send_to_posthog`, `posthog_client`, `posthog_enabled`, `distinct_id` and the hard-coded key, which is still present in base `resource_monitor.rs:335`.
- `start_monitoring` keeps a single interval.
- No Sentry calls.
- All metrics remain local: logs and the JSONL file.
- Cross-reference: cloud-010 (PostHog) and cloud-011 (Sentry).

**Base files.**
- `resource_monitor.rs`.
- Callers: `crates/screenpipe-engine/src/bin/screenpipe-engine.rs:883-884` and `apps/screenpipe-app-tauri/src-tauri/src/server_core.rs:299-300`. Drop the `telemetry_enabled` argument, or ignore it.

---

### HLI-011: Windows native suspend/resume notification and "system suspended" flag (changed, M)

**Observed** (strings 0x6a03d70..0x6a04193; tracing callsites `sleep_monitor.rs:121,703,726,729,760,762`; code VA 0x1425A0E30 start, 0x1425B0D80 thread body, 0x1425A1140 callback):

**1. Start.**
- `start_sleep_monitor()` logs an INFO line announcing a combined suspend/resume and screen-lock monitor. This replaces the base INFO line, which announced a lock monitor based on `OpenInputDesktop` polling.
- It then spawns one OS thread. The 5 s poll interval is unchanged.

**2. Registration**, first thing on that thread:
- Heap-allocate a `DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS { Callback, Context = null }` and deliberately leak it, so it lives for the process lifetime.
- Call `PowerRegisterSuspendResumeNotification(DEVICE_NOTIFY_CALLBACK (= 2), &params, &mut HPOWERNOTIFY)` (powrprof.dll; import 0xa6a99c8).
- Return `ERROR_SUCCESS` → an INFO line confirming the registration. Otherwise → a WARN line reporting the registration failure with the returned `<code>` (0x6a04128).
- The registration is never unregistered.

**3. Callback**, signature `ULONG CALLBACK (PVOID ctx, ULONG type, PVOID setting)`, always returns 0:

| Power broadcast | `type` | Effect |
|---|---|---|
| `PBT_APMSUSPEND` | 4 | set suspended = true |
| `PBT_APMSTANDBY` | 5 | set suspended = true |
| `PBT_APMRESUMECRITICAL` | 6 | set suspended = false |
| `PBT_APMRESUMESUSPEND` | 7 | set suspended = false |
| `PBT_APMRESUMESTANDBY` | 8 | set suspended = false |
| `PBT_APMRESUMEAUTOMATIC` | 18 | set suspended = false |
| anything else | | ignored |

- The flag is a new process-global `AtomicBool` (bss 0x14A7160C0). It sits beside the existing `RECENTLY_WOKE` (0x14A7160B0), `RECENTLY_WOKE_SEQ` (0x14A7160B8) and `SCREEN_IS_LOCKED` (0x14A7160B1).
- The callback does no logging, no DB pause and no stream invalidation.
- Masks observed: 0x401F bitmask on `type−4`; lookup table at VA 0x146A0BBC8 = `[1,1,0,0,…]`.

**4. Poll loop**, every 5 s:
1. Clock-gap check, as in base (`elapsed > poll + 15 s`). On a gap: **clear the suspended flag** (safety net for a missed resume) and run the base `mark_recently_woke("windows")`:
   - set `RECENTLY_WOKE`;
   - bump the sequence;
   - an INFO line reporting a detected wake, naming the platform;
   - `permission_monitor::notify_wake()`;
   - start a 30 s clear thread.
2. `OpenInputDesktop(0, false, 0)`: success → close it and treat as unlocked; failure → locked.
3. Swap `SCREEN_IS_LOCKED` and mirror it into `screenpipe_config::set_screen_locked`.
4. On a transition, log an INFO line saying the screen became locked or unlocked, citing whether `OpenInputDesktop` was available.
   - Base used the same messages and logic.

**5. Fallback.**
- If registration fails, behaviour degrades to base: wake via clock gap, lock via polling.
- The suspended flag then stays false, so compaction falls back to the lock/power reasons only.
- There is no message-window (`WM_POWERBROADCAST`) or `WTSRegisterSessionNotification` path: neither import is present.

**6. New accessor.** Implied by readers at VA 0x1427C86F3 / 0x1429FBDAC: a public `system_suspended() -> bool` in `sleep_monitor`. It is read only by snapshot compaction (HLI-012). Vision capture, audio, coverage and health do not read it.

**What wake and lock feed (consumer matrix; only the bold rows are new):**

| Signal | Consumer | Behaviour | Owner |
|---|---|---|---|
| `SCREEN_IS_LOCKED` | vision capture loop | pause state, release stream, skip capture; startup skip (base `event_driven_capture.rs:684/896/1989`) | vision (vision.md:180) |
| `SCREEN_IS_LOCKED` via `screenpipe_config` | audio recorder | pause while locked unless record-while-locked; rebuild stream after unlock (base `run_record_and_transcribe.rs:191/234`); `/health` `audio_status=screen_locked` | audio / api-033 |
| `SCREEN_IS_LOCKED` | Windows UIA a11y | pause while locked (base `windows_uia.rs:71`) | vision |
| **`SCREEN_IS_LOCKED`** | **meeting detectors** | **skip scan iterations** (HLI-013) | meetings |
| **system suspended, `SCREEN_IS_LOCKED`** | **snapshot compaction** | **defer / pause** (HLI-012) | this item / vision-compaction-encoder |
| `RECENTLY_WOKE` | permission monitor, HD recorder | suppress stale permission flips; (base) | unchanged |
| UI events (not lock) | recording coverage | idle after 300 s without input (HLI-001) | this dimension |

**Base files:**
- `crates/screenpipe-engine/src/sleep_monitor.rs` (Windows `start_sleep_monitor`, new static and accessor, gap-path clear).
- `crates/screenpipe-engine/Cargo.toml`: add `Win32_System_Power` to the `windows` 0.58 features. shell-007 keep-awake needs the same feature for `SetThreadExecutionState`.

**Implementation notes:**
- The callback runs on a system thread pool. Keep it to an atomic store; no allocation, logging or locks.
- Use `windows::Win32::System::Power::{PowerRegisterSuspendResumeNotification, DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, HPOWERNOTIFY}` and `windows::Win32::UI::WindowsAndMessaging::DEVICE_NOTIFY_CALLBACK`, or the raw value 2.
- Store the `HPOWERNOTIFY` in a static so it is never dropped.
- Keep the macOS and Linux paths compiling. On those platforms `system_suspended()` returns false (macOS could set it from `will_sleep`/`did_wake`; optional).
- Tests: a callback-dispatch unit test (call the extern fn directly with each `type` and assert the flag); an assertion that the gap path clears it.

---

### HLI-012: snapshot compaction deferral reasons (changed, S)

**Observed.** The reason strings (0x6ab6dd8) form an enum. Its order is the priority, and index 6 means "run":

| Index | Reason | Fires when |
|---|---|---|
| 0 | system suspended | suspended flag (HLI-011) |
| 1 | screen locked | `SCREEN_IS_LOCKED` |
| 2 | on battery | power state not on AC (base already skips this; mapping of the byte fields to `on_ac` / `os_low_power` / `capture_paused` / `PowerMode` is inferred from base struct semantics) |
| 3 | OS low-power mode | `PowerState.os_low_power` |
| 4 | capture power-paused | active power profile `capture_paused` |
| 5 | battery-saver mode | `PowerMode::BatterySaver` |
| 6 | (none) | proceed |

Decision function at VA 0x1427C8600..0x1427C8799; reads at VA 0x1427C86F3 / 0x1427C8700.

**Behaviour:**
- Before each cycle the worker computes the reason and logs (DEBUG) "snapshot compaction: skipping <reason>" (0x6ab6ef0). It then waits one poll interval, keeping the base poll and shutdown select.
- During a cycle, between chunks, it re-checks **only** suspended/locked (VA 0x1429FBDAC). If either is set, it logs "snapshot compaction: paused <reason>" (0x6ab6f7c), stops feeding ffmpeg (vision-compaction-encoder pause handling) and keeps the source JPEGs.
- The thermal throttle (its log reports the throttle and the thermal state, 0x6ab6f48) is unchanged from base.

**Base files.** `crates/screenpipe-engine/src/snapshot_compaction.rs` (loop at `:57-110`), `power/monitor.rs` (`PowerState`), `power/profile.rs` (`PowerMode`, `capture_paused`).

**Note.** vision.md `vision-compaction-encoder` mentions pause handling only generically. This item fixes the reason set and its order.

---

### HLI-013: meeting detectors skip while the screen is locked (changed, S; cross-dimension, owner = meetings)

**Observed:**
- The meeting-v2 scan loop (VA 0x1429DBE3D, next to its manual-meeting skip log at string offset 0x6ab54c6) reads `SCREEN_IS_LOCKED` and jumps back to the wait when set.
- The audio-process meeting detector loop (VA 0x1429C655B, next to its manual-meeting skip log at string offset 0x6ab4002) does the same.
- Neither emits a dedicated log line.
- Base meeting loops do not consult the lock flag (`grep screen_is_locked` in `meeting_*` = 0).

**Behaviour.** While locked, skip detection entirely: no new meeting starts and no state changes. An already-running meeting is ended by the detectors' normal timeouts. Inferred: the skip happens before any scan work.

**Base files.** `crates/screenpipe-engine/src/meeting_watcher.rs`, `meeting_detector.rs`, or their v2 equivalents per meetings_memory.md. Listed here so the lock consumer matrix is complete.

---

## 3. Numeric defaults (summary)

| Item | Constant | Value | Confidence |
|---|---|---|---|
| HLI-001 | coverage tick | 5 s | observed |
| HLI-001 | accepted elapsed per tick | 1..=15 s (else dropped) | observed |
| HLI-001 | idle threshold (since last UI-event flush) | > 300 s | observed |
| HLI-001 | stall threshold (since `last_db_write_ts`) | ≥ 121 s stalled / ≤ 120 s recording | observed |
| HLI-001 | startup grace when no write yet | pipeline uptime < 120.0 s | observed |
| HLI-004 | monitor tick / PostHog tick | 30 s / 60 s | inferred (base) |
| HLI-005 | top-list length | 12 | observed |
| HLI-006 | pressure: mcp_child count | > 2 | observed |
| HLI-006 | pressure: mcp child+external count | > 9 | observed |
| HLI-006 | pressure: mcp child+external RSS | ≥ 1.0 GB | observed |
| HLI-006 | pressure: total RSS / total CPU | ≥ 8.0 GB / ≥ 250 % | observed |
| HLI-007 | warm-up / window / min span | 1800 s / 21600 s / 10800 s | observed |
| HLI-007 | min / max samples | 30 / 4096 | observed |
| HLI-007 | slope / r² / end / fitted growth | ≥ 150 MB/h / ≥ 0.8 / ≥ 1.5 GB / ≥ 0.5 GB | observed |
| HLI-007 | report count | once per process | observed |
| HLI-008 | JSONL truncate | 10 MB | inferred (base) |
| HLI-011 | poll / wake gap / woke flag | 5 s / > poll + 15 s / 30 s | observed (gap, flag); poll value from the spawn argument |
| HLI-011 | registration flags | `DEVICE_NOTIFY_CALLBACK` = 2 | observed |

---

## 4. Open questions

1. **Coverage while the UI recorder is off.**
   - The target classifies everything as idle, so `coverage_ratio` stays 0.
   - Should Freenpipe fall back to `GetLastInputInfo` when `ui_recorder.running == false`? That would deviate from the target.
   - Recommendation: keep parity. Expose `ui_recorder` alongside so readers can interpret the result.
2. **Coverage with vision disabled.** Active time counts as `active_stalled_secs` after 120 s. Parity says keep it. Alternatively, Freenpipe could skip spawning the ticker when `disableVision` is set; that is a deviation.
3. **Order of `groups`** in `ProcessBreakdown`: not recovered (VA 0x143A19180 builds it from a hash map). Sort by `rss_gb` descending for deterministic output.
4. **Exact float precisions** in the status, leak and breakdown log templates: inline fmt-spec bytes were not fully decoded. This only affects log text.
5. **`phys_footprint_gb` on macOS**: whether it uses `task_info` (`TASK_VM_INFO.phys_footprint`). Windows equals the RSS total. Out of scope for the Windows-first build.
6. **Health-dimension ownership of `health-startup.json`** (audio-M06 / shell-M01) stays with shell. Nothing in this dimension writes that file.
