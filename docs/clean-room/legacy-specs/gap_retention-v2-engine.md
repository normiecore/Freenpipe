# Freenpipe RE spec: engine retention v2 (`lean` mode, heavy-text strip, hybrid-aware deletion)

Dimension key: `gap_retention-v2-engine`. Delta ids: `gap_retention-v2-engine-NNN`.

Target: screenpipe v2.7.104 Windows build (`<screenpipe-install>/screenpipe-app.exe`). Static evidence only; the exe was never run.
Base: Freenpipe @ upstream `892199f`, mainly `crates/screenpipe-engine/src/retention.rs` (472 lines) and `crates/screenpipe-db/src/db.rs`.

Clean-room note: behaviour below is paraphrased. Interop identifiers are quoted exactly: routes, JSON fields, enum strings, table and column names, log-event wording where it serves as a test oracle, and the SQL statement shapes that define which rows are touched. No decompiled code, JS bodies or UI prose were copied. UI copy is summarised only.

Scope: the engine side only. The settings UI (`localRetentionMode` radio `off|media|lean|all`, confirm dialog, `/data/compact` button) is already specified in `frontend.md` item frontend-039. The hybrid storage format itself is db-021/db-022. The `ocr_text` retirement is db-005, `display_layout` is db-007, the semantic tables are db-008, the activity ledger is db-013 and `/data/compact` is db-025/api-030. This spec only describes how retention and deletion interact with them.

---

## 0. Evidence and method

* String offsets are file offsets in the exe, as in `<re-workspace>/app_strings.tsv`. Raw-byte reads used `tools\verify_raw.py`, `tools\vapi_hexdump.py` and three helpers written for this dimension:
  * `tools\rv2_rawctx.py`: raw regex search over exe bytes. It catches multi-line SQL and short literals that the TSV splits.
  * `tools\rv2_va.py`: PE VA↔file-offset mapping, plus decoding of panic `Location` records (file, line, col) near an offset. It tells which source file and line a literal belongs to.
  * `tools\rv2_leascan.py`: finds RIP-relative `LEA`s that reference a literal, then lists compare-immediates near each one. It recovered the numeric bound 36500.
* Format strings in this build use Rust's compact `fmt` template encoding: a length byte, then the literal piece, then `0xC0` for each argument. So a message such as `retention: lean stripped elements=` is followed in the bytes by `frames_ax_json=` and `ui_events=` pieces, and the argument count can be read directly.
* Tracing callsites: `retention.rs` event records at `0x6a6d7b0..0x6a6ed88` give target line numbers 181, 198, 248, 308, 346, 364, 383, 397, 409, 419, 453, 455, 505, 529, 545, 569, 598, 615, 625, 656, 668, 673. The target file is about 680 lines; the base file is 472.
* Rodata string order within one function follows source order closely. The loop's six messages appear in the same order as the six loop callsite lines (346→419). That ordering is how log lines were mapped to lines (§2).
* REA: parent JS-application evidence `ev_57f23857f31e9207a9f71e1d612412e78f9c5dc0d29af8c68f97372786b6216d`. The settings-module trace (`module` seed `settings/page-d9c69007346aaf6a.js`) is `ev_f95e14984a9317611427c97745e4a4d093b2ad959845fe7120198b94f4fdd526`. Literal seeds `/retention/configure` (`ev_b031057f…f98f353`), `/data/storage-preview?older_than_days=` (`ev_1d271b89…27ea9`) and `retention-mode-lean` (`ev_f16ffde4…a1d31`) returned no-match because the graph is partial. Those strings were read directly from `frontend\_next\static\chunks\app\(main)\settings\page-d9c69007346aaf6a.js` at the character offsets cited below.
* The native REA session was not used; it is owned by the coordinator.

---

## 1. Base vs target inventory

| Area | Base (892199f) | Target (v2.7.104) | Delta |
|---|---|---|---|
| `RetentionMode` enum | `Media` (default), `All`; serde/clap lowercase | `Media`, `Lean`, `All` (Debug names `MediaLeanAll` `0x6abc4d8`; serde variants `media`,`lean`,`all` `0x51544a0`) | -001 |
| `off` | not an engine concept; app sends `enabled:false` | still app-only: `off` = `localRetentionEnabled=false` (frontend `page-d9c6…js` @542041) | -001, -010 |
| `/retention/configure` validation | `retention_days >= 1` only | `1 <= retention_days <= 36500` (`0x515437d`, `0x515439e`; code `cmp ecx,0x8E94` at file `0x9f86cd`) | -005 |
| Configure log on re-enable | `retention: re-enabled with …` | `retention: started with <N>d (<mode>)` (`0x5154464`); "re-enabled" string gone | -005 |
| `/retention/status` shape | `enabled, retention_days, mode, last_cleanup, last_error, total_deleted` | same fields (`0x51543f0`, `0x51544e0`); `mode` may now be `lean` | -001 |
| Loop cadence | 30 s initial sleep, 300 s interval, `run_now` notify, 1 h batches, 500 ms yield | 30 s (`mov edx,30` @`0x2a20829`), 300 s (`mov edx,300` @`0x2a209f0`), 1 h batch (`3600` @`0x2a21bc4`); yield not re-verified | unchanged |
| Cutoff guard | `now - Duration::days(d)` (can panic on absurd d) | checked arithmetic: `days*86400` (`imul …,0x15180` @`0x2a21107`), on overflow warn `retention: retention_days=<N> out of range, skipping cleanup cycle` (`0x6ab9186`) | -004 |
| Oldest timestamp | read inside `do_local_cleanup`; log `cleaning up data before <cutoff> …` | read in the loop; log `retention: cleaning up data from <oldest> to <cutoff> (<N>d retention, mode=<M>)` (`0x6ab91f6`); `failed to get oldest timestamp` moved to the loop | -004 |
| Media cycle | `evict_media_in_range` per batch + unlink | same (`0x6ab8dc8`, `0x6ab8e37`) | unchanged |
| Lean cycle | absent | per batch: media eviction, then `strip_heavy_text_in_range` (`0x6ab8e6c..0x6ab8eea`) | -002, -003 |
| All cycle | `delete_time_range_batch(..., true)` + unlink; log includes `ocr=` | same but log `frames= audio= ui_events=` (no `ocr=`) (`0x6ab8f23`) | -007 |
| File unlink | per-mode loops; warn on any error | one helper: NotFound → debug `retention: file already gone, nothing to evict: <path>` (`0x6ab90ef`); other → warn `retention: failed to evict file <path>: <err>` (`0x6ab90c8`); "failed to delete file" string gone | -004 |
| Post-loop | `All` only: `cleanup_orphaned_chunks` + `PRAGMA incremental_vacuum(1000)` | same statements; message now says it only reclaims pages under `auto_vacuum=incremental` (`0x6ab900b`); `/data/compact` doc names lean/all as reasons to VACUUM (`0x6a6ca48`) | -004 |
| `strip_heavy_text_in_range` | absent (0 base hits) | `UPDATE frames SET accessibility_tree_json = NULL, text_json = NULL …` + elements delete + ui_events delete; debug `strip_heavy_text_in_range committed: elements=, frames_stripped=, ui_events=` (`0x6aac088..0x6aac1f0`) | -003 |
| Chunk NULL-safety | `NOT IN (SELECT DISTINCT video_chunk_id FROM frames …)` without `IS NOT NULL` in evict UPDATEs, batch-delete SELECTs, orphan cleanup, `delete_by_machine_id` | every inner subquery adds `AND video_chunk_id IS NOT NULL` / `AND audio_chunk_id IS NOT NULL` (`0x6aaa600..0x6aab450`, `0x5140400..`) | -006 |
| `delete_time_range*` tables | ocr_text, elements (+anchor re-home), frames, audio_transcriptions, ui_events | no ocr_text; same others; + `display_layout` keep-newest-in-range (`0x6aabd6c`, `0x5140d13`) | -007 |
| `DeleteTimeRangeResponse` | includes `ocr_deleted` | `frames_deleted, audio_transcriptions_deleted, audio_chunks_deleted, video_chunks_deleted, accessibility_deleted, ui_events_deleted` (+file counters); `ocr_deleted` absent everywhere (`0x5151f10`; raw search 0 hits) | -007 |
| Hybrid (parquet) storage | absent | sealed-payload "retention" routine in `storage/sealing.rs`: batches of 128 frames with `payload_detail_present=1`, coordinated payload replacement, retry on `retention generation changed; retry` / `retention frame disappeared; retry` (`0x6ab2350`, `0x6ab2490`) | -008 |
| Semantic tables | absent | GC statements for unreferenced `semantic_runs` then `semantic_items` (`0x5145407`, `0x5145510`; `semantic.rs:1164`); migration comment says actor aliases are meant to outlive the retention of semantic items (`0xa2a9eea`) | -009 |
| Activity ledger | absent | AFTER DELETE triggers on `frames`, `ui_events`, `audio_transcriptions` (db-013) cascade automatically, so nothing extra is needed in retention | (db-013) |
| outputs / feedback / memories / meetings / starred_sessions | n/a | no time-range delete statement exists for any of them; retention does not touch them (§5) | none |
| Starred-session protection | n/a | none: no evict, strip, delete or retention SQL or message references `starred_sessions` (§5) | none |
| `/data/storage-preview` | media-only file estimate (`file_count`,`bytes`) | unchanged; route doc identical (`0x6a6c95c`); UI calls it with `older_than_days` only, no mode (page-d9c6…js @541416); the lean confirm dialog shows the media-only figure | none (§6) |
| `/data/evict-media` | evict + unlink + `bytes_freed` | unchanged field list (`0x6a63968`, `0x5151df0`); no strip | none (§6) |
| App auto-start | sends mode only if `media`/`all`, else `media` | accepts `all`/`lean`/`media` (`0x52ac0c0`); fresh-store guard leaves retention off (`0x52ad1b7`) | -010 |
| CLI `--retention-mode` | `media`/`all` | no CLI help in the target app (engine embedded); Freenpipe CLI must accept `lean` | -011 |

---

## 2. Target `retention.rs` reconstruction (line map, inferred from callsites + string order)

| Target line | Level (inferred) | Message (exact interop wording unless marked paraphrased) | Where |
|---|---|---|---|
| 181 | info | `retention: disabled` | configure handler, enabled→disabled |
| 198 | info | `retention: started with <N>d (<mode:?>)` | configure handler, (re)spawn loop |
| 248 | info | `retention: initialized with <N>d retention` | configure handler, first init |
| 308 | info | `retention: manual run triggered` | `/retention/run` |
| 346 | info | `retention: manual run triggered, starting now` | loop, notify branch |
| 364 | warn | `retention: retention_days=<N> out of range, skipping cleanup cycle` | loop, checked-cutoff failure |
| 383 | warn | `retention: failed to get oldest timestamp: <err>` | loop |
| 397 | info | `retention: cleaning up data from <oldest> to <cutoff> (<N>d retention, mode=<M:?>)` | loop |
| 409 | info | `retention: deleted <n> records` | loop, only when n>0 |
| 419 | warn | `retention: cleanup error: <err>` | loop |
| 453 / 455 | debug / warn | `retention: file already gone, nothing to evict: <path>` / `retention: failed to evict file <path>: <err>` | shared unlink helper |
| 505 | info | `retention: batch evicted video_chunks=… audio_chunks=… snapshots=… (files: video=… audio=… snapshots=…)` | Media branch |
| 529 | warn | `retention: batch evict failed for range <a> to <b>: <err>` | Media branch |
| 545 | warn | `retention: lean media evict failed for range <a> to <b>: <err>` | Lean branch, step 1 |
| 569 | info | `retention: lean stripped elements=<e> frames_ax_json=<f> ui_events=<u>` | Lean branch, step 2 ok |
| 598 | warn | `retention: lean text strip failed for range <a> to <b>: <err>` | Lean branch, step 2 err |
| 615 | info | `retention: batch deleted frames=… audio=… ui_events=… (video_files=… snapshot_files=… audio_files=…)` | All branch |
| 625 | warn | `retention: batch delete failed for range <a> to <b>: <err>` | All branch |
| 656 | warn | `retention: orphan chunk cleanup failed: <err>` | post-loop |
| 668 | info | `retention:` prefix, announces the incremental vacuum and adds a parenthetical that pages are reclaimed only when `auto_vacuum=incremental` (paraphrased) | post-loop |
| 673 | warn | `retention: incremental vacuum failed: <err>` | post-loop |

The Media/Lean/All order of 505→625 is inferred from rodata order and line gaps. The base order was All then Media. The order has no behavioural effect.

---

## 3. Delta items

### gap_retention-v2-engine-001 — `RetentionMode::Lean` (engine enum, API, status)

* **Kind:** changed. **Size:** S. **Confidence:** observed (enum, serde names), inferred (default).
* **Evidence:** Debug variant names `MediaLeanAll` at `0x6abc4d8`; serde variant table `medialeanall` at `0x51544a0`, followed by `last_cleanup last_error total_deleted RetentionConfigureRequest` (`0x51544e0`). `struct RetentionConfigureRequest with 3 elements` (`0x51551b0`) shows the request still has exactly `enabled, retention_days, mode` (`0x51543f0`). Base enum: `retention.rs:64-68`.
* **Behaviour:**
  * Add a variant between `Media` and `All`. Its wire string is `"lean"`, used in serde (lowercase), clap `ValueEnum` (lowercase) and the OpenAPI schema. Declaration order `Media, Lean, All` matches the target. Keep `Media` as `#[default]`; the frontend default `localRetentionMode:"media"` (`378-9f286ebe7c0f0713.js` @18435) and the base `RetentionConfig::default` agree.
  * `/retention/status` returns `mode:"lean"` when active; no new fields. `/retention/configure` echoes `mode` in its `{success, enabled, retention_days, mode}` body.
  * `off` is not an engine value. The engine rejects `"mode":"off"` through serde (axum JSON rejection), as the base does. The app expresses off as `{"enabled": false}` and never sends `mode` with it (page-d9c6…js @542041).
  * Update the `RetentionConfigureRequest.mode` doc comment and the `RetentionMode` doc to describe three modes.
* **Base files:** `crates/screenpipe-engine/src/retention.rs` (enum, docs). Every exhaustive `match` on `RetentionMode` must gain a `Lean` arm: `retention.rs::do_local_cleanup` and `crates/screenpipe-engine/src/bin/screenpipe-engine.rs:1588` (banner, see -011).

### gap_retention-v2-engine-002 — Lean cleanup cycle (engine side)

* **Kind:** new. **Size:** M. **Confidence:** observed (step order and messages), inferred (counter accounting, continue-on-error).
* **Evidence:** `0x6ab8e6c` `retention: lean media evict failed for range … to …: …`; `0x6ab8ea6` `retention: lean stripped elements=… frames_ax_json=… ui_events=…`; `0x6ab8eea` `retention: lean text strip failed for range … to …: …`; callsite lines 545/569/598. UI semantics (settings chunk @547738, @548677, @555108, paraphrased): lean does everything media does and additionally removes the per-element OCR and accessibility detail, while search over text, transcripts, the timeline and memories remain available. For lean, the UI labels the running counter as a count of cleaned items (@550917).
* **Behaviour per 1-hour batch `[batch_start, batch_end]`** (batches walk from the oldest timestamp to the cutoff, as in base):
  1. **Media step.** Call `evict_media_in_range(batch_start, batch_end)`, the same DB operation as Media mode. On success, unlink the returned video/audio/snapshot files with the shared helper (-004). On error, warn with the "lean media evict failed" message and **continue to step 2**. The two steps are independent; the message wording makes the strip a separate step and not a consequence of the evict.
  2. **Strip step.** Call `strip_heavy_text_in_range(batch_start, batch_end)` (-003). On success, if any of its three counters is non-zero, info-log `lean stripped elements=<elements_deleted> frames_ax_json=<frames_stripped> ui_events=<ui_events_deleted>`. On error, warn with the "lean text strip failed" message.
  3. Sleep/yield between batches as Media/All do (base: 500 ms).
* **Accounting (inferred):** add `video_chunks_evicted + audio_chunks_evicted + snapshots_evicted + elements + frames_stripped + ui_events` to the cycle total. That total feeds `total_deleted` in `/retention/status`, which the UI presents as a cleaned-item count for lean. Set the per-cycle "something happened" flag if any counter is non-zero.
* **Post-loop:** see -004. Lean never deletes frames or audio rows, so `cleanup_orphaned_chunks` has nothing to do for it.
* **Implementation notes:**
  * Freenpipe's on-disk shape must match the target so screenpipe-written DBs stay interoperable. Lean keeps every `frames` row (timestamps, app/window, `full_text`, `accessibility_text`, FTS), every `audio_transcriptions` row and every `memories` row. It drops only element geometry, AX JSON, OCR word-box JSON and UI-event rows.
  * Hot frame cache: base retention never touches `hot_frame_cache`, and no target evidence says the target does. The data routes do evict it. Optional for Freenpipe: evicting `hot_frame_cache` for the batch range avoids stale snapshot paths.
* **Base files:** `crates/screenpipe-engine/src/retention.rs` (`do_local_cleanup` new match arm).
* **Depends on:** -001, -003, -004.

### gap_retention-v2-engine-003 — DB operation `strip_heavy_text_in_range`

* **Kind:** new. **Size:** M. **Confidence:** observed (frames UPDATE, counter names, transaction messages), inferred-strong (elements and ui_events statements), inferred (anchor re-homing).
* **Evidence:**
  * `0x6aac088..0x6aac1f0` (db crate, maintenance CGU). A `Location` record at `0x6aac070` resolves to `screenpipe-db/src/db/maintenance.rs:725` just before the literal. The function therefore most likely lives in `db/maintenance.rs`; the coordinator hint said `accessibility.rs`, but the `accessibility.rs` records at `0x6aac3c0` follow a different query.
  * The literals are: `UPDATE frames SET accessibility_tree_json = NULL, text_json = NULL WHERE timestamp BETWEEN ?1 AND ?2 AND (accessibility_tree_json IS NOT NULL OR text_json IS NOT NULL)`; then the debug template `strip_heavy_text_in_range committed: elements=<a>, frames_stripped=<b>, ui_events=<c>`; then `failed to commit strip_heavy_text_in_range transaction: <err>`. So it is one write transaction with three counters.
  * The strip function needs an elements statement and a ui_events statement, but no other literal for either exists anywhere in the exe. Raw searches cover all `UPDATE ui_events SET` and `DELETE FROM ui_events` statements and all `= NULL … BETWEEN ?1 AND ?2` statements (tools `rv2_rawctx.py`). The only range-bounded ui_events statement is `DELETE FROM ui_events WHERE timestamp BETWEEN ?1 AND ?2`. The only range-bounded elements statement is `DELETE FROM elements WHERE frame_id IN (SELECT id FROM frames WHERE timestamp BETWEEN ?1 AND ?2)`. Both sit in the same CGU (`0x6aabb1c`, `0x6aabd41`) as `delete_time_range_batch`, and rustc dedupes identical literals within a CGU. Conclusion: lean deletes `elements` rows and `ui_events` rows in the range. It does not NULL individual ui_events columns. The coordinator's description ("strips some ui_events content") is not borne out by the strings.
* **Contract** (new `DatabaseManager` method, async, takes RFC 3339 bounds like its siblings):
  * Signature (paraphrase): `strip_heavy_text_in_range(start, end) -> Result<StripHeavyTextResult, sqlx::Error>`, where `StripHeavyTextResult { elements_deleted: u64, frames_stripped: u64, ui_events_deleted: u64 }`. Field names are Freenpipe-internal; only the log keys are observable.
  * Open one `begin_immediate_with_retry()` transaction (same lock discipline as `evict_media_in_range`). Bind `start.to_rfc3339()` / `end.to_rfc3339()` as `?1`/`?2`, matching the batch-delete conventions.
  * **(a) Anchor re-homing first (inferred, required for correctness).** `elements_ref_frame_id` lets a newer frame borrow an older frame's element rows. Run the same re-home sequence that `delete_time_range_batch` uses (`db.rs:6871-6916`), so frames after the batch that point at an anchor inside it keep their elements. That sequence is: find in-range anchors referenced from outside the range; pick `MIN(id)` among the outside referrers as the new owner; move the `elements.frame_id`; repoint `elements_ref_frame_id`; clear the new owner's own ref. The literals exist in the same CGU. Without this step, the first frames after every cutoff lose their element tree, which contradicts the UI's assurance that the timeline still works under lean.
  * **(b)** `DELETE FROM elements WHERE frame_id IN (SELECT id FROM frames WHERE timestamp BETWEEN ?1 AND ?2)`. `rows_affected` → `elements`. This deletes rows of every source (`accessibility` and `ocr`). The `elements_fts` delete trigger (base migration `20260301000000`) keeps the FTS index consistent.
  * **(c)** `UPDATE frames SET accessibility_tree_json = NULL, text_json = NULL WHERE timestamp BETWEEN ?1 AND ?2 AND (accessibility_tree_json IS NOT NULL OR text_json IS NOT NULL)`. `rows_affected` → `frames_stripped`. `full_text`, `accessibility_text`, `app_name`, `window_name`, `browser_url`, `snapshot_path`/`video_chunk_id`, redaction timestamps and FTS rows are untouched. The `IS NOT NULL` guard makes re-runs cheap and keeps the counter honest.
  * **(d)** `DELETE FROM ui_events WHERE timestamp BETWEEN ?1 AND ?2`. `rows_affected` → `ui_events`.
  * Commit. On commit failure, `error!` with the "failed to commit" wording, return Err, and touch nothing. On success, `debug!` the committed line.
  * Statement order (a)(b)(c)(d) is inferred. Only (a)-before-(b) matters.
* **Side effects to expect and test:**
  * `activity_ui_events_ad` (db-013) fires for every deleted ui_event. It removes `activity_actions`/`activity_evidence` rows with `source_type='ui_event'`. Intervals whose last evidence was a UI event, and unlocked tasks left empty, are then collected by the cascade triggers. So lean ages out activity-ledger detail that rests only on UI events. That matches target behaviour; document it, do not work around it.
  * The redaction worker backlog shrinks for the stripped rows; that is fine.
  * Endpoints that read `frames.text_json` (OCR word boxes, text positions) or `accessibility_tree_json`/`elements` return empty for stripped frames. This is intended: lean removes the on-screen geometry.
  * Hybrid storage: steps (b) and (d) operate on the `elements` virtual table and the bulk-archived `ui_events` via their triggers (db-022). Step (c) only affects resident or staged frame rows; sealed payloads are handled by -008.
* **Freenpipe sequencing:** depends on db-005 (`frames.text_json`). Until db-005 lands, Freenpipe still stores OCR boxes in `ocr_text.text_json`. In that interim, also NULL `ocr_text.text_json` for frames in the range, and drop that extra statement once db-005 retires `ocr_text`.
* **Base files:** `crates/screenpipe-db/src/db.rs` (new method next to `evict_media_in_range`, plus the result struct near `EvictMediaResult` at :96). If the target-like split `db/maintenance.rs` is adopted, put it there. Add a unit test that seeds frames, elements (including a cross-range `elements_ref_frame_id` anchor) and ui_events, then asserts the counters and that the out-of-range referrer keeps its elements.

### gap_retention-v2-engine-004 — Loop and cycle restructuring (guard, oldest-in-loop, unlink helper, post-loop policy)

* **Kind:** changed. **Size:** S. **Confidence:** observed (messages, the 86400 multiply-and-branch, constants), inferred (exact skip semantics, vacuum for lean).
* **Evidence:** `0x6ab9158..0x6ab925a` (loop messages in source order); `0x6ab90c8`/`0x6ab90ef` (unlink helper); `0x6ab8fe0`/`0x6ab900b` (post-loop). Code at file `0x2a21107`: `imul r8, rdi, 0x15180` (days×86400), a call to a checked time-arithmetic helper, then `test eax,eax; jz` to the block that references the "out of range" template (LEA at `0x2a211da`). Constants: 30 s at `0x2a20829`, 300 s at `0x2a209f0`, 3600 at `0x2a21bc4`.
* **Behaviour:**
  1. **Wake-up:** unchanged. 30 s after spawn, then every 300 s or on `run_now` (log `manual run triggered, starting now`). Read `(retention_days, mode)` from shared state; skip when no runtime exists or it is disabled.
  2. **Checked cutoff:** convert days to seconds and subtract from now using checked arithmetic. If that fails (out of chrono range), warn `retention: retention_days=<N> out of range, skipping cleanup cycle` and skip the cycle. The process must not panic. This is defence in depth: configure already caps days at 36500 (-005), but state can also come from other paths (CLI, future persisted config).
  3. **Oldest timestamp in the loop:** call `get_oldest_timestamp()`. The SQL is unchanged: `SELECT MIN(ts) FROM ( SELECT MIN(timestamp) AS ts FROM frames UNION ALL SELECT MIN(timestamp) AS ts FROM audio_transcriptions )` at `0x6aab1e6`. On error, warn `retention: failed to get oldest timestamp: <err>` and skip. When there is no data, skip quietly (inferred). Otherwise info-log `retention: cleaning up data from <oldest RFC3339> to <cutoff RFC3339> (<N>d retention, mode=<Mode:?>)` and pass `oldest` into the cleanup function. Base logged "before <cutoff>" and fetched oldest inside the cleanup.
  4. **Result handling:** unchanged. Ok(n) logs `retention: deleted <n> records` when n>0, then sets `last_cleanup=now`, clears `last_error` and adds to `total_deleted`. Err logs `retention: cleanup error: <err>` and stores `last_error`.
  5. **Shared unlink helper** for every mode. Remove the file; `NotFound` logs at debug `retention: file already gone, nothing to evict: <path>` and counts as success. Any other error warns `retention: failed to evict file <path>: <err>`. This replaces base's per-mode loops (base All used "failed to delete file", which is gone from the target). Tolerating NotFound matters because base builds could unlink files without marking the chunk evicted (see -006), leaving rows that point at missing files.
  6. **Post-loop maintenance**, only if the cycle changed anything:
     * `cleanup_orphaned_chunks()` for **All** (observed in base, string kept). Warn `retention: orphan chunk cleanup failed: <err>` on error.
     * `PRAGMA incremental_vacuum(1000)` with an info log (prefix `retention:`) announcing the incremental vacuum and noting that it reclaims pages only when `auto_vacuum=incremental`, and warn `retention: incremental vacuum failed: <err>`. Run it for **All**, and (inferred) also for **Lean**, since lean frees the most pages. The 12-line gap between the orphan-failure and vacuum callsites (656→668) suggests a separate condition. The pragma is harmless when `auto_vacuum` is not incremental; neither the base nor the target sets it, so in practice pages go to the freelist and are reused. To physically shrink, the user runs `POST /data/compact` (db-025), whose route doc explicitly cites retention lean/all (`0x6a6ca48`).
* **Base files:** `crates/screenpipe-engine/src/retention.rs` (`spawn_retention_loop`, `do_local_cleanup` signature takes `oldest`, new helper fn).

### gap_retention-v2-engine-005 — `/retention/configure` validation and logging

* **Kind:** changed. **Size:** S. **Confidence:** observed.
* **Evidence:** `0x515437d` template `retention_days must be at most <arg>` (one argument; the u32 `36500` is stored in rodata right before it at `0x5154378`, bytes `94 8e 00 00`); `0x515439e` `retention_days must be at least 1` (both exact strings, client-visible). Handler code at file `0x9f86c5`: `test ecx,ecx; jz <at-least-1 error>; cmp ecx,0x8E94; jbe <ok>`, else the at-most error. Logs `0x5154450` `retention: disabled`, `0x5154464` `retention: started with <N>d (<mode>)`, `0x515440a` `retention: initialized with <N>d retention`. Base: `retention.rs:120-128`.
* **Behaviour:**
  * Effective days = `request.retention_days.unwrap_or(14)`, as in base. If 0, return 400 `{"error":"retention_days must be at least 1"}` (exact string, client-visible). If greater than **36500** (about 100 years), return 400 `{"error":"retention_days must be at most 36500"}` (exact string, client-visible). The error text includes the bound as a number. Validate before taking the write lock.
  * `mode` is optional and validated by serde (`media|lean|all`). `enabled` defaults to true when absent (base).
  * Runtime transitions are unchanged in effect: disable aborts the task (`retention: disabled`); enable on a stopped runtime spawns a fresh loop and logs `retention: started with <N>d (<mode:?>)` (replaces "re-enabled with"); first-time init logs `initialized with …`. While running, changing days or mode updates shared state and takes effect next cycle.
  * Response bodies unchanged: `{success, enabled, retention_days, mode}`.
* **Implementation note:** keep the bound as a named constant such as `MAX_RETENTION_DAYS: u32 = 36500` and reuse it in the loop guard (-004) and the CLI arg (-011). The app UI only offers 7/14/30/60/90 (`r$` list at settings chunk @536856), so the bound is reachable only through the API.
* **Base files:** `crates/screenpipe-engine/src/retention.rs` (`retention_configure`).

### gap_retention-v2-engine-006 — NULL-safe chunk subqueries (evict, batch delete, orphan cleanup, device delete)

* **Kind:** changed (bug fix with data consequences). **Size:** S. **Confidence:** observed.
* **Evidence:** target evict SQL `0x6aaa600..0x6aab140`. Both `UPDATE video_chunks SET file_path = '', evicted_at = CURRENT_TIMESTAMP …` and the audio UPDATE now carry `AND video_chunk_id IS NOT NULL` / `AND audio_chunk_id IS NOT NULL` in both the `IN` and `NOT IN` subqueries. Batch delete selects `0x6aab450..0x6aab7c0`. Orphan cleanup `0x6aab340`: `DELETE FROM video_chunks WHERE id NOT IN (SELECT DISTINCT video_chunk_id FROM frames WHERE video_chunk_id IS NOT NULL)` and the audio twin. `delete_time_range`/`_local` `0x5140400..0x5140d13`; `delete_by_machine_id` `0x5140fd8..0x51411e2`.
* **Base defect:**
  * Base `evict_media_in_range` SELECTs (`db.rs:6634-6662`) filter NULLs, but its UPDATEs (`:6677-6703`) do not. Event-driven capture writes frames with `video_chunk_id NULL` until snapshot compaction, so the UPDATE's `NOT IN` evaluates to NULL and marks nothing. The files the SELECT returned are then unlinked anyway. Result: DB rows keep pointing at deleted mp4/wav files, and the next cycle tries to unlink them again.
  * Base `delete_time_range_batch` video/audio SELECTs (`:6829-6858`), `cleanup_orphaned_chunks` (`:6994-7006`), `delete_time_range`/`_local` orphan deletes (`:6346`, `:6363`, `:6545`, `:6562`) and `delete_by_machine_id` (`:7079`, `:7097`) have the same NULL trap. All mode then silently leaks mp4/wav files and orphan chunk rows.
* **Behaviour:** add `AND <chunk_id> IS NOT NULL` to every `SELECT DISTINCT video_chunk_id FROM frames …` and `SELECT DISTINCT audio_chunk_id FROM audio_transcriptions …` subquery used with `IN`/`NOT IN`, in all of the statements listed. Apply the same fix to `estimate_evictable_bytes` (`:6750-6774`) so `/data/storage-preview` agrees with what eviction will actually do. Whether the target's estimator is also fixed was not separately verified; the fix is safe either way.
* **Base files:** `crates/screenpipe-db/src/db.rs` (functions above). Add a regression test with one NULL-`video_chunk_id` frame outside the range.

### gap_retention-v2-engine-007 — Time-range deletion: table coverage and result shape

* **Kind:** changed. **Size:** S. **Confidence:** observed.
* **Evidence:** target `delete_time_range_batch` statement list `0x6aab450..0x6aabed2` (anchor select at `0x6aab9f9`), in order:
  1. the cloud-only video select (archive variant);
  2. the local video select (`file_path NOT LIKE 'cloud://%'`);
  3. the audio select;
  4. the anchor select `SELECT DISTINCT f.id FROM frames f WHERE f.timestamp BETWEEN ?1 AND ?2 AND EXISTS (SELECT 1 FROM frames ref WHERE ref.elements_ref_frame_id = f.id AND ref.timestamp NOT BETWEEN ?1 AND ?2)`;
  5. `DELETE FROM elements …`;
  6. re-home trio (`SELECT MIN(id) …`, `UPDATE elements SET frame_id …`, `UPDATE frames SET elements_ref_frame_id …` ×2);
  7. `DELETE FROM frames WHERE timestamp BETWEEN ?1 AND ?2`;
  8. `DELETE FROM audio_transcriptions …`;
  9. `DELETE FROM ui_events …`;
  10. `DELETE FROM display_layout WHERE timestamp BETWEEN ?1 AND ?2 AND id NOT IN (SELECT id FROM display_layout WHERE timestamp BETWEEN ?1 AND ?2 ORDER BY timestamp DESC, id DESC LIMIT 1)`.

  The snapshot select is deduped with evict's. Debug: `delete_time_range_batch committed: frames=, audio_transcriptions=, accessibility=, ui_events=` (no `ocr=`). `delete_time_range committed: frames=, audio_transcriptions=, audio_chunks=, video_chunks=, accessibility=, ui_events=` (`0x5140d13`). `delete_time_range_local committed: frames=, audio_transcriptions=, audio_chunks=, video_chunks=, ui_events=` (`0x5141e7f`). Response field list `frames_deleted audio_transcriptions_deleted audio_chunks_deleted video_chunks_deleted accessibility_deleted ui_events_deleted` (`0x5151f10`); `ocr_deleted` has 0 raw hits. Retention All log `0x6ab8f23` drops `ocr=`.
* **Behaviour:**
  * **ocr_text:** no longer deleted, because the table is gone (db-005). Remove `ocr_deleted` from `DeleteTimeRangeResult` and from the `/data/delete-range` `DeleteTimeRangeResponse` JSON. Freenpipe can keep the field with value 0 for one release if a client depends on it, but the target does not emit it. Retention All's batch total becomes `frames + audio_transcriptions + ui_events`.
  * **display_layout:** in each deleted range, delete every layout row except the newest one by `(timestamp DESC, id DESC)`. Rationale: layout rows are written only on change, so the newest row before the remaining data still describes the arrangement that data was captured under. Over many 1-hour batches this leaves at most one row per batch-hour of old history, which is negligible. Apply this to `delete_time_range_batch`, `delete_time_range` and `delete_time_range_local`. The last of these reuses the deduped literal and is inferred.
  * **elements re-homing:** unchanged from base.
  * **Lean does not call this function.**
  * **Format caveat:** `display_layout.timestamp` defaults to `strftime('%Y-%m-%dT%H:%M:%fZ')` (`Z` suffix), while bounds are bound as `to_rfc3339()` (`+00:00`). Lexical `BETWEEN` can therefore misplace rows by a sub-second at an hour boundary. The target accepts this; Freenpipe can match it, or normalise both to the same form if it writes `display_layout` with explicit RFC 3339.
* **Base files:** `crates/screenpipe-db/src/db.rs` (`DeleteTimeRangeResult` :68, `delete_time_range` :6206, `_local` :6410, `_batch` :6806); `crates/screenpipe-engine/src/routes/data.rs` (`DeleteTimeRangeResponse` :33, handler :105-115); `retention.rs` All-branch log. Depends on db-005 and db-007.

### gap_retention-v2-engine-008 — Retention on hybrid (Parquet) storage

* **Kind:** new. **Size:** L. **Confidence:** observed (SQL, messages, file/line), inferred (control flow, how it is reached from the lean strip).
* **Evidence:**
  * `screenpipe-db/src/storage/sealing.rs` literals at `0x6ab1b98..0x6ab24b2`. `Location` records resolve to `sealing.rs` lines 123-148 (payload replacement) and 23/38/45/46 (retention loop); `rv2_va.py loc 6ab2300`.
  * Replacement SQL:
    * `replacement requires hybrid storage` (`0x6ab1b98`)
    * `SELECT p.generation,p.state,p.file_id,p.bytes,m.policy,m.staging_bytes FROM frame_payloads p CROSS JOIN storage_metadata m WHERE p.frame_id=?` (`0x6ab1c30`)
    * `UPDATE storage_metadata SET maintenance=1,revision=revision+1,staging_bytes=?`
    * `UPDATE frames SET full_text=?,accessibility_text=?,accessibility_tree_json=?,text_json=?,window_name=COALESCE(?,window_name),browser_url=COALESCE(?,browser_url),payload_full_text_length=…,payload_detail_present=? IS NOT NULL OR ? IS NOT NULL WHERE id=?`
    * `UPDATE frame_payloads SET generation=generation+1,state='staged',file_id=NULL,bytes=?,policy=?,completed_surfaces=?,attempts=0,retry_at=NULL,last_error=NULL WHERE frame_id=?` (`0x6ab1f3e`)
    * the redacted-at bitmask update
    * `DELETE FROM frames_fts WHERE rowid=?` + re-insert
    * `UPDATE payload_files SET state='dirty' WHERE id=?` (`0x6ab22d8`)
    * `UPDATE storage_metadata SET maintenance=0`
  * Retention-specific literals: `retention generation changed; retry` (`0x6ab2350`), `SELECT f.id,p.policy,p.completed_surfaces FROM frames f JOIN frame_payloads p ON p.frame_id=f.id WHERE f.timestamp BETWEEN ? AND ? AND f.payload_detail_present=1 ORDER BY f.id LIMIT 128` (`0x6ab2373`), column name `completed_surfaces` (`0x6ab2460`), `retention frame disappeared; retry` (`0x6ab2490`). These sit in `sealing.rs`, not `snapshot.rs`: the file pointer of the adjacent `Location` records is `0x146ab27bb` → `sealing.rs`, length 88. The coordinator hint said `snapshot.rs`, whose path string follows immediately.
  * Related trigger behaviour is in db-021: `hybrid_frame_sealed_guard` aborts plain updates of payload columns on sealed frames; `hybrid_frame_update` and `hybrid_frame_delete` keep accounting.
* **Why it exists:** in hybrid mode a sealed frame's `accessibility_tree_json`/`text_json` live in the immutable `detail.parquet` of a payload file, and the resident columns are NULL. The plain strip UPDATE (-003 c) cannot reach them: its `IS NOT NULL` guard skips them, and a direct write would hit the sealed guard. The target therefore has a dedicated hybrid routine for retention.
* **Behaviour (paraphrased; the control flow is inferred from the literal set):**
  1. **Entry.** Only when the active storage format is `hybrid-parquet-v1`; calling it on plain SQLite errors with "replacement requires hybrid storage". In the lean cycle it runs for the same `[start, end]` batch, after the SQL strip transaction (-003) has committed (inferred placement). The SQL strip has already handled staged frames, whose `payload_detail_present` the `hybrid_frame_update` trigger reset to 0, so this routine sees essentially only sealed frames.
  2. **Page loop.** Repeatedly select up to **128** frames in the range with `payload_detail_present=1`, ordered by `frames.id`, reading `(id, policy, completed_surfaces)` from `frame_payloads`. Each successful strip drops a frame out of the predicate, so the loop needs no cursor and stops on an empty page.
  3. **Per frame:**
     * Snapshot the payload state: `generation`, `state`, `file_id`, `bytes`, plus `storage_metadata.policy` and `staging_bytes`.
     * Read the frame's current search columns (`full_text`, `accessibility_text`), from Parquet if sealed (the db-021 reader).
     * In one write transaction in maintenance mode (`maintenance=1`, `revision+1`, `staging_bytes` adjusted by the resident bytes added), perform a coordinated payload replacement:
       * write back `full_text`/`accessibility_text` unchanged, `accessibility_tree_json=NULL`, `text_json=NULL`, `window_name`/`browser_url` unchanged (NULL args keep them through `COALESCE`);
       * recompute the `payload_*` columns, so `payload_detail_present` becomes 0;
       * set `frame_payloads` back to `state='staged'`, `generation+1`, `file_id=NULL`, new `bytes`, and **the same `policy` and `completed_surfaces` read in step 2**. Redaction progress is carried over, not reset, so the frame is not re-queued for PII redaction;
       * reset the retry fields;
       * re-index `frames_fts` for the row;
       * mark the old payload file `dirty`;
       * leave maintenance (`maintenance=0`).
     * Optimistic concurrency: if the frame's `generation` changed between snapshot and write (the sealing worker or redaction touched it), abort that frame with "retention generation changed; retry". If the frame row vanished (concurrent delete), abort with "retention frame disappeared; retry". Either way the page is re-queried. Bound the retries (the storage budget's `read_attempts`; exact count not recovered).
  4. **Aftermath:** the sealing worker later re-seals the frame into a new Parquet file with an empty detail projection. Dirty files are rewritten or retired, and retired files are deleted from disk by sealing GC (db-021). Disk is therefore reclaimed asynchronously, not by the retention cycle. While frames are waiting to re-seal, their search text counts against `staging_bytes`. Stop or pause when the staging budget is exhausted instead of failing the capture path: the insert trigger aborts captures with "staging budget reached".
* **Mode semantics on hybrid storage:**
  * **Media:** unchanged (chunk files and snapshots live outside the payload store).
  * **Lean:** SQL strip (-003) plus this routine. `elements` deletion goes through the `screenpipe_elements` virtual table, which supports a `frame_id` constraint. Its counters and `_bulk_element_*` bookkeeping are maintained by the vtab (db-022). `ui_events` deletion goes through the bulk table's delete trigger, which marks bulk files dirty.
  * **All:** no special code. `DELETE FROM frames` fires `hybrid_frame_delete`, which drops the FTS row, marks the payload file dirty, subtracts staged bytes and deletes `frame_payloads`. Bulk tables' delete triggers mark their files dirty. Sealing GC reclaims the files. `cleanup_orphaned_chunks` and `PRAGMA incremental_vacuum` still apply to the SQLite index.
  * `/data/compact` (online VACUUM) is refused on hybrid indexes (db-024/db-025). Physical shrink of the index needs the offline compact.
* **Freenpipe sequencing:** do this only when db-021/db-022 are implemented. Until then Freenpipe refuses to open hybrid roots (db-021 minimum), so retention only ever sees plain SQLite. Keep a `StorageFormat` check at the call site so the SQL strip never targets a hybrid root without this routine.
* **Base files:** new `crates/screenpipe-db/src/storage/sealing.rs` (or `storage/retention.rs`); `strip_heavy_text_in_range` call site; `retention.rs` (unchanged API).

### gap_retention-v2-engine-009 — Semantic run/item GC after frame deletion

* **Kind:** new. **Size:** S. **Confidence:** observed (SQL, migration comment), inferred (call sites).
* **Evidence:** `DELETE FROM semantic_runs WHERE NOT EXISTS ( SELECT 1 FROM frames WHERE frames.semantic_run_id = semantic_runs.id )` (`0x5145407`, db copy `0x6aaf328`), then `DELETE FROM semantic_items WHERE NOT EXISTS ( SELECT 1 FROM semantic_run_items WHERE semantic_run_items.item_id = semantic_items.id )` (`0x5145510`). `Location` `semantic.rs:1164`. The migration comment for `semantic_actors` indicates that aliases persist after retention removes semantic items (`0xa2a9eea`), so items are expected to disappear when their source frames age out.
* **Behaviour:** after frames are deleted (retention All cycle, `/data/delete-range`, `delete_by_machine_id`), run the two statements in order. Deleting runs cascades `semantic_run_items` (`ON DELETE CASCADE`). Items then have no run-items left; their RESTRICT FK is satisfied and the second statement removes them. `semantic_item_actors` cascades, and `semantic_items_fts` follows through `semantic_items_ad`. `semantic_actors`/`semantic_actor_aliases` are kept on purpose. Lean and Media never delete frames, so no GC is needed for them.
* **Placement (inferred):** run it outside the per-batch transaction, once per cycle, next to `cleanup_orphaned_chunks`. Both are full-table anti-joins. Log and continue on failure.
* **Base files:** `crates/screenpipe-db/src/db/semantic.rs` (new, owned by db-008); call from `retention.rs` post-loop and `routes/data.rs` delete handlers. Depends on db-008.

### gap_retention-v2-engine-010 — App auto-start passes `lean`; fresh-store guard

* **Kind:** changed. **Size:** S. **Confidence:** observed (accepted set, guard message), inferred (default policy around the guard).
* **Evidence:** app `src\sync.rs` mode allow-list literals `all`,`lean`,`media`, then `localRetentionEnabled localRetentionDays localRetentionMode` (`0x52ac0c0`); app log strings `local retention auto-started (retention=…`, `local retention auto-start failed (…`, `local retention auto-start: server not reachable: …` (`0x4994660..0x49946cc`, base-present). The app `store.rs` message (`0x52ad1b7`), paraphrased: while creating a new settings store the app found prior recordings (`db.sqlite`) in the data dir, so it keeps local retention disabled on the assumption that the store was recovered or migrated. Base: `apps/screenpipe-app-tauri/src-tauri/src/sync.rs:756-761` filters to `media|all`.
* **Behaviour:**
  * `auto_start_retention` accepts `lean` in its mode filter. Without this, a user who picked lean silently runs media after every restart.
  * When the app creates a brand-new settings store (no prior `store.bin`) but `<data_dir>/db.sqlite` already exists, it must keep `localRetentionEnabled=false`. Never auto-enable deletion in that case, and log that it is doing so. This protects users whose store was reset (corruption recovery, reinstall, data-dir migration) from having old recordings deleted by a default. Whether the target defaults retention **on** for truly fresh installs was not recovered: the JS defaults still say `false/14/"media"` (`378-…js` @18435). Freenpipe keeps the off default, so the guard is mostly a safety net if defaults ever change.
* **Base files:** `apps/screenpipe-app-tauri/src-tauri/src/sync.rs` (`auto_start_retention`), `apps/screenpipe-app-tauri/src-tauri/src/store.rs` (fresh-store init path). The UI already exists (frontend-039).

### gap_retention-v2-engine-011 — CLI `--retention-mode lean`

* **Kind:** changed. **Size:** S. **Confidence:** inferred (the target app ships no CLI parser for these flags; raw search for `retention-days`/`retention-mode` returns 0 hits).
* **Behaviour:** the Freenpipe engine CLI keeps `--retention-days` (0 = off) and `--retention-mode`. With -001 the value enum automatically accepts `lean`. Update the help text to describe all three modes. Add a banner arm for Lean, e.g. "lean (drop media + element detail)", in `bin/screenpipe-engine.rs:1588-1593`. Optionally reject `--retention-days` above 36500 at parse time (clap range) for parity with -005. The CLI auto-enable POST (`:1804-1842`) already forwards `mode` unchanged.
* **Base files:** `crates/screenpipe-engine/src/cli/mod.rs:632-642`, `crates/screenpipe-engine/src/bin/screenpipe-engine.rs:1575-1595`.

---

## 4. Ordering of one retention cycle (target, consolidated)

1. Wake up (timer or `/retention/run`), read `(days, mode)`; skip if disabled.
2. Checked cutoff; skip the cycle with the "out of range" warning on overflow.
3. `get_oldest_timestamp`; skip on error or no data; log "cleaning up data from … to …".
4. For each 1-hour batch from oldest to cutoff:
   * **Media:** `evict_media_in_range` (one tx), then unlink the returned files.
   * **Lean:** `evict_media_in_range` (tx 1), then unlink; then `strip_heavy_text_in_range` (tx 2: re-home anchors, delete elements, NULL frame AX/OCR JSON, delete ui_events); then, on hybrid storage, the sealed-payload detail strip (per-frame maintenance txs).
   * **All:** `delete_time_range_batch(…, collect_all_files=true)` (one tx: re-home anchors, delete elements, frames, audio_transcriptions, ui_events, display_layout keep-newest; triggers cascade the activity ledger and hybrid accounting), then unlink.
   * Sleep about 500 ms.
5. Post-loop, only when something changed: All runs `cleanup_orphaned_chunks` (NULL-safe) and semantic GC (-009, inferred); All, and inferred Lean, run `PRAGMA incremental_vacuum(1000)`.
6. Record `last_cleanup`/`last_error`/`total_deleted`.

Each batch step is its own `BEGIN IMMEDIATE` transaction, and files are unlinked only after commit. A failed step logs and moves on to the next step or batch; one bad hour never aborts the cycle. That matches base behaviour and the target's per-step messages.

---

## 5. Coverage of newer tables (question 6)

| Table / data | Touched by retention? | How | Evidence |
|---|---|---|---|
| `frames` | All: deleted. Lean: `accessibility_tree_json`,`text_json` NULLed. Media: `snapshot_path` NULLed | -003, -007 | `0x6aabcbf`, `0x6aac088` |
| `elements` (+`elements_fts`) | All and Lean: deleted for in-range frames after anchor re-homing | -003, -007 | `0x6aabb1c` |
| `ui_events` | All and Lean: deleted in range | -003, -007 | `0x6aabd41` |
| `audio_transcriptions` | All only | -007 | `0x6aabcf3` |
| `video_chunks`/`audio_chunks` | Media/Lean: `file_path=''`, `evicted_at` set. All: orphan rows deleted post-loop | -006 | `0x6aaa600..`, `0x6aab340` |
| `display_layout` | All: deleted in range except the newest row | -007 | `0x6aabd6c` |
| activity ledger (`activity_*`) | indirectly, by the db-013 AFTER DELETE triggers on frames, ui_events, audio_transcriptions. Note that **Lean** also erodes UI-event-only evidence | triggers | `0xa2ad0a9..0xa2ad6aa` |
| `semantic_runs`/`semantic_run_items`/`semantic_items`/`semantic_item_actors` | All: GC of unreferenced rows (caller inferred); `frames.semantic_run_id` has no cascade toward runs | -009 | `0x5145407` |
| `semantic_actors`/`semantic_actor_aliases` | never (by design) | — | `0xa2a9eea` |
| `memories` | never; `memories.frame_id` becomes NULL via `ON DELETE SET NULL` when All deletes the frame (base FK) | — | `0x5118563` (doc says frames are pruned by retention and memories are the durable link) |
| `vision_tags` | All: cascade via `vision_tags.vision_id → frames ON DELETE CASCADE` (base FK) | — | base migration `20240821074222` |
| `outputs`, `output_search_*`, `feedback`, `meetings`, `meeting_transcript_segments`, `pipe_executions`, `pipe_event_runs` | not by retention. Pipes prune their own history (`ROW_NUMBER` keep-N, `claimed_at` cutoff); others only by explicit id deletes | — | DELETE inventory via `rv2_rawctx.py` (140 statements, none time-ranged for these) |
| `starred_sessions` | **neither deleted nor protected.** No evict, strip, delete or retention statement or message references it; frames/media inside a starred interval age out like any other | — | all `starred_sessions` SQL is in `db/starred.rs` (`0x516d1ff..0x516d467`, `0x524853b`) and search/activity code |

Freenpipe recommendation: match the target (no protection) for parity, and record the question in open questions. Protecting starred intervals would be a product decision; if adopted, implement it by splitting batch ranges around the merged starred islands. The target already has a merged-islands query (`0x524853b`, `LIMIT 101`) that could be reused. Mark it as a Freenpipe extension.

---

## 6. `/data/storage-preview` and `/data/evict-media` (question 5)

* **storage-preview:** unchanged in the target. The route doc string is identical to base (`0x6a6c95c`; in our words, it describes an estimate of the disk space that media eviction would free for the requested window, and names the retention confirm dialog as its consumer). The query fields are still `older_than_days | start,end` (`0x5151df0`) and the response is still `file_count`,`bytes`. The settings UI calls `/data/storage-preview?older_than_days=<q>` without a mode for every mode (page-d9c6…js @541416) and, in the confirm dialog, shows the returned byte total and file count as the space the action would free, identically for media, lean and all (@555600-556810). **The estimate therefore ignores lean's database stripping and all's row deletion; it only counts media files.** No delta is needed for parity. The NULL-safe fix in -006 should also be applied to `estimate_evictable_bytes`.
  * Optional Freenpipe extension (non-interop, additive): accept `mode` and return extra fields such as `db_bytes_estimate`, using `SUM(length(accessibility_tree_json)+length(text_json))` plus an element-row estimate. Existing clients ignore unknown fields. Do not change `file_count`/`bytes` semantics.
* **evict-media:** unchanged (`EvictMediaResponse` = `video_chunks_evicted, audio_chunks_evicted, snapshots_evicted, video_files_deleted, audio_files_deleted, snapshot_files_deleted, bytes_freed`). It does not strip text. Lean is only reachable through the retention loop; there is no `/data/strip-*` route in the oasgen route blob (`0x5257ca8`).

---

## 7. Open questions

1. Whether incremental vacuum also runs after a Lean cycle. Inferred yes; the strings support either reading.
2. Whether the target's lean strip performs the `elements_ref_frame_id` anchor re-homing. The literals are present in the same CGU and the step is needed for correctness; it is recommended regardless.
3. Exact caller(s) of the semantic GC (`semantic.rs:1164`): retention post-loop, `/data/delete-range`, a periodic semantic worker, or several of these.
4. How the hybrid sealed-payload strip is reached: called from `strip_heavy_text_in_range`, from the engine lean branch, or from the sealing worker reacting to a policy. Also its retry bound.
5. Whether the loop's "failed to get oldest timestamp" path updates `last_cleanup`/`last_error`. Base updated `last_cleanup` because the error path returned `Ok(0)`.
6. App default for truly fresh installs: is `localRetentionEnabled` ever defaulted on, which is what the fresh-store guard implies? It matters only if Freenpipe changes defaults.
7. Whether the target's `estimate_evictable_bytes` also gained the NULL-safe subqueries. The function name is not in strings, and its SELECTs are shared and deduped with evict's.
