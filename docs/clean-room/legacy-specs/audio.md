# Audio capture, transcription and speakers: delta spec (base 892199f → screenpipe v2.7.104)

Dimension key: `audio`. Delta ids: `audio-NNN`.
Clean-room status: everything below is paraphrased from static evidence (binary strings, embedded SQL, extracted frontend bundle, files shipped next to the exe). Interop identifiers (routes, JSON fields, SQL, settings keys, env vars, file names, event names) are quoted exactly. No upstream source was consulted. The embedded `frontend/CHANGELOG.md` was deliberately **not** read, because it is a post-relicense changelog (CLAUDE.md rule).

## 0. Conventions and evidence

* `0x…` values are hex **file offsets** in `<re-workspace>/app_strings.tsv`. Rust packs neighbouring literals, so one offset often covers a run of related strings.
* `FE:` paths are relative to `<re-workspace>/frontend/`. `@N` is a character offset inside that JS file.
* REA evidence: parent JS application analysis `ev_57f23857f31e9207a9f71e1d612412e78f9c5dc0d29af8c68f97372786b6216d`; feature trace for the `speakers` literal `ev_113bba37bf5e61dcd11fc0dc3dec255f44b702e95b684208e938ff8ae1967dee` (2 seed matches in endpoint literals of two JS assets). Traces for `/audio/`, `/speakers/sample/` and `/speakers/reassign` returned `no-match` (`ev_ab7e43b7…`, `ev_ee9f3a6c…`, `ev_0edc2d8a…`), because the REA graph only partly covers this bundle. Most frontend facts therefore come from direct reads of the chunk files.
* Working files and tools: `<re-workspace>/out/audio/*.txt` (region dumps), `tools\audio_sgrep.py`, `tools\audio_region.py`, `tools\audio_jsctx.py`, `tools\audio_migtable.py`. The last one reads the sqlx migration-version table from the exe (88-byte stride, first entry near file offset `0xa2aec30`). Output: `out\audio\migversions.txt`, 117 versions.
* Target source modules come from panic and tracing paths (`out\audio\src_paths.txt`).
* **Confidence:** *observed* means stated directly by strings, SQL, frontend code or shipped files. *Inferred* means reconstructed from log wording or structure. Every numeric default not visible in strings is marked *inferred*.
* Schema DDL for the new migrations belongs to `specs/db.md` (db-001, db-006, db-011, db-014). Versions there match what was extracted here. This spec covers the audio-side behaviour that reads and writes those columns.

## 1. Inventory: base vs target

### 1.1 Module map

| Area | BASE (Freenpipe @892199f) | TARGET v2.7.104 (observed source paths / strings) | Status |
|---|---|---|---|
| Device enumeration / default following | `core/device.rs`, `core/device_detection.rs`, `audio_manager/device_monitor.rs` (DEVICE_RECOVERY, PINNED_FALLBACK, eCommunications output) | same files, plus a Windows PnP container probe (`PKEY_Device_ContainerId`, `CM_Locate_DevNodeW/CM_Get_Parent/CM_Get_Device_IDW`, `SWD\MMDEVAPI\`, `0x6afa134..0x6afa2a1`), a Bluetooth transport/combo probe (`0x6afa4e8`, `0x6afa950`) and a render-endpoint activity query (`0x6afa840`) | changed (audio-011, audio-017) |
| Windows system audio | cpal WASAPI loopback on the render device | own WASAPI loopback client `core/process_tap/windows.rs`: `default-render-endpoint`, `process-tree:`, `exclude-process-tree:` targets (`0x6af8430..0x6af9c41`) | new (audio-013) |
| macOS system audio | CoreAudio Process Tap `core/process_tap.rs` (+ exclusions) | split into `core/process_tap/…`; macOS unchanged in substance | – |
| Audio exclusions | macOS-only file `~/.screenpipe/audio-exclusions.json`; Tauri `audio_exclusions.rs` read/write; `read_app_bundle_metadata` macOS only | Windows engine honours the same file (exe names); Windows `read_app_bundle_metadata` reads `.exe` (`0x498df7a`); the UI limits Windows to a single excluded app | changed (audio-014) |
| Mic AEC | WASAPI endpoint AEC (`windowsInputAecEnabled`), macOS VPIO | adds software AEC3+NS+AGC2 via the `sonora` crate (`core/aec.rs`), `aecMode` enum `off|screenpipe|macos|windows` | new (audio-012) |
| Capture gating | always capture (plus DRM, schedule, lock) | `audioCaptureMode` `always|meetings-only|disabled`, meeting-session devices, Bluetooth meeting gate | new (audio-010, audio-011) |
| Meeting-aware capture | live tap for meeting streaming (`meeting_streaming/*`) | adds `audio_manager/meeting_piggyback.rs`, `piggyback_listeners.rs`, `core/meeting_audio/windows.rs`, `meeting_processes.rs` (WASAPI session snapshot) | new (audio-015, audio-016) |
| Output follow / health | n/a | `audio_manager/windows_output_follow.rs` (`[AUDIO_FOLLOW]`, `[SPEAKER_WATCHDOG]`), Tauri `engine_events/audio_health.rs` | new (audio-017, audio-018) |
| VAD | silero v5 (vad-rs fork) with WebRTC fallback; downloads `silero_vad_v5.onnx` | same logic; model also shipped at `<install>\vad\silero_vad_v5.onnx` | changed, packaging only (audio-022) |
| Chunking / source buffer | 30 s chunks, BT/wired gap fill, startup grace, lock handling | same strings (`0x6b1db34..0x6b1dd95`, `0x512c5a9..0x512cab0`) | unchanged |
| Engines enum | Deepgram, WhisperTiny(+Q), WhisperLargeV3(+Q), WhisperLargeV3Turbo(+Q), OpenAICompatible, Qwen3Asr, Parakeet, ParakeetMlx, Disabled | identical Display list (`0x6b15c2d`) and parse names (`0x6b13940`, `0x6b13aaf`) | unchanged set |
| Whisper | whisper-rs, `use_gpu(true)`, no Windows GPU backend compiled | ggml-vulkan compiled in (`GGML_VK_*` `0x6ba9870..`), `vulkan-1.dll` shipped; GPU→CPU recovery path (`0x49a56e7..0x49a5a46`, `0x6af62f0..0x6af6b38`) | changed (audio-019) |
| Parakeet / Qwen3-ASR | audiopipe rev 2fd5c36, features `qwen3-asr-antirez-blas`,`parakeet`; `directml` feature wired | audiopipe rev b099a2e: DirectML EP + CPU recovery + session recycling; `transcription/parakeet_windows.rs` adapter picker, `SCREENPIPE_DIRECTML` | changed (audio-020) |
| Deepgram batch | `/v1/listen` direct or screenpipe-cloud proxy, IPv6→IPv4 retry | adds rate-limit retry (`reset_in`), cloud account codes | changed (audio-024) / decloud (audio-030) |
| OpenAI-compatible | single attempt, 30 s timeout | retries, HTTP 429 backoff, timeout scales with audio length (`0x49a4b04..0x49a4c26`) | changed (audio-024) |
| Live meeting streaming | `meeting_streaming/*` (selected-engine, deepgram-live, screenpipe-cloud), stall events, 15 min / 2 h limits | same, plus `stream_id` and `session_speaker_id` on delta/final events and segments | changed (audio-001); decloud (audio-031) |
| Reconciliation | batch sweep, PendingTranscription (12 fields), local diarization, merged audio replace | PendingTranscription (15 fields) with staged merged audio + sha256, `pending-chunks/` markers, DB-error pause, chunk cap | changed (audio-023) |
| Diarization tables | `diarization_runs`, `diarization_segments`, `speaker_identity_evidence` (20260515000000) | DDL identical (`0xa2a0127..0xa2a0ba4`); same insert SQL (`0x49819a8..0x4981bfa`) | unchanged |
| `audio_transcriptions_new` migrations | 20241127172038, 20241210205142 | text identical (`0xa28a530..0xa28a681`, `0xa28c4c0..0xa28cb71`) | unchanged |
| Speaker identification | single nearest-below-threshold match (0.55), always learn | top-2 candidates, ambiguity abstain, duration gates (identify / profile-update), modes `skip|match_only|match_and_learn` | changed (audio-007) |
| Speaker routes | unnamed, update, search, delete, hallucination, merge, similar, reassign, undo-reassign | same nine plus `GET /speakers/sample/:audio_chunk_id`; reassign `scope`, whole-speaker rename, segment undo; search pagination and `include_samples` | changed/new (audio-002, audio-003, audio-005, audio-006) |
| Speaker DB integrity | merge/delete touch 2 tables | merge/reassign touch 5 tables; orphan restore migration; table self-heal | changed (audio-004, audio-008, db-011) |
| Calendar/self speaker naming | `calendar_speaker_id.rs` (dedup, alias cleanup, auto self naming) | same features; auto-self query is much stricter (`0x6ab0f21..0x6ab1424`) | changed (audio-009) |
| Retranscribe | `/audio/retranscribe`, `/meetings/:id/retranscribe`, backlog routes | same routes; meeting response adds two fields and emits a summary-refresh event | changed (audio-025) |
| Device routes | `/audio/list`, `/audio/start|stop`, `/audio/device/start|stop|status` | same routes; disabled-mode error and status-change event | changed (audio-026) |
| Metrics / health | `AudioPipelineMetrics`, `/health` audio fields, `/ws/metrics` | adds `last_stream_timeout_at`, `audio_capture_mode`, `mic_capture_active` | changed (audio-027) |
| ORT init | panic-guarded session init | watchdog thread with timeout; statuses `ok|init_failed|init_timed_out` feed `health-startup.json` | new (audio-021) |
| Eval crates | `screenpipe-audio-eval`, `screenpipe-meeting-eval` (dev only) | not shipped in the app; no observable delta | unchanged |

### 1.2 Backends compiled into the Windows x64 target (inferred from strings and shipped DLLs)

| Backend | Evidence | Conclusion |
|---|---|---|
| whisper.cpp + Vulkan | `ggml-vulkan.cpp`, `GGML_VK_*` env names (`0x6ba9870..0x6ba9e50`), `ggml_backend_vk_*` (`0x6bab190..`), `vulkan-1.dll` + `VulkanRT-License.txt` in install dir, a log naming the GPU backend asked for per platform (Metal for macOS, Vulkan for Windows) (`0x6b0e070`) | whisper-rs built with Vulkan; GPU on by default; CPU fallback |
| Parakeet TDT 0.6B v2/v3 ONNX | `istupakov/parakeet-tdt-0.6b-v3-onnx`, `…-v2-onnx`, `encoder-model.int8.onnx`, `decoder_joint-model.int8.onnx`, `vocab.txt` (`0x6b451a8..0x6b45370`) | audiopipe parakeet via ORT, DirectML EP with CPU fallback |
| Qwen3-ASR 0.6B (antirez C, OpenBLAS) | `qwen3_asr_antirez.rs`, `Qwen/Qwen3-ASR-0.6B`, `libopenblas.dll` shipped | present |
| Qwen3-ASR 0.6B ONNX | `louis030195/qwen3-asr-0.6b-onnx`, `conv_stem.onnx`, `encoder.onnx`, `decoder_kv.onnx` (`0x6b47b14..0x6b47c28`) | present, DirectML EP |
| Deepgram (direct / cloud proxy) | `https://api.deepgram.com/v1/listen`, `https://api.screenpipe.com/v1/listen` (`0x6b13f73`, `0x6b13fa2`) | present (cloud proxy must be removed, audio-030) |
| OpenAI-compatible | `/v1/audio/transcriptions` (`0x6b036b2`), default `whisper-1` @ `http://127.0.0.1:8080` (`0x498c2c0`) | present |
| Parakeet-MLX | not applicable on Windows | absent |
| ONNX Runtime | `onnxruntime.dll`, `onnxruntime_providers_shared.dll`, `DirectML.dll` shipped; crate `ort-2.0.0-rc.12` | ORT 2.0 rc.12 with the DirectML EP (base pins rc.10) |
| Speaker models | `<install>\models\segmentation-3.0.onnx`, `<install>\models\wespeaker_en_voxceleb_CAM++.onnx` | bundled (audio-022) |
| AEC | git checkout `sonora-c0cc0c3c8411109a/70c6736` with crates `sonora`, `sonora-aec3`, `sonora-agc2`, `sonora-ns`, `sonora-fft`, `sonora-simd`, `sonora-common-audio` (`0x9f43740..`) | pure-Rust WebRTC audio-processing port (audio-012) |

### 1.3 Settings surface (Rust `RecordingSettings` field order, `0x5a42a38`, `0x52305f0`, `0x52c8878`)

`disableAudio, audioTranscriptionEngine, transcriptionMode, audioCaptureMode*, meetingLiveTranscriptionEnabled, meetingLiveTranscriptionProvider, audioDevices, useSystemDefaultAudio, experimentalCoreaudioSystemAudio, experimentalMeetingPiggyback*, alwaysRecordBluetoothMic*, windowsInputAecEnabled, macosInputVpioEnabled, screenpipeAecEnabled*, aecMode*, audioChunkDuration, deepgramApiKey, filterMusic, batchMaxDurationSecs, vocabularyWords, …, disableMeetingDetector, ignoredMeetingApps, …, recordWhileLocked, languages, …, userName, openaiCompatibleEndpoint, openaiCompatibleApiKey, openaiCompatibleModel, openaiCompatibleHeaders, openaiCompatibleRawAudio, …`. Fields marked `*` are absent from base `crates/screenpipe-config/src/recording.rs`. Details in audio-028.

## 2. Delta items

---

### audio-001: Live-stream-scoped meeting speakers (`stream_id`, `session_speaker_id`)

* **Kind:** changed. **Size:** M. **Confidence:** observed (schema, event fields, insert SQL, frontend use); inferred (id format).
* **Evidence:** migration `20260828143000_scope_live_meeting_speakers` (`0xa2ade13..0xa2ae17a`; DDL in db-014). Event structs: `MeetingTranscriptDelta` has 10 fields and `MeetingTranscriptFinal` has 11 (`0x6b1e1e0`, `0x6b1e220`). Field strings `stream_id item_id device_type delta replace captured_at session_speaker_id speaker_name` (`0x6b04fc8`, `0x6b24167`). Insert `INSERT OR IGNORE INTO meeting_transcript_segments (meeting_id, provider, model, item_id, device_name, device_type, stream_id, session_speaker_id, speaker_name, transcript, captured_at) VALUES (?1..?11)` (`0x6b31fc8`). Deepgram live tokens `:turn:`, `speaker `, `channel_index`, `deepgram:` (`0x6af7aa8..0x6af7cb8`). Transcript row shape with `session_speaker_id` (`0x51a10e0`, `0x6a303a0`). FE: `_next/static/chunks/app/(main)/home/page-c18385b042b3382e.js` @464245..@465182 keeps `sessionSpeakerId` on live finals and caches finals in `sessionStorage` key prefix `screenpipe-meeting-live-finals:`.
* **Behaviour:**
  1. Each live provider connection (one websocket per device and meeting) gets an opaque `stream_id`. It is minted when the stream opens and changes after any reconnect.
  2. A Deepgram diarization label identifies a speaker only within the stream that produced it, so the speaker identity of a live final is `session_speaker_id`. It is an opaque string that combines provider, stream and provider label (suggested shape `deepgram:<stream_id>:speaker:<n>`; inferred). A reconnect therefore cannot merge "speaker 1" from two different streams.
  3. `speaker_name` stays the human-readable label (for example "speaker 1", or `SCREENPIPE_MEETING_LOCAL_SPEAKER_NAME` for the input device).
  4. Global cross-meeting identity remains `speaker_id`, filled in later by embedding match. Base `backfill_meeting_segment_speakers` already does this.
  5. Persist `stream_id` and `session_speaker_id` on every live final. Rows written before the migration read `stream_id='legacy'`.
  6. Grouping and labelling in meeting transcript views and summaries should key on `COALESCE(session_speaker_id, speaker_name)` (see `0x4984fff`). The meetings dimension owns the transcript endpoint shape. It must expose `session_speaker_id`.
* **Data shapes:** `MeetingTranscriptDelta { meeting_id, provider, model, stream_id, item_id, device_name, device_type, delta, replace, captured_at }` and `MeetingTranscriptFinal { meeting_id, provider, model, stream_id, item_id, device_name, device_type, session_speaker_id: Option, speaker_name: Option, transcript, captured_at }`. Event names `meeting_transcript_delta` / `meeting_transcript_final` are unchanged. Tauri forwards them to `meeting-transcript-delta` / `meeting-transcript-final`.
* **Base files:** `crates/screenpipe-audio/src/meeting_streaming/{events.rs,deepgram_live.rs,controller.rs,selected_engine.rs}`, `crates/screenpipe-db/src/db.rs` (segment insert, transcript select), `apps/screenpipe-app-tauri/components/meeting-notes/*`.
* **Notes:** The selected-engine path has no provider diarization. Use `stream_id` = session id and leave `session_speaker_id` NULL. Keep the unique key `(meeting_id, provider, item_id, device_name, device_type)` and put the stream id into `item_id` (the `:turn:` token suggests `<stream>:turn:<n>`; inferred) so reconnects do not collide.

### audio-002: Speaker autocomplete and people-filter support

* **Kind:** changed. **Size:** S. **Confidence:** observed.
* **Evidence:** migration `20260616120000_speaker_filter_autocomplete_indexes` (`0xa2a4dad`; db-006). Light search SQL: `SELECT MIN(id) as id, name, '{}' as metadata FROM speakers WHERE name LIKE ? || '%' COLLATE NOCASE AND hallucination = 0 AND name IS NOT NULL AND name != '' GROUP BY name ORDER BY name COLLATE NOCASE LIMIT ? OFFSET ?` (`0x5147a16..0x5147b64`). The full search adds `LIMIT ? OFFSET ?` to `NamedSpeakers` (`0x5147b96..0x5147cb1`). Request field `include_samples` (`0x5153508`). FE callers: `_next/static/chunks/3308-e676c712bfd27088.js` @37302 (`name`, `limit` 5 or 20, `include_samples=false`, 3 s timeout); `3696-4fbfea4ced3de899.js` @380880/@381919/@383722 (`limit=50&include_samples=false`, @-mention speaker suggestions); settings and home pages call `/speakers/search?name=…` with no extra parameters.
* **Behaviour:** `GET /speakers/search` accepts `name` (prefix; empty matches all), `limit`, `offset`, and `include_samples` (default true).
  * When `include_samples=false`: run the light query. It returns `{id, name, metadata:"{}"}` grouped by name, ordered case-insensitively and paginated. The `idx_speakers_hallucination_name` index serves it.
  * When `include_samples=true`: keep the base sampled query, plus pagination and NOCASE ordering.
  * Defaults when absent (inferred): `limit=100`, `offset=0`.
* **Base files:** `crates/screenpipe-engine/src/routes/speakers.rs` (`SearchSpeakersRequest`), `crates/screenpipe-db/src/db.rs::search_speakers`.

### audio-003: `audio_samples` entries gain `audio_chunk_id`, and unplayable paths are filtered

* **Kind:** changed. **Size:** S. **Confidence:** observed.
* **Evidence:** every `RecentAudioPaths` CTE now selects `ac.id as audio_chunk_id` and emits `'audio_chunk_id', rap.audio_chunk_id` in the `json_object` (unnamed `0x5146e69..0x5147949`, similar `0x5146379..0x5146e35`, search `0x5147ce1..0x5148509`, `0x6ab00ac..0x6ab0b68`). New filter `AND TRIM(COALESCE(ac.file_path, '')) NOT IN ('', '.')` next to the existing `NOT LIKE 'cloud://%'`.
* **Behaviour:** speaker `metadata.audio_samples[]` items become `{path, audio_chunk_id, transcript, start_time, end_time, timestamp}`. Chunks with empty or `.` paths never appear as samples. The UI relies on `audio_chunk_id` to call audio-005.
* **Base files:** `crates/screenpipe-db/src/db.rs` (`get_unnamed_speakers`, `search_speakers`, `get_similar_speakers`).

### audio-004: Speaker merge and reference integrity across all speaker-referencing tables

* **Kind:** changed. **Size:** S. **Confidence:** observed (SQL); inferred (exact ordering).
* **Evidence:** table list `audio_transcriptions meeting_transcript_segments speaker_embeddings diarization_segments speaker_identity_evidence` used with templated `UPDATE <table> SET speaker_id = ?1 WHERE speaker_id = ?2`, then `DELETE FROM speakers WHERE id = ?1`, `SELECT COUNT(*) FROM <table> WHERE speaker_id = ?1`, and a warning log that keeps the absorbed speaker because some table still references it (`0x5145680..0x514584a`). Same table list at `0x6aafdd0`. Undo uses `SELECT EXISTS(SELECT 1 FROM speakers WHERE id = ?)` + `INSERT INTO speakers (id, name) VALUES (?, '')` + `UPDATE <table> SET speaker_id = ?1 WHERE id = ?2` (`0x5147988..0x51479f1`).
* **Behaviour:** when one speaker is absorbed into another (`/speakers/merge`, and the absorb phase of reassign propagation), repoint `speaker_id` in **all five** tables inside one IMMEDIATE transaction. Then count the remaining references to the absorbed id in each table. Delete the absorbed `speakers` row only if all counts are zero; otherwise keep it and log a warning. Speaker deletion (`/speakers/delete`) is unchanged (`0x5145890..0x5145a14` equals base). The FK actions on `diarization_segments` (SET NULL) and `speaker_identity_evidence` (CASCADE) handle those two tables. Migration `20260812120000` and the startup self-heal (db-011) repair legacy orphans.
* **Base files:** `crates/screenpipe-db/src/db.rs` (`merge_speakers`, `reassign_speaker` phase 3, `undo_speaker_reassign`).

### audio-005: `GET /speakers/sample/:audio_chunk_id` (voice-sample streaming)

* **Kind:** new. **Size:** M. **Confidence:** observed (route, params, errors, headers); inferred (cap value, encoding details).
* **Evidence:** route `/speakers/sample/:audio_chunk_id` sits in the non-OpenAPI router list (`0x5258848`, next to `/vision/status`, `/audio/metrics`). Handler strings (`0x5153810..0x51539e0`) include the content type `audio/wav` and the cache header value `private, max-age=3600`, plus error texts for: a chunk with no playable local file path; a requested range outside the chunk; cloud-hosted samples that cannot be played locally; failures to build the response, decode the sample, or load the chunk; a missing chunk; non-finite start/end values; a sample that exceeds the length cap; and an end that is not after the start. FE: `_next/static/chunks/app/(main)/settings/page-d9c69007346aaf6a.js` @615880 (player builds `?start=<s>&end=<s+duration>` on the API base URL with auth), call sites @621733, @624714 (duration capped at 3 s), @627021, @628324, @631148, @632035.
* **API:**
  * `GET /speakers/sample/{audio_chunk_id}?start=<f64 secs>&end=<f64 secs>`. Both values are offsets in seconds into the chunk file.
  * 200: `Content-Type: audio/wav`, `Cache-Control: private, max-age=3600`, body is a WAV of the decoded range (mono 16-bit PCM suggested; inferred).
  * 400: non-finite start/end, `end <= start`, or range longer than the cap.
  * 404: chunk not found.
  * 4xx: path empty or `.` (no playable file); path starts with `cloud://`; range outside the decoded duration.
  * 500: decode failure.
  * Auth: same as other recording reads. The frontend appends credentials to the URL because `<audio src>` cannot send headers. Accept the existing query-token mechanism used for media URLs.
* **Behaviour:** look up `audio_chunks.file_path`, decode it with the existing ffmpeg/symphonia decoder, slice `[start,end)` (allow a small tolerance at the end), and encode as WAV.
* **Defaults:** maximum sample length is *inferred*. Recommended: 120 s. The UI passes full transcription segment durations, which can be long in batch mode, so either use a generous cap or clamp in the client.
* **Base files:** `crates/screenpipe-engine/src/routes/speakers.rs` (new handler), `server.rs` (register beside `/audio/metrics`, outside oasgen), `crates/screenpipe-audio/src/utils/audio/pcm_decode.rs` / `utils/ffmpeg.rs` (decode), `hound` (WAV encode). Frontend: `components/settings/speakers-section.tsx` (replace `convertFileSrc(path)` playback with this endpoint).

### audio-006: Speaker reassign v2 (scope, whole-voice rename, meeting-segment propagation, segment undo)

* **Kind:** changed. **Size:** M. **Confidence:** observed (fields, SQL, frontend use); inferred (scope semantics details).
* **Evidence:**
  * Request `ReassignSpeakerRequest { audio_chunk_id, new_speaker_name, propagate_similar, scope }` (`0x5153508`).
  * Response `new_speaker_id, new_speaker_name, transcriptions_updated, embeddings_moved, renamed_whole_speaker, previous_speaker_name, old_assignments, old_segment_assignments` (`0x51535a8`, `0x6a726a0`).
  * Only one assignment struct exists: `SpeakerOldAssignment { transcription_id, old_speaker_id }` (`0x51533df`).
  * SQL (`0x5145b40..0x5146015`): `speaker name cannot be empty`; `SELECT COUNT(*) FROM speakers WHERE id = ?1 AND (name IS NULL OR TRIM(name) = '')`; case-insensitive target lookup `… WHERE s.name IS NOT NULL AND TRIM(s.name) <> '' AND LOWER(TRIM(s.name)) = LOWER(?1) AND s.hallucination = 0 GROUP BY s.id ORDER BY COUNT(se.id) DESC LIMIT 1`; `SELECT name FROM speakers WHERE id = ?1`; `UPDATE speakers SET name = ?1 WHERE id = ?2`; `UPDATE speakers SET name = ?1, hallucination = 0 WHERE id = ?2`; meeting-segment match `SELECT mts.id, mts.speaker_id FROM meeting_transcript_segments mts JOIN audio_transcriptions at ON at.audio_chunk_id = ?1 AND at.transcription = mts.transcript AND ABS(julianday(at.timestamp) - julianday(mts.captured_at)) <= 2.0 / 86400.0 WHERE mts.speaker_id IS NULL OR mts.speaker_id <> ?2`; `UPDATE meeting_transcript_segments SET speaker_id = ?1 WHERE id = ?2`.
  * FE: chunk `1491-4c6947e70c72ead9.js` @14511 (bulk relabel sends `propagate_similar:false, scope:"chunk"`) and @254361/@254885/@255204 (single relabel sends `propagate_similar:true, scope:"auto"`; undo either renames back through `/speakers/update` when `previous_speaker_name` is non-null, or posts `{old_assignments, old_segment_assignments}` to `/speakers/undo-reassign`).
* **Behaviour:**
  1. Trim `new_speaker_name` and reject an empty name with 400.
  2. Resolve the target named speaker case- and space-insensitively, skipping hallucinations and preferring the row with the most embeddings. Create one if none exists.
  3. `scope:"auto"` (default when absent; inferred): if the chunk's current speaker is **unnamed** and no named speaker with that name exists, rename that speaker row in place. This labels the whole voice cluster.
     * Set `renamed_whole_speaker=true` and `previous_speaker_name` = the old name (`""` for unnamed).
     * `transcriptions_updated` = row count for that speaker.
     * `old_assignments=[]`.
     * Clear `hallucination` on rename.
     * When a named target already exists, fall through to the chunk path.
  4. `scope:"chunk"`: base behaviour. Move only this chunk's `audio_transcriptions` rows plus one embedding. With `propagate_similar` true, run majority-vote absorption (threshold 0.55, >50% of embeddings; base values) and use audio-004 for the absorb step.
  5. In both paths, also move the matching live `meeting_transcript_segments` rows (same text, `captured_at` within 2 s, speaker differs or is NULL). Record each as `{transcription_id: <segment id>, old_speaker_id}` in `old_segment_assignments`. Use 0 or the original id for NULL.
  6. `previous_speaker_name` is `null` unless a whole-speaker rename happened.
* **Undo:** `POST /speakers/undo-reassign {old_assignments:[…], old_segment_assignments:[…]}` restores `audio_transcriptions` and `meeting_transcript_segments` respectively, re-creating missing speaker ids as nameless rows. Response `{restored}` counts both.
* **Base files:** `crates/screenpipe-engine/src/routes/speakers.rs`, `crates/screenpipe-db/src/db.rs::{reassign_speaker, undo_speaker_reassign}`, frontend speaker-assign popover and timeline bulk relabel.

### audio-007: Speaker identification gating (identify gate, profile-update gate, ambiguity abstain)

* **Kind:** changed. **Size:** M. **Confidence:** observed (log vocabulary, top-2 SQL, modes); inferred (numeric gates).
* **Evidence:**
  * Modes `skip`, `match_only`, `match_and_learn` (`0x49a7455`, `speaker/prepare_segments.rs`).
  * Logs (`0x49a4c3c..0x49a5171`) cover: an empty embedding, after which the transcript is stored with no speaker; storing with no speaker because the duration is under the identity gate; skipping identification under the identify gate; a match line with `id=`, `name=` and `decision=` fields; an ambiguity line with `best_distance=`, `runner_up_distance=` and `margin=` that abstains; declining to create a speaker when nothing matched and the duration is under the profile-update gate; and failures updating the centroid or adding an embedding.
  * Reconciliation has batch variants: a skip under the identity gate, and a match line with `id=` for the batch (`0x49a12ff..0x49a136c`).
  * Top-2 SQL (`0x4984beb..0x4984e9c`): per-speaker `MIN(vec_distance_cosine(se.embedding, vec_f32(?1)))` over `speaker_embeddings` joined to `speakers`, `ORDER BY distance, id LIMIT 2`; centroid fallback `SELECT id, name, metadata, vec_distance_cosine(centroid, vec_f32(?1)) AS distance FROM speakers WHERE centroid IS NOT NULL ORDER BY distance, id LIMIT 2`.
  * Embedding guard: an error rejecting audio that is too short for an embedding (fewer than 400 samples, which is one 25 ms fbank window at 16 kHz), and an fbank shape round-trip failure (`0x6b202d0..0x6b20307`).
* **Behaviour:** classify each segment by voiced duration `d`:
  * `d < identify_gate` → mode `skip`. No lookup; store the transcript with `speaker_id=NULL`.
  * `identify_gate ≤ d < profile_update_gate` → mode `match_only`. Look up, never create, never touch the centroid or embeddings.
  * `d ≥ profile_update_gate` → mode `match_and_learn`. Look up; on match update the centroid and add or rotate an embedding (base `add_embedding_to_speaker`, max 10); on no match create a speaker.

  The lookup returns the best two candidates (embedding table first, centroid fallback).
  * Match when `best < threshold` and `(runner_up − best) ≥ margin` (or there is no runner-up).
  * When both are below threshold and the gap is under the margin, abstain and store NULL.
  * Log the decision string (`matched`, `matched_learned`, `created`, `abstained`, `skipped`; naming is Freenpipe's choice).
  * Refuse to compute embeddings for fewer than 400 samples.
* **Defaults:**
  * threshold 0.55 (observed in base, unchanged).
  * identify gate 1.0 s, profile-update gate 3.0 s, margin 0.05 (all *inferred*). Make them constants with tests.
* **Base files:** `crates/screenpipe-audio/src/transcription/transcription_result.rs` (`get_or_create_speaker_from_embedding`), `speaker/prepare_segments.rs`, `speaker/embedding.rs`, `audio_manager/reconciliation.rs` (batch speaker), `crates/screenpipe-db/src/db.rs::get_speaker_from_embedding` (return up to 2 candidates with distances).

### audio-008: Guarded per-chunk speaker backfill (live-evidence aware)

* **Kind:** changed. **Size:** S. **Confidence:** observed.
* **Evidence:** a single guarded update (`0x4984f11..0x4985952`). CTEs `chunk_rows`, `live_evidence` (joins `meeting_transcript_segments` by `julianday(captured_at)=julianday(timestamp)`, `device_name`, device type, identical text, where `transcription_engine='live'`, with identity `COALESCE(m.session_speaker_id, m.speaker_name)`), and `live_identities`. The statement is `UPDATE audio_transcriptions SET speaker_id = ?2 WHERE audio_chunk_id = ?1 AND speaker_id IS NULL AND …`. Reconciliation logs report a per-chunk backfill (`speaker_id=`), a count of backfilled rows, and a count of live meeting segments mapped to global speaker ids (`0x49a1e50..0x49a2109`). Candidate scan `SELECT ac.id, ac.file_path, ac.timestamp FROM audio_transcriptions at INNER JOIN audio_chunks ac … WHERE at.speaker_id IS NULL AND at.timestamp >= ?1 ORDER BY at.timestamp DESC LIMIT ?2` (`0x4981c88`).
* **Behaviour:** when the background speaker backfill identifies a chunk's speaker, apply it only if every condition holds:
  * (a) no row in the chunk already has a different speaker;
  * (b) no diarization segment of the chunk overlaps or names a different speaker;
  * (c) no diarization run of the chunk has more than one distinct `provider_speaker_label`;
  * (d) if the chunk has `live` rows, all matched live evidence resolves to exactly one non-empty stream-scoped identity whose `speaker_id` (if set) equals the candidate, and every live row has a matching segment.

  Otherwise leave the rows NULL. This replaces the base unconditional `update_transcriptions_speaker` in the backfill path.
* **Base files:** `crates/screenpipe-db/src/db.rs` (new `assign_chunk_speaker_if_unambiguous`), `crates/screenpipe-audio/src/audio_manager/reconciliation.rs` (backfill pass).
* **Depends on:** audio-001.

### audio-009: Stricter automatic self-speaker naming

* **Kind:** changed. **Size:** S. **Confidence:** observed.
* **Evidence:** query `0x6ab0f21..0x6ab1424`. It uses CTE `recent_input` (input-device rows with a speaker, `timestamp >= ?2`), an unnamed non-hallucination speaker, `(SELECT COUNT(DISTINCT speaker_id) FROM recent_input) = 1`, `transcription_engine != 'live'`, `end_time - start_time >= 2.0`, the speaker never seen on an output device, a timestamp outside any `meetings` window, and `HAVING COUNT(DISTINCT audio_chunk_id) >= ?1 LIMIT 1`. Logs record the automatic naming of the dominant input speaker and the start of speaker identification with a `user_name=` field (`0x6ab6798`, `0x6ab6998`).
* **Behaviour:** the background task that names the device owner (setting `userName`) only fires when:
  * the recent window contains exactly one unnamed input speaker;
  * that speaker's segments are at least 2 s long and outside meetings;
  * it never appears on output audio;
  * it is not a live-engine row;
  * it appears in at least N distinct chunks.

  Defaults: lookback window and N are *inferred*. Keep base `MIN_TRANSCRIPTION_COUNT=3` for N; use a 24 h lookback.
* **Base files:** `crates/screenpipe-db/src/db.rs::get_dominant_unnamed_input_speaker` (new signature `(min_chunks, since)`), `crates/screenpipe-engine/src/calendar_speaker_id.rs`.

### audio-010: Audio capture mode (`always` | `meetings-only` | `disabled`) and meeting-session devices

* **Kind:** new. **Size:** L. **Confidence:** observed (setting, enum, logs, UI); inferred (timings).
* **Evidence:**
  * Settings: zod `audioCaptureMode: enum["always","meetings-only","disabled"]` (FE `_next/static/chunks/7451-47878874a9162526.js`); default `"meetings-only"` (FE `378-9f286ebe7c0f0713.js` defaults object); restart-required key set includes `disableAudio` and `audioCaptureMode` (settings page @255447); selector UI @320679 (its help text explains that this mode records audio only during detected meetings and depends on meeting detection being enabled).
  * Rust: `audio_capture_mode`, `meetings-only`, `always` (`0x5179f38`, `0x5256730`); `/health` field `audio_capture_mode` (`0x5256cd6`).
  * Logs (all paraphrased here):
    * `meetings-only` idle state releasing N streams while it waits for a meeting, and an idle cleanup that releases N late streams (`0x49a0ba8..0x49a0c8c`);
    * skipping a device start because `meetings-only` capture is waiting for a meeting (`0x49a3460`);
    * capture waiting for an active meeting; skipping a meeting-session device start with no confirmed meeting; stopping a just-opened meeting-session device because the meeting ended during startup; a meeting that ended mid-open (`0x49a3a00..0x49a3bae`);
    * dropping an audio chunk under `meetings-only` because no meeting is confirmed (`0x49a3e50`);
    * scheduling recovery of the configured audio streams when a meeting is detected (`0x499f870`);
    * `cleanup_stale_device(X)` leaving a session device alone, since only `stop_session_device` may remove it (`0x49a3689`);
    * audio capture disabled, and the audio manager start being skipped for that reason (`0x59fa070`);
    * the meeting detector being turned off because audio capture is disabled (`0x498c2c0`).
  * Tray status texts for screen-only recording with audio waiting on a meeting, and for meeting detection being unavailable (`0x4994c0d..0x4994c47`).
* **Behaviour:**
  * `disabled`: the audio manager never starts. `/audio/start` and device start return the "disabled" error (audio-029). The meeting detector is not started on audio grounds.
  * `always`: base behaviour.
  * `meetings-only`:
    * Streams are opened only between a confirmed `meeting_started` and `meeting_ended`. Devices opened for the meeting are tracked as **session devices**. Only the meeting-end path (`stop_session_device`) may stop them; the stale-device cleaner leaves them alone.
    * On meeting start, schedule recovery that opens the configured devices.
    * On meeting end, release every stream after a grace period, retrying stragglers ("late streams").
    * If a stream finishes opening after the meeting ended, close it at once.
    * Chunks that arrive without a confirmed meeting are dropped and never persisted.
    * The tray shows a state in which audio is waiting on a meeting.
    * If meeting detection is disabled, report that state and show a hint in the UI.
* **Defaults:** default mode `meetings-only` for new installs. Existing installs with no value should behave as `always` (the FE falls back to `"always"` when `audioCaptureMode` is unset). Release grace 30 s (*inferred*).
* **Base files:** `crates/screenpipe-config/src/recording.rs` (`audio_capture_mode: String`, serde `audioCaptureMode`), `crates/screenpipe-engine/src/recording_config.rs`, `crates/screenpipe-audio/src/audio_manager/{builder.rs,manager.rs,device_monitor.rs}`, `crates/screenpipe-engine/src/routes/{audio.rs,health.rs}`, the engine meeting-event subscription, Tauri tray (`src-tauri/src/tray.rs`), settings UI `components/settings/recording-settings.tsx`.
* **Depends on:** meeting detection (meetings dimension), audio-026, audio-028.

### audio-011: Bluetooth microphone meeting gate (`alwaysRecordBluetoothMic`)

* **Kind:** new. **Size:** M. **Confidence:** observed (setting, gate fields, probes); inferred (Windows probe detail; UTF-16 literals are invisible to the ASCII dump).
* **Evidence:**
  * Gate log `bluetooth microphone gate: device=, blocked=, in_meeting=, explicitly_selected=, always_override=, combo_headset=` (`0x49a3c00..0x49a3c6c`).
  * A log skipping a Bluetooth mic start outside a meeting while `always_record_bluetooth_mic` is off (`0x49a34b3..0x49a350e`); a `[DEVICE_RECOVERY]` log skipping a default-input change because the new default is held by the Bluetooth meeting gate (`0x499fd48`); `bluetooth mic gate: failed to start` (`0x49a0cf0`).
  * Probes: `Bluetooth transport probe failed for '…'` (`0x6afa4e8`); a combo-headset probe failure that falls back to treating the device as a combo (`0x6afa950`); PnP walk strings `PKEY_Device_ContainerId`, `SWD\MMDEVAPI\`, `CM_Locate_DevNodeW`, `CM_Get_Parent`, `CM_Get_Device_IDW` (`0x6afa134..0x6afa2a1`).
  * Name heuristic list (base already has it in `device_detection.rs`).
  * Setting default `alwaysRecordBluetoothMic:false`; UI toggle @351965 (its help text explains the default: a Bluetooth mic is opened only while a meeting runs, because keeping it open lowers the headset's playback quality); a selection hint that explicitly choosing a Bluetooth mic lets it record continuously (FE settings page message hash `08df666c97c80fe9`).
* **Behaviour:** an input device is gated when all hold:
  1. it is Bluetooth;
  2. it is a combo headset (same container has a render endpoint, so opening the mic forces the hands-free profile);
  3. the user did not select it explicitly in `audioDevices` while `useSystemDefaultAudio=false`;
  4. `alwaysRecordBluetoothMic` is off;
  5. no meeting is active.

  A gated device is not opened, and default-input switches to it are skipped. When a meeting starts the gate lifts; when it ends the device is stopped.

  Windows detection:
  * Read the endpoint's container id (`PKEY_Device_ContainerId`).
  * Walk the PnP parent chain from the MMDevice instance (`SWD\MMDEVAPI\{…}`) with `CM_Locate_DevNodeW` / `CM_Get_Parent` / `CM_Get_Device_IDW`.
  * Treat an enumerator prefix of `BTHENUM`, `BTHHFENUM` or `BTHLEDEVICE` as Bluetooth (*inferred*).
  * A combo headset is a Bluetooth input whose container id matches an active render endpoint.
  * If a probe fails, assume combo (fail safe toward gating).
  * Fall back to the existing name heuristics when the PnP walk fails.
* **Base files:** `crates/screenpipe-audio/src/core/{device.rs,device_detection.rs}` (new `windows_bt_probe`), `audio_manager/{manager.rs,device_monitor.rs}`, `crates/screenpipe-config/src/recording.rs` (`always_record_bluetooth_mic`), settings UI.
* **Depends on:** audio-010 (meeting state plumbing).

### audio-012: Software echo cancellation (AEC3 + NS + AGC2) and `aecMode`

* **Kind:** new. **Size:** L. **Confidence:** observed (crate, pipeline logs, setting mapping); inferred (frame sizes and drift thresholds).
* **Evidence:**
  * Crates `sonora`, `sonora-aec3`, `sonora-agc2`, `sonora-ns` (git checkout, `0x9f43740`).
  * `core/aec.rs` logs: init with aggressive noise suppression and AGC2 (`0x6b23b18`); `process_render` and `process_capture` errors (including a bypass variant of the latter); a failure to set the stream delay; and a large-negative-drift warning that discards N stale mic samples to keep the speaker reference aligned (`0x6b24050..0x6b24138`).
  * Recorder logs (`0x512c678..0x512c9fb`): startup of the always-on software cleaning stage (AEC3 + NS + AGC2) for a mic at 16 kHz; a dynamic subscription to a speaker stream (with its rate) as the reference; mic and speaker resampling errors; a mic-channel lag count; a diagnostics line with the fields `drift=`, `mic_buf=`, `speaker_buf=`, `bypass=`, `processed=`, `aligned=`, `bypass_frames=`, `speaker_underflow=`, `dropped=`; a failed final mic flush; and recording stopped because a device disconnected.
  * Settings: `AecMode` enum `off|screenpipe|macos|windows` (`0x5a43078`; zod enum in FE `7451-…js`). Mapping in FE `378-…js` @71191: a mode is only valid on its own platform (`screenpipe` is valid everywhere, `macos` only on macOS, `windows` only on Windows, otherwise `off`). Flags are derived as `screenpipeAecEnabled = mode=="screenpipe"`, `macosInputVpioEnabled = mode=="macos"`, `windowsInputAecEnabled = mode=="windows"`. An enterprise policy `aec-mode-control {defaultValue, forceDisabled}` can force `off`. Default `aecMode:"off"`.
* **Behaviour:** when `screenpipeAecEnabled`:
  1. Run each **input** stream through a pipeline: resample mic to 16 kHz mono; subscribe to the currently captured **output** stream (system loopback) as the far-end reference, resampled to 16 kHz.
  2. Feed reference frames to `process_render` and mic frames to `process_capture` in 10 ms frames (160 samples; inferred).
  3. Estimate and set the stream delay from buffer occupancy. On large negative drift, drop stale mic samples so the reference stays aligned.
  4. When no reference is available (speaker underflow) or processing errors, bypass AEC but keep NS+AGC2 if possible. Count bypass frames.
  5. Periodically log diagnostics. The cleaned mic audio replaces the raw mic samples sent to the chunker, VAD, STT and the meeting live tap.
  6. Flush on stop.

  `windows` mode keeps the base WASAPI endpoint-AEC path. `macos` keeps VPIO.
* **Base files:** new `crates/screenpipe-audio/src/core/aec.rs`; `core/run_record_and_transcribe.rs` (insert the stage); `core/stream.rs` (expose an output-stream broadcast to subscribe to); `audio_manager/{builder.rs,manager.rs}` (option `screenpipe_aec_enabled`); `crates/screenpipe-config/src/recording.rs` (`screenpipe_aec_enabled`, `aec_mode`); `recording_config.rs`; settings UI (one echo-cancellation selector that writes all four keys); `apps/screenpipe-app-tauri/lib/hooks/use-settings.tsx`.
* **Notes:** sonora is a third-party git dependency. Verify its licence before adding it. Alternatives are `webrtc-audio-processing` bindings or a port of AEC3 under a compatible licence. Keep NS/AGC behind the same flag.

### audio-013: Windows WASAPI loopback engine (whole-endpoint and per-process)

* **Kind:** new. **Size:** L. **Confidence:** observed (target kinds, API calls, errors, build gate); inferred (exact activation parameters).
* **Evidence:**
  * `core/process_tap/windows.rs` strings (`0x6af8430..0x6af9040`): target labels `process-tree:`, `exclude-process-tree:`, `default-render-endpoint`; exclusions file constants; `ms-teams.exe`. Log and error texts cover: COM initialisation failing for the loopback; rebuilding the loopback when the exclusion target or the default render endpoint changes; packet-drain and event-wait failures; the loopback target exiting; a failed wait on the target pid that stops the tap; being unable to open the target pid to watch for its exit; failures setting the event handle, starting or initialising the loopback client, and obtaining `IAudioCaptureClient`; `GetBuffer`, `ReleaseBuffer` and `GetNextPacketSize` failures; and a timeout while starting capture.
  * `0x6af97c0..0x6af9c41`: errors and status texts for a missing or invalid target pid; an undeterminable Windows build (per-PID loopback then assumed unavailable); availability status, unavailable below build 20348 and available with the build number; an activation timeout for a pid; `ActivateAudioInterfaceAsync` failing for a pid; an error explaining that per-process taps need build 20348+ and that older builds cannot isolate a root pid without recording the whole system mix; and failures getting the default render endpoint, creating `MMDeviceEnumerator`, and activating that endpoint's `IAudioClient`.
  * `0x6af4817..0x6af48be`: the label `wasapi-local`, plus errors for a missing activation operation, a process-loopback activation that yields no `IAudioClient`, an activated interface of the wrong type, a failed process-loopback activation, and a failed `GetActivateResult`.
  * `0x6af4610..0x6af46e2`: a meeting-tap compatibility-fallback log for requested pids that resolve to several independent roots, and a variant that also takes in the ancestor processes of the Teams desktop app (reporting both the Teams pid and the root pid).
* **Behaviour:** an event-driven WASAPI capture client with three target kinds:
  * `default-render-endpoint`: classic loopback of the default render device. Rebuild when the default render endpoint changes.
  * `process-tree:<pid>`: `ActivateAudioInterfaceAsync` on the virtual process-loopback device with include-target-process-tree mode for the root pid. Watch the pid handle and stop when it exits.
  * `exclude-process-tree:<pid>`: same activation with exclude mode. It captures everything except that process tree and is used for exclusions (audio-014).

  Rules:
  * Per-process modes need Windows build ≥ 20348. On older builds report "unavailable" so callers fall back to whole-endpoint capture.
  * Activation and start each have timeouts.
  * Run the capture loop on a dedicated COM-initialized thread: wait on the event handle, drain packets with `GetNextPacketSize`/`GetBuffer`/`ReleaseBuffer`, and convert to f32 mono at the mix rate.
  * Feed the existing `AudioStream` broadcast so downstream code (chunking, VAD, live tap, AEC reference) is unchanged.
  * When several requested pids resolve to more than one independent root, fall back to whole-endpoint capture. Special case: when the Teams desktop process (`ms-teams.exe`) is the meeting app, include its ancestry so the right root is chosen.
* **Interop:** device naming for streams opened this way is `"<label> (output)"`. The meeting variant is named `Meeting Tap (output)` (audio-016). Whole-endpoint loopback keeps the base device names so `audioDevices` settings stay valid.
* **Base files:** new `crates/screenpipe-audio/src/core/process_tap/windows.rs` (move the macOS file to `process_tap/macos.rs` with a `mod.rs` facade); `core/stream.rs` (Windows output devices use the new client instead of the cpal loopback, behind a feature flag with cpal fallback); `Cargo.toml` `windows` features (`Win32_Media_Audio`, `Win32_System_Threading`, `Win32_System_Com_StructuredStorage`, `Win32_System_Variant`). Follow `TESTING.md` audio checklist.

### audio-014: Windows audio exclusions (exe-based, one excluded app)

* **Kind:** changed. **Size:** M. **Confidence:** observed.
* **Evidence:**
  * The engine reads `SCREENPIPE_AUDIO_EXCLUSIONS_PATH` / `.screenpipe/audio-exclusions.json` in the Windows module (`0x6af8497`, `0x6af9b39` `excluded_apps`, `bundle_id`).
  * Tauri Windows metadata reader: `… is not a Windows executable`, fallback name `Application`, icon as `data:image/png;base64,` (`0x498df7a..0x498dfc1`); git dep `windows-icons` (`out\audio\git_checkouts.txt`).
  * FE settings page @300748/@301571: on Windows the file picker filters `exe` with default path `C:\Program Files`; `checkCoreaudioProcessTapAvailable` gates the panel; a panel note limits Windows to a single excluded app (hash `db8f9357d4442a76`).
  * Commands `read_audio_exclusions`, `write_audio_exclusions`, `read_app_bundle_metadata`, `check_coreaudio_process_tap_available` (FE `8001-00ed120106af2449.js` @32540/@50677; `0x5177a68`).
* **Behaviour:**
  * The file shape is unchanged: `{"excluded_apps":[{"bundle_id","name","icon"}]}`. On Windows, `bundle_id` holds the executable file name (lower-cased, e.g. `spotify.exe`; casing inferred).
  * The engine polls the file mtime and the running process list. When an excluded exe is running, the system-audio stream is rebuilt as `exclude-process-tree:<root pid>` (audio-013). Only the first running excluded app is honoured, because WASAPI excludes a single tree. When it exits or the list changes, rebuild (log "exclusion target changed").
  * `read_app_bundle_metadata(path)` on Windows:
    * Validate that the path is a PE executable.
    * Return `{bundleId: <file name>, name: <FileDescription or ProductName, else file stem, else "Application">, icon: <32 px PNG data URL>}`.
  * `check_coreaudio_process_tap_available` returns true on Windows build ≥ 20348.
* **Base files:** `apps/screenpipe-app-tauri/src-tauri/src/audio_exclusions.rs` (Windows branch), `commands.rs` (process-tap availability on Windows), `crates/screenpipe-audio/src/core/process_tap/windows.rs`, settings UI (`components/settings/recording-settings.tsx`: enable the panel on Windows, `.exe` filter, single-app note).
* **Depends on:** audio-013.

### audio-015: Windows audio-session snapshot (`meeting_processes`)

* **Kind:** new. **Size:** M. **Confidence:** observed (APIs, labels, RDP note); inferred (snapshot struct).
* **Evidence:**
  * `meeting_processes.rs` (`0x6af6f18..0x6af7460`): source labels `wasapi:`, `wasapi-session:`; failures getting `IAudioSessionEnumerator`, activating `IAudioSessionManager2`, creating the device enumerator, and counting or enumerating audio endpoints; a per-endpoint skip log in the audio-process snapshot; an RDP/VDI warning (with no client-side proxy, enumerating devices locally does not reveal the remote user's microphone, which can weaken meeting detection).
  * `core/meeting_audio/windows.rs` (`0x6af9c94..0x6af9e10`): a `meeting_audio:` log for a failed Windows input resolution for a pid.
  * Consumers: engine `meeting_watcher/audio_process/*` (meetings dimension) and piggyback `mic-holder set` (`0x49a25ef`, `0x49a2995`).
* **Behaviour:** `snapshot()` enumerates active **capture** and **render** endpoints. For each, it activates `IAudioSessionManager2`, walks `IAudioSessionEnumerator`, and records sessions whose state is active (or recently active) with their process id and endpoint name. It returns:
  * `capture_sessions: [{pid, endpoint_name, state}]` (the "mic holders");
  * `render_sessions: [{pid, endpoint_name, state}]`;
  * `supported: bool`;
  * `remote_session: bool`.

  `resolve_meeting_input(pid)` returns the capture endpoint(s) a given process (or its tree) is using. Endpoints that fail are skipped, not fatal. Under RDP (`GetSystemMetrics(SM_REMOTESESSION)`), set `remote_session` and warn once.
* **Base files:** new `crates/screenpipe-audio/src/meeting_processes.rs`, new `core/meeting_audio/windows.rs`. Export them for the engine meeting watcher.

### audio-016: Meeting piggyback ("smart recording": `experimentalMeetingPiggyback`)

* **Kind:** new. **Size:** XL. **Confidence:** observed (states, outcomes, logs, settings); inferred (strike and retry constants, state-machine details).
* **Evidence:**
  * `audio_manager/meeting_piggyback.rs` (`0x49a239b..0x49a2a5d`, `0x6b1af48..0x6b1c4e5`): disengage reasons `not configured`, `invalid device name`, `user paused`, `audio capture disabled`, `meeting ended`; `failed to restart resumed device`; transition reasons `start completed without a running stream`, `flag_disabled`, `meeting_unpublished`, `meeting_pid_missing`, `tap_unavailable`, `inactive`; a mic-selection log with `source=`, `devices=` and `bluetooth_allowed=true` fields; mic sources `windows`, `configured`, `system_default`, `process (input)`; tap device `Meeting Tap (output)`; tap start failures counted as strikes; a rebuild of the tap when the `mic-holder set` changes, and rebuild failures over the new pid set, also counted as strikes; a message for an unsupported OS (isolation requires macOS 14.4 or later, or Windows build 20348 or later; the meeting falls back to configured capture); a disengage log with fields `reason=`, `meeting_seen=`, `pid_count=`, `tap_available=`, `tap_was_streaming=`, `tap_started_count=`, `tap_strikes=`; `engaged_to_disengaged`; a log that capture is following the meeting mic; `piggyback_mic_capture_failed` when the meeting mic cannot be opened; a log that slows mic retries after N failed attempts over a period; a log that suspends the other mics once the meeting mic delivers audio; a mic-holder enumeration failure (with `supported=`) that keeps the previously adopted pid set; a tap that registered but never streamed being torn down; `piggyback_tap_gave_up` after N per-process tap failures in one meeting, staying on stable capture (with `bundle_id=`); outcomes `no_pid`, `stable_fallback`, `partial_piggyback`, `full_piggyback`, `unavailable`; `[MEETING_PIGGYBACK]` compatibility-outcome logs that report whether the configured output was kept and is running, whether none was running (in which case normal output follow stays on), and whether restoring the configured output succeeded, was skipped or failed.
  * Skip `skipping start of piggyback-suspended device` (`0x49a33a8`); logs for not starting a normal audio device because smart recording owns capture for the meeting, and for stopping a normal device that was mid-open when smart recording engaged (`0x49a3409..0x49a3598`).
  * Telemetry structs (`0x59f9980`): `PiggybackMeetingSummary {outcome, meeting_seconds, tap_streaming_seconds, tap_started_count, tap_strikes, tap_gave_up, mic_resolved_devices, mic_session_started, mic_capture_failed, meeting_app_bundle_id, pid_known, manual, captured_no_audio, platform, os_version}`, `PiggybackCaptureTransition {transition, reason, meeting_seen, tap_available, tap_was_streaming}`; event names `piggyback_meeting_summary`, `piggyback_capture_transition` (`0x49a27d4`, `0x6a145d8`); engine `piggyback_telemetry.rs` forwards them to analytics.
  * Settings: `experimentalMeetingPiggyback` defaults from remote-control policy `smartRecording.defaultEnabled` (FE `378-…js`); preference key `smartRecording`.
* **Behaviour** (when the flag is on and a meeting with a known app pid is published):
  1. **Output side.** Start a per-process loopback over the meeting app's process tree as the device `Meeting Tap (output)` (audio-013 `process-tree:`). While it streams, configured output devices may stay running (compatibility outcome logged) or be suspended. Rebuild when the mic-holder pid set changes. Each failure counts a strike. After the strike limit, give up for this meeting (`piggyback_tap_gave_up`) and stay on stable capture (configured or system-default output). If the tap registers but delivers no frames within a timeout, tear it down and count a strike.
  2. **Input side.** Resolve which capture endpoint the meeting app holds (audio-015 `resolve_meeting_input`). Precedence for the mic source: `process` (the meeting app's own endpoint), then `configured`, then `system_default`; `windows` is the Windows communications default. Open that mic as a session device even if it is Bluetooth (`bluetooth_allowed=true`). Once it delivers audio, suspend other mics for the meeting to avoid duplicate transcripts. If it fails to open, retry with backoff, slowing retries after N attempts, and emit `audio_capture_health` `mic_capture_failed` (audio-018).
  3. **End.** On meeting end, or on any disengage reason, stop the tap and session mics, resume suspended devices, restore configured outputs, and emit transition and summary records. Outcome classification:
     * `full_piggyback`: tap and meeting mic both streamed.
     * `partial_piggyback`: only one of them streamed.
     * `stable_fallback`: the tap gave up or was unavailable.
     * `no_pid`: the meeting app pid was unknown.
     * `unavailable`: the OS is unsupported.
  4. The normal device monitor must not start or stop devices that piggyback owns.
* **Telemetry:** Freenpipe must not send these records anywhere. Log them at debug level and optionally expose them through `/audio/metrics`.
* **Defaults (inferred):** tap strike limit 3; start-streaming timeout 10 s; mic retry backoff 2 s doubling to 30 s after 5 attempts.
* **Base files:** new `crates/screenpipe-audio/src/audio_manager/{meeting_piggyback.rs,piggyback_listeners.rs}`; `audio_manager/{manager.rs,device_monitor.rs}` (ownership checks); `crates/screenpipe-config/src/recording.rs` (`experimental_meeting_piggyback`); engine meeting events wiring; settings UI toggle.
* **Depends on:** audio-013, audio-015, audio-010, audio-018.

### audio-017: Windows output follow (`[AUDIO_FOLLOW]`) and speaker watchdog

* **Kind:** new. **Size:** M. **Confidence:** observed (logs); inferred (intervals).
* **Evidence:**
  * `audio_manager/windows_output_follow.rs` (`0x49a2ab3..0x49a2dd3`): `[AUDIO_FOLLOW]` logs for starting a loopback on an endpoint that is playing audio but not being captured, a failed loopback start with its retry delay, and stopping the loopback after the endpoint has been idle for some seconds.
  * `[SPEAKER_WATCHDOG]` logs for cycling output capture when, during a meeting, an endpoint is rendering but no speaker chunks have arrived for some seconds; for notifying the user when speaker capture is still silent after the restart (naming the rendering and the captured endpoints); `speaker capture recovered`; `restart of X failed`.
  * `failed to query Windows render endpoint activity` (`0x6afa840`).
* **Behaviour:**
  * **Follow (Windows only, active when `useSystemDefaultAudio` is on):**
    * Poll the activity of every active render endpoint (`IAudioMeterInformation` peak or active sessions).
    * When an endpoint other than the captured ones is audibly rendering, start a whole-endpoint loopback on it (audio-013) as an extra output device.
    * Stop it after it has been idle for the idle timeout.
    * Retry failed starts with backoff.
    * Disabled while piggyback owns output capture, unless its compatibility outcome says normal follow remains enabled.
  * **Watchdog (during meetings):**
    * If any endpoint is rendering but no output-device chunks were produced within the window, restart output capture once.
    * If it is still silent after the restart, emit `audio_capture_health` `speaker_silent` with `rendering_endpoints` and `captured_outputs` (audio-018).
    * Emit `recovered` when chunks resume.
* **Defaults (inferred):** poll interval 2 s; idle stop 30 s; watchdog window 90 s; post-restart window 60 s.
* **Base files:** new `crates/screenpipe-audio/src/audio_manager/windows_output_follow.rs`; `device_monitor.rs` (spawn and coordinate).
* **Depends on:** audio-013, audio-018.

### audio-018: Audio capture health, device status and fallback events (engine → Tauri → UI)

* **Kind:** new. **Size:** M. **Confidence:** observed.
* **Evidence:**
  * Engine event structs: `AudioCaptureHealthEvent {rendering_endpoints, captured_outputs, capturing_inputs, …}` and `AudioCaptureHealthState {speaker_silent, recovered, mic_capture_failed}` (`0x522e3cc`); `AudioDeviceFallbackEvent {pinned_device, fallback_device}` with `AudioDeviceFallbackState {engaged, cleared, unavailable}` and `AudioDeviceStatusChangedEvent {device_name, is_user_disabled, …}` (`0x6189e78`).
  * Engine event names `audio_capture_health_speaker_silent`, `audio_capture_health_recovered`, `audio_capture_health_mic_capture_failed`, `audio_device_fallback_engaged|cleared|unavailable`, `audio_device_status_changed` (`0x49acce0`, `0x52d7f18`).
  * Tauri `engine_events/audio_device.rs`: mapping to `audio-device-status-changed`, `audio-device-fallback-unavailable`, `audio-device-fallback-cleared`, `audio-device-fallback-engaged` (`0x5220d26`); OS notifications for fallback engaged, restored (`audio_device_fallback_restored:`) and unavailable; placeholder `pinned_device__no_input__` (`0x52206d0..0x522099a`).
  * Tauri `engine_events/audio_health.rs`: `audio-capture-health-mic-capture-failed`, `audio-capture-health-recovered`, `audio-capture-health-speaker-silent` (`0x528703d`); notifications with action `restart_recording` (`0x5286970..0x5286ce1`).
  * FE home page @823834 refreshes the device list on `audio-device-status-changed` (and polls every 10 s).
* **Data shapes:**
  * `audio_capture_health_*` payload: `{state: "speaker_silent"|"recovered"|"mic_capture_failed", rendering_endpoints: [string], captured_outputs: [string], capturing_inputs: [string], device?: string, reason?: string, meeting_id?: i64}`. Optional fields are inferred.
  * `audio_device_status_changed`: `{device_name, is_running, is_user_disabled}`.
  * `audio_device_fallback_*` keeps the base `{state, pinned_device, fallback_device}`. For `unavailable`, `fallback_device` is empty and `pinned_device` may be `pinned_device__no_input__` when no input exists.
* **Behaviour:**
  * The engine emits fallback `unavailable` when a pinned mic disappears past grace and no other input exists (base already logs this).
  * `restored` is the Tauri-side notification on `cleared`.
  * Device status events fire on every start, stop, pause or resume through the API or the monitor.
  * Tauri forwards all of these to the webview and raises native notifications. Respect `notificationPrefs` (`captureStalls`) and the meeting state.
* **Base files:** `crates/screenpipe-events/src/custom_events/audio_devices.rs` (+ new `audio_health.rs`), `crates/screenpipe-audio/src/audio_manager/{device_monitor.rs,manager.rs}`, `crates/screenpipe-engine/src/routes/audio.rs`, `apps/screenpipe-app-tauri/src-tauri/src/engine_events/{audio_device.rs, audio_health.rs (new), mod.rs}`, home page device list hook.

### audio-019: Whisper on Vulkan (Windows) with GPU→CPU recovery

* **Kind:** changed. **Size:** M. **Confidence:** observed (Vulkan backend compiled, logs); inferred (recovery state machine).
* **Evidence:**
  * `whisper context: GPU unavailable; using CPU fallback`, and a log naming the GPU backend asked for per platform (Metal for macOS, Vulkan for Windows) (`0x6b0e070`).
  * Inference recovery (`0x49a5500..0x49a5a46`) logs cover: an inference failure that rebuilds the session on CPU and reruns the same audio; `CPU state installed`; GPU inference failures whose CPU recovery cannot proceed (recovery model not cached, recovery failed, or recovery configuration failed); a CPU inference that finished using a shared recovery state another session had installed; a CPU inference that finished after GPU recovery; failure to create the shared CPU state; and a session that lost its state before CPU recovery.
  * State creation (`0x6af62f0..0x6af6b38`): GPU state creation failing and the shared context being rebuilt on CPU; and a CPU recovery already running after a GPU failure.
  * ggml-vulkan symbols and `GGML_VK_*` env names; shipped `vulkan-1.dll` and `VulkanRT-License.txt`.
* **Behaviour:**
  * At context creation, query ggml backends. If no GPU device exists, create the context with `use_gpu(false)`.
  * When GPU state creation or a `full()` call fails:
    * Build one **shared** CPU context (process-wide, created once under a lock; other sessions wait or reuse it).
    * Install a CPU state in the failing session and retry the same audio once.
    * Afterwards the engine stays on CPU until restart.
  * If the CPU model file is missing, return the original error and keep the chunk pending for reconciliation.
* **Base files:** `crates/screenpipe-audio/Cargo.toml` (`vulkan = ["whisper-rs/vulkan", "audiopipe?/vulkan-ggml"]`), `transcription/whisper/{model.rs,batch.rs}`, `transcription/engine.rs` (session recovery), app `Cargo.toml` and Windows release feature list (`vulkan`), `tauri.windows.conf.json` resources (`vulkan-1.dll` + licence; optional, since the GPU driver normally provides the loader).

### audio-020: DirectML for Parakeet / Qwen3-ASR (adapter selection, `SCREENPIPE_DIRECTML`, CPU recovery)

* **Kind:** changed. **Size:** M. **Confidence:** observed.
* **Evidence:**
  * `transcription/parakeet_windows.rs` (`0x6b13d30..0x6b13f5f`): the label `adapter ordinal`; `D3D12CreateDevice` failing with an HRESULT, or succeeding without returning a device; failed capability checks; no hardware adapter that is non-software, non-remote and D3D12-compatible; a selection log giving the chosen adapter's ordinal, name, dedicated video memory in MiB and `UMA=`; DXGI adapter enumeration failing and CPU being used; `SCREENPIPE_DIRECTML` being explicitly disabled; the literals `true` and `SCREENPIPE_DIRECTML`.
  * audiopipe parakeet (`0x6b44e60..0x6b46162`): DirectML session creation; DirectML init failure with fallback to CPU; DirectML sessions recycled after N inferences to prevent slowdown in long sessions; a failed recycle that falls back to CPU; CPU inference completing after recovery from a DirectML init failure.
  * Qwen3 (`0x6b46ce0..0x6b486cc`): logs for attempting the DirectML EP for a model, a model loaded with DirectML, and failed DirectML sessions being released before CPU recovery.
  * `DirectML.dll` shipped.
* **Behaviour:**
  * Before loading ORT models on Windows: if env `SCREENPIPE_DIRECTML` is set to anything other than `true`/`1`, use CPU.
  * Otherwise enumerate DXGI adapters and skip software and remote adapters. Call `D3D12CreateDevice` (feature level 11_0) and run the capability checks. Choose the first passing adapter, preferring discrete or high dedicated memory, and pass its ordinal as the DirectML `device_id`. Log the adapter name, VRAM and UMA. If none qualifies, use CPU.
  * On EP init failure, build CPU sessions.
  * On an inference failure under DirectML, release the DML sessions, initialise CPU sessions and retry once.
  * Recycle DML sessions every N inferences (inferred N = 500) to avoid slowdown.
* **Base files:** new `crates/screenpipe-audio/src/transcription/parakeet_windows.rs`; `transcription/engine.rs` (Parakeet/Qwen3 constructors); audiopipe upgrade (new revision with DirectML + recovery) or local equivalents through `ort` EP configuration; `Cargo.toml` `windows` features (`Win32_Graphics_Dxgi`, `Win32_Graphics_Direct3D12`, `Win32_Graphics_Direct3D`); bundle `DirectML.dll` and `onnxruntime_providers_shared.dll`; bump `ort` to `=2.0.0-rc.12` (inferred from `ort-2.0.0-rc.12` paths).
* **Notes:** audiopipe is an external git dependency. Confirm the newer revision's licence before upgrading.

### audio-021: ONNX Runtime init watchdog and audio model health

* **Kind:** new. **Size:** S. **Confidence:** observed (statuses, components); inferred (timeout).
* **Evidence:**
  * `ort-watchdog:`, `ok`, `init_failed`, `init_timed_out`, `runtime speaker_embedding speaker_segmentation ready models` (`0x6b1ddaa..0x6b1de7f`), plus texts for a timeout after N seconds (attributed to a probable ONNX Runtime init hang on the host), a watchdog worker that exited with no result, and a failure to spawn the watchdog thread.
  * Core health diagnostics `health-startup.json` with components `transcription`, `vad`, `speaker_segmentation`, `speaker_embedding`, `dependencies`, `runtime` and states `ok|init_failed|init_timed_out|not_required|missing|degraded`; env probes `SCREENPIPE_FFMPEG_PATH`, `ORT_DYLIB_PATH`, `HF_HOME`, `HF_HUB_CACHE` (`0xa0e4df0..0xa0e5288`).
  * Startup deferral: a log about postponing a restart to avoid an onnxruntime teardown race (`0x49988da`).
* **Behaviour:**
  * Run ORT environment creation and each audio model session build (segmentation, embedding, silero, Parakeet/Qwen ONNX) on a dedicated thread. Wait with a timeout. If it does not finish, mark the component `init_timed_out` and continue without it; do not block the audio manager.
  * Panics or errors set `init_failed`.
  * Publish a `models` map `{runtime, speaker_embedding, speaker_segmentation, vad, transcription} → status` to the health diagnostics writer (health dimension owns the file format) and to `/health` `dependencies`.
* **Defaults:** timeout 60 s (*inferred*).
* **Base files:** `crates/screenpipe-audio/src/speaker/mod.rs` (wrap the existing panic guard), `segmentation/segmentation_manager.rs`, `vad/silero.rs`, `transcription/engine.rs`; health dimension for `health-startup.json`.

### audio-022: Bundled model and runtime resolution

* **Kind:** changed. **Size:** S. **Confidence:** observed (shipped files); inferred (lookup order).
* **Evidence:**
  * Install dir `<screenpipe-install>/` contains `models\segmentation-3.0.onnx`, `models\wespeaker_en_voxceleb_CAM++.onnx`, `vad\silero_vad_v5.onnx`, `onnxruntime.dll`, `onnxruntime_providers_shared.dll`, `DirectML.dll`, `libopenblas.dll`, `vulkan-1.dll`, `VulkanRT-License.txt`.
  * Loader names `segmentation-3.0.onnx`, `wespeaker_en_voxceleb_CAM++.onnx` (`0x49a75d1`) and `silero_vad_v5.onnx(.downloading)` (`0x49a6698`).
  * Download fallback URLs still point at the upstream repository (`0x6b1fe98`).
* **Behaviour:** resolve each model in this order:
  1. a directory next to the executable (`<exe_dir>\models\…`, `<exe_dir>\vad\…`);
  2. the Tauri resource dir;
  3. the existing cache (`<cache>\screenpipe\models`, `<cache>\screenpipe\vad`);
  4. download (audio-032).

  Copy-on-first-use into the cache is optional.
* **Base files:** `crates/screenpipe-audio/src/speaker/models.rs`, `vad/silero.rs`, `models/download.rs`; `apps/screenpipe-app-tauri/src-tauri/tauri.windows.conf.json` resources (base repo already holds `crates/screenpipe-audio/models/pyannote/*.onnx`; add a silero copy).

### audio-023: Reconciliation durability (pending-chunk markers, staged merged audio)

* **Kind:** changed. **Size:** M. **Confidence:** observed (struct fields, directories, logs); inferred (caps).
* **Evidence:**
  * `struct PendingChunk with 3 elements`, `struct PendingTranscription with 15 elements`, `struct PendingChunkTranscription with 7 elements` (`0x52bbda8..0x52bbec8`).
  * PendingTranscription field list `file_path timestamp transcription audio_chunk_id engine device is_input duration_secs speaker_id diarization_provider diarization_segments secondary_chunk_ids merged_audio_path merged_audio_sha256 secondary_file_paths` (`0x6aff7ff`); PendingChunkTranscription fields include `text device_name start_time end_time` (`0x6aff7ff`, `0x6221608`).
  * Directories `pending-transcriptions`, `pending-chunks`; file stems `chunk-`, `.chunk-`, `json.tmp` (`0x6afeaa8..0x6aff698`, `0x6b02608`).
  * Logs (paraphrased):
    * pending-chunk markers (`0x49a13c0..0x49a16af`): the marker cap being reached so a chunk is not queued for recovery (a suspected write outage); a marker written; an orphaned chunk recovered; a re-insert that keeps failing; a marker dropped because its audio file is gone; recovery paused after N consecutive DB errors (pool probably saturated); a count of orphaned chunks recovered from disk; an orphaned chunk queued for recovery;
    * transcription journal (`0x49a1768..0x49a1a99`): transcription deferred because there is no durable result directory, or because pending results are unavailable; a failure to journal a completed transcription; a DB write failure for a chunk whose result went to the pending cache; failures to verify or to stage merged audio (the originals are then left alone);
    * merged audio that is missing or changed, so the source chunks are kept (`0x6aff7ff`);
    * a chunk+transcription insert that failed after N attempts and was persisted for reconciliation, and a per-device insert that was rolled back together with its batch and is being retried (`0x49a4e34..0x49a4fe0`); the code `collateral_batch_abort` (`0x6b0cda8`);
    * the audio manager refusing to start transcription because it has no output path, and so no durable recovery (`0x49a4518`);
    * an audio processing outcome that could not be saved, leaving the audio pending for recovery (`0x49a52a0`).
* **Behaviour:**
  1. **Capture-time marker.** When the early-persist `audio_chunks` insert fails after its retries, write `<data_dir>/pending-chunks/chunk-<ts>-<device>.json`. The `PendingChunk {file_path, timestamp, device_name}` triple is inferred. Write to `.tmp` then rename. Do nothing if the marker count is already at the cap.
  2. **Startup and sweep recovery.** Read markers and re-insert the chunk rows. Drop a marker when its audio file is gone. Pause recovery after K consecutive DB errors.
  3. **Batch transcription journal.** Before replacing primary audio with merged audio:
     * Write the merged file to a temp path and compute its sha256.
     * Write `pending-transcriptions/<chunk>.json` (`PendingTranscription` v2 adds `merged_audio_path`, `merged_audio_sha256`, `secondary_file_paths`, with `#[serde(default)]` for old files).
     * Commit DB rows. Only then swap the merged file into place and delete secondaries.
     * On replay, verify the sha256. If it is missing or different, keep the source chunks.
     * `PendingChunkTranscription` (text, device_name, start_time, end_time, plus the per-chunk id, speaker and timestamp) is the per-secondary-chunk row variant.
  4. Refuse to start background transcription when there is no data dir.
  5. When the sweep hits the per-sweep chunk cap, continue draining immediately instead of waiting a full interval.
* **Defaults:** marker cap 10 000, consecutive-DB-error pause 5 (both *inferred*); base `RECONCILIATION_CHUNKS_PER_SWEEP=50` is kept.
* **Base files:** `crates/screenpipe-audio/src/audio_manager/{reconciliation.rs,manager.rs}`, `transcription/transcription_result.rs`.

### audio-024: Network STT robustness (OpenAI-compatible retries; Deepgram rate limits)

* **Kind:** changed. **Size:** S. **Confidence:** observed (logs and error tokens); inferred (backoff numbers).
* **Evidence:**
  * OpenAI-compatible (`0x49a4b04..0x49a4c26`): retry logs for an HTTP 429 rate limit and for a general request failure, each with the attempt number and the delay; a final failure log with `audio=`, `timeout=` and `attempt=` fields. Retryable error tokens (substring-matched against transport errors): `timed out`, `timeout`, `connection reset`, `connection closed`, `broken pipe`, `connection refused`, `error sending request` (`0x6b036cb`). MP3 encoder strings `audio/mpeg` (`0x6b02fa1..0x6b030d8`).
  * Deepgram (`0x49a5c86..0x49a5d26`): retry logs for a rate limit and for a request failure, each with the attempt number; a log that the rate limit persisted after N attempts; `reset_in` (`0x6b03a38`).
* **Behaviour:**
  * **OpenAI-compatible:** up to 3 attempts.
    * HTTP 429: honour `Retry-After` if present, else exponential backoff (2 s, 4 s).
    * Transport errors matching the tokens above: back off 1 s, then 2 s.
    * The per-request timeout scales with audio duration: `max(OPENAI_COMPATIBLE_TIMEOUT_SECS, audio_secs × 0.5)`, capped at 600 s (inferred).
  * **Deepgram direct:** on 429, retry after `Retry-After` or the JSON `reset_in` seconds, up to 3 attempts. After that, surface a rate-limit error and leave the chunk pending.
* **Base files:** `crates/screenpipe-audio/src/transcription/openai_compatible/batch.rs`, `transcription/deepgram/batch.rs`.

### audio-025: Meeting retranscribe response additions and summary refresh

* **Kind:** changed. **Size:** S. **Confidence:** observed.
* **Evidence:** `MeetingRetranscribeResponse` field run `chunks_found batches_processed seconds_processed replaced_segments transcript_updated_at summary_refresh_event_emitted` (`0x6207a80`); event `meeting_summary_refresh_requested` with reasons `summary_generation`, `transcript_replaced`, and failure log (`0x620814b`); progress id prefix `meeting-retranscribe:` (`0x6207fba`). Other strings (2 h cap, "stop the meeting first", invalid range, choose an engine) equal the base.
* **Behaviour:** after a meeting retranscribe replaces segments:
  * set `transcript_updated_at` (RFC 3339, now);
  * if the meeting has a note or summary pipeline configured, publish `meeting_summary_refresh_requested {meeting_id, reason:"transcript_replaced"}` on the event bus and set `summary_refresh_event_emitted=true`.

  The meetings and pipes dimensions own the consumer.
* **Base files:** `crates/screenpipe-engine/src/routes/retranscribe.rs`.

### audio-026: Audio device routes: disabled mode and status events

* **Kind:** changed. **Size:** S. **Confidence:** observed.
* **Evidence:** `routes/audio.rs` strings (`0x59fb5d0..0x59fb779`): `Audio capture is disabled in settings` (exact string, client-visible), `failed to publish audio device status change`. Manager (`0x59f9be9..0x59f9cef`): `user paused audio device`, `user resumed audio device`, a log that restarts the pipeline to resume a device after the manager had stopped because capture was disabled, and a message that a device did not start because capture is gated or its stream is unavailable. Route set is unchanged (`0x5257ca8`); `/audio/device/status` stays unauthenticated (FE `9665-…js` @11889).
* **Behaviour:**
  * `POST /audio/start` and `POST /audio/device/start` return 409 or 400 `{success:false, message:"Audio capture is disabled in settings"}` (exact string, client-visible) when `audioCaptureMode=disabled` or `disableAudio`.
  * Device start on a stopped manager restarts the pipeline. If the stream does not come up because of meetings-only or Bluetooth gating, the response says capture is gated.
  * Every successful start or stop publishes `audio_device_status_changed` (audio-018).
* **Base files:** `crates/screenpipe-engine/src/routes/audio.rs`, `crates/screenpipe-audio/src/audio_manager/manager.rs`.

### audio-027: Metrics and health additions

* **Kind:** changed. **Size:** S. **Confidence:** observed.
* **Evidence:** `AudioPipelineMetrics` snapshot field list adds `last_stream_timeout_at` (`0x62061ba`). `/health` field run includes `audio_capture_mode` (`0x5256cd6`, `0x499ae90`). `/ws/metrics` JSON pointers `/audio/audio_level_rms`, `/audio/mic_capture_active` read by the Windows native overlay (`0x498fbe8`, `src\native_overlay_win.rs`), with the producer at `0x512e850`.
* **Behaviour:**
  * Record the timestamp (epoch secs) of the last stream receive timeout.
  * Report `audio_capture_mode` in `/health` (string; `disabled` when audio is off).
  * Add boolean `audio.mic_capture_active` to `/ws/metrics`: true when at least one input device is running and produced frames in the last 5 s (window inferred).
* **Base files:** `crates/screenpipe-audio/src/metrics.rs`, `crates/screenpipe-engine/src/routes/{health.rs,websocket.rs}`, `server.rs` (`/audio/metrics` JSON).

### audio-028: Audio settings surface and defaults

* **Kind:** changed. **Size:** M. **Confidence:** observed.
* **Evidence:** Rust field order `0x5a42a38`; FE defaults (`378-9f286ebe7c0f0713.js` defaults object near `audioChunkDuration:30`); zod schema (`7451-47878874a9162526.js` @56901); settings mapping (`378-…js` @71191, @74219); macOS-only Qwen3 guard (settings page @309270: on macOS `qwen3-asr` resets to `whisper-large-v3-turbo-quantized`; the engine list hides Qwen3 on macOS and labels Parakeet experimental there, @324055).
* **New keys** (camelCase in `store.bin` settings and `RecordingSettings`; snake_case in Rust):

| key | type | default | notes |
|---|---|---|---|
| `audioCaptureMode` | `"always"|"meetings-only"|"disabled"` | `"meetings-only"` (new installs) | restart required; audio-010 |
| `experimentalMeetingPiggyback` | bool | Freenpipe: `false` (target takes it from remote policy `smartRecording.defaultEnabled`) | audio-016 |
| `alwaysRecordBluetoothMic` | bool | `false` | audio-011 |
| `screenpipeAecEnabled` | bool | `false` | derived from `aecMode` |
| `aecMode` | `"off"|"screenpipe"|"macos"|"windows"` | `"off"` | platform-validated; drives the three AEC booleans |

* **Changed defaults vs base:**
  * `filterMusic` `true` (base `false`);
  * `experimentalCoreaudioSystemAudio` `true` (macOS only; base forced it `false` via a migration);
  * `autoUpdate` `true`;
  * `usePiiRemoval` `true` (out of audio scope).
* **Restart-required keys:** `disableAudio`, `audioCaptureMode` (FE set at @255447; it restarts the whole engine instead of only the capture session).
* **Unchanged keys:** `audioTranscriptionEngine` default `whisper-large-v3-turbo-quantized`; `meetingLiveTranscriptionEnabled` `true`; `meetingLiveTranscriptionProvider` `"selected-engine"` (values `selected-engine|screenpipe-cloud|disabled|deepgram-live`); `audioDevices ["default"]`; `useSystemDefaultAudio true`; `audioChunkDuration 30`; `transcriptionMode "batch"`; `languages []`; `ignoredMeetingApps []`; `recordWhileLocked false`; `windowsInputAecEnabled false`; `macosInputVpioEnabled false`; `vocabularyWords`; `openaiCompatible*`; `batchMaxDurationSecs`; `userName`.
* **Base files:** `crates/screenpipe-config/src/{recording.rs,defaults.rs}`, `crates/screenpipe-engine/src/{recording_config.rs,cli/mod.rs}` (optionally add `--audio-capture-mode`, `--aec-mode`, `--meeting-piggyback`, `--always-record-bluetooth-mic`; the target app embeds no clap help, so names are Freenpipe's choice), `apps/screenpipe-app-tauri/src-tauri/src/store.rs`, `lib/hooks/use-settings.tsx`, `lib/utils/validation.ts`, `lib/utils/tauri.ts` (regenerate bindings), settings UI.

### audio-029: ee-stub: remote-control policy for audio controls

* **Kind:** ee-stub. **Size:** S. **Confidence:** observed.
* **Evidence:** FE `378-…js` @71191: `remoteControlPolicy {schemaVersion:1, boolean:{semanticContext, coreAudioSystemAudio, smartRecording:{defaultEnabled, forceDisabled}}, aecMode:{defaultValue, forceDisabled}}`, controls named `aec-mode-control` etc.; `remoteControlPreferences.aecMode`.
* **Behaviour:** Freenpipe has no enterprise policy server. Stub the policy as "no policy": `forceDisabled=false`, defaults from local settings. Keep the preference fields readable so a local admin could later force-disable AEC or piggyback through `store.bin`.
* **Base files:** frontend settings hooks; no engine change.

### audio-030: decloud: screenpipe-cloud batch transcription

* **Kind:** decloud. **Size:** M. **Confidence:** observed.
* **Evidence:**
  * Target: `https://api.screenpipe.com/v1/listen` + `Bearer` (`0x6b13f73`); account-state codes `account_not_in_good_standing`, `not in good standing`, `daily_cost_limit_exceeded`, `transcription_capacity_paused` (`0x6b024a8`); `Screenpipe hosted transcription paused (` (`0x49a5c58`); a reconciliation log that pauses the sweep until the next cycle when the transcription API rejects the account (`0x49a1c5a`); firewall hints naming `api.screenpipe.com` (`0x49a5ee0..0x49a6401`).
  * Frontend pro-defaults migration `_proCloudAudioDefaultsAppliedV2` flips logged-in users to `screenpipe-cloud` for both batch and live (`378-…js` @15094); engine fallback reasons `notLoggedIn` / `missingDeepgramKey` (settings @255447).
  * The base carries the same proxy: `DeepgramTranscriptionConfig::screenpipe_cloud`, `recording_config.rs:293`, `store.rs` engine resolution, `main.rs:1136`, frontend `use-settings.tsx` ~L490-505.
* **Behaviour (Freenpipe):**
  * Remove the `screenpipe-cloud` endpoint. Keep accepting the string `"screenpipe-cloud"` as an `audioTranscriptionEngine` value so existing screenpipe settings load. Resolve it at runtime to Deepgram with the user's own key when `deepgramApiKey` is set, otherwise to the local default (`whisper-large-v3-turbo-quantized`). Never construct an `api.screenpipe.com` URL.
  * Drop the pro-cloud defaults migration.
  * Treat the cloud account codes as generic non-retryable 4xx.
  * Rewrite user-facing network error text so it names the configured host, not screenpipe's.
* **Base files:** `crates/screenpipe-audio/src/transcription/deepgram/{mod.rs,batch.rs}`, `crates/screenpipe-engine/src/recording_config.rs`, `apps/screenpipe-app-tauri/src-tauri/src/{store.rs,main.rs}`, `lib/hooks/use-settings.tsx`, settings engine selector (hide the cloud option).

### audio-031: decloud: live meeting transcription via screenpipe-cloud realtime

* **Kind:** decloud. **Size:** S. **Confidence:** observed.
* **Evidence:**
  * `wss://api.screenpipe.com/v1/realtime`, `/v1/realtime`, env `SCREENPIPE_MEETING_REALTIME_URL`, `SCREENPIPE_MEETING_CLOUD_TOKEN`, `SCREENPIPE_MEETING_STREAMING_PROVIDER`, `SCREENPIPE_MEETING_DEEPGRAM_LIVE_URL` (default `wss://api.deepgram.com/v1/listen`), `SCREENPIPE_MEETING_TRANSCRIPTION_MODEL` (default `nova-3`), `SCREENPIPE_MEETING_TRANSCRIPTION_LANGUAGE`, `SCREENPIPE_MEETING_LOCAL_SPEAKER_NAME`, `SCREENPIPE_MEETING_DEEPGRAM_API_KEY` / `DEEPGRAM_API_KEY` (`0x6af42a0..0x6af4498`, `0x6af8320`); provider values `disabled|selected-engine|screenpipe-cloud|deepgram-live` (`0x6af46f1`).
  * Live parameters equal the base (`endpointing=800`, `utterance_end_ms=1500`, `vad_events`, `diarize`, `language=multi` default, `keyterm`).
  * The base has the identical config (`meeting_streaming/config.rs`).
* **Behaviour:**
  * Keep the `deepgram-live` (BYO key) and `selected-engine` providers.
  * Map a stored `screenpipe-cloud` provider to `deepgram-live` if a key exists, else to `selected-engine`.
  * Remove the default realtime URL. `SCREENPIPE_MEETING_REALTIME_URL` stays honoured only as an explicit user override for a self-hosted compatible endpoint.
* **Base files:** `crates/screenpipe-audio/src/meeting_streaming/{config.rs,controller.rs}`, frontend live-notes provider selector.

### audio-032: decloud: model download origins

* **Kind:** decloud. **Size:** S. **Confidence:** observed.
* **Evidence:** speaker model fallback URLs point at `https://github.com/screenpipe/screenpipe/raw/refs/heads/main/crates/screenpipe-audio/models/pyannote/…` (`0x6b1fe98`; the base has the same, `speaker/models.rs:95-101`). Qwen3 ONNX from `louis030195/qwen3-asr-0.6b-onnx` (`0x6b47c28`).
* **Behaviour:**
  * Ship the pyannote ONNX models from this repo (they exist at `crates/screenpipe-audio/models/pyannote/`) and resolve them locally (audio-022).
  * If a download fallback is kept, point it at a Freenpipe-controlled release asset or the original upstream model publishers, never the post-relicense screenpipe `main` branch.
  * Silero stays at `snakers4/silero-vad`; whisper at `ggerganov/whisper.cpp`; Parakeet at `istupakov/*`.
* **Base files:** `crates/screenpipe-audio/src/speaker/models.rs`, `models/download.rs`.

### audio-033: decloud: piggyback and audio telemetry

* **Kind:** decloud. **Size:** S. **Confidence:** observed.
* **Evidence:** `piggyback_meeting_summary`, `piggyback_capture_transition`, `audio_capture_health_mic_capture_failed` forwarded by engine `piggyback_telemetry.rs` (`0x6a145d8..0x6a146b0`); settings telemetry fields `setting_audio_capture_mode`, `setting_audio_device_count`, `setting_audio_transcription_engine`, … (`0x5179b20`).
* **Behaviour:** these PostHog events must not be sent. Keep the structs for local debug logging only, and drop the analytics forwarder entirely.
* **Base files:** `crates/screenpipe-engine/src/analytics.rs` (no-op), new piggyback module (log only).

## 3. Unchanged areas (checked)

* Diarization schema and insert path; `speaker_identity_evidence` with `approved = (source == 'manual')` (`0x4981aee..0x4981bfa` equals base `db.rs:1789-1868`).
* `audio_transcriptions_new` migrations (text equal).
* Search diarization columns (`diarization_mode/speaker_label/provider/source/confidence`, 0.05 s tolerance) (`0x62182c0..0x6219274`).
* VAD engines and the silero download flow.
* Source-buffer gap fill; startup grace; lock rebuild; DRM output stop.
* Calendar-assisted constraints; seeding the embedding manager; speaker dedup and alias cleanup.
* Meeting streaming lifecycle (reattach, 15 min inactivity, 2 h cap, stall events, live coverage marking, mirroring into `audio_transcriptions`).
* `/v1/audio/transcriptions` upload endpoint (fields `file`, `engine`, temp `upload.bin`).
* Engine enum and string aliases.
* OpenAI-compatible defaults (`whisper-1`, `http://127.0.0.1:8080`).
* Reconciliation backlog routes.

## 4. Open questions

1. Numeric gates for speaker identification (identify gate, profile-update gate, ambiguity margin) are not in strings. Should they be calibrated with `screenpipe-audio-eval` before shipping?
2. `scope` may have values beyond `auto|chunk` (for example `speaker`). Only `auto` and `chunk` were observed in the frontend.
3. Exact `session_speaker_id` / `item_id` string formats from Deepgram live. They are opaque, but compatibility with screenpipe-written rows only matters for display grouping.
4. Bluetooth detection on Windows. The enumerator prefixes (`BTHENUM`, `BTHHFENUM`, `BTHLEDEVICE`) are UTF-16 literals invisible to the ASCII string dump. Confirm with a native REA pass if the coordinator opens the binary.
5. The licences of the `sonora` (AEC3 port) and newer `audiopipe` revisions must be verified before taking them as dependencies.
6. The `/speakers/sample` length cap value. Choose one consistent with batch-mode segment lengths.
7. Whether `meetings-only` should become Freenpipe's default for existing installs (the target defaults new installs only). The decision touches `VISION.md` "stability over features".
8. `PendingChunk` (3 fields) and `PendingChunkTranscription` (7 fields): the full field lists are partly ambiguous from packed strings.

## Verification

Adversarial verification pass (2026-10-15). Method:
* every cited offset was re-read in `app_strings.tsv` (`tools\audv_off.py`, `tools\audv_grep.py`);
* every identifier was grepped in the BASE (`<repo>`) (`tools\audv_basehas.py` plus direct reads of `db.rs`, `reconciliation.rs`, `manager.rs`, `device_manager.rs`, `audio_exclusions.rs`, `permissions.rs`, `Cargo.toml`, `release-app.yml`, `tauri.windows.conf.json`);
* the frontend claims were re-read at the cited character offsets;
* the other specs were checked for duplicates.

REA application traces for `smart-recording-control` (`ev_f26ba2d1c3cea1201a503b1ea8ef59a2ac71257b55a52619866aca242af21dd4`) and `include_samples` (`ev_896fc78501f0792dbb1ed33e9d2603eb0095f4459a5aa583eb177c1a7ee247d0`) both returned `no-match`, because the parent graph `ev_57f23857…` covers this bundle only partially. The frontend facts below therefore come from direct chunk reads.

Sweep output listing target audio strings that are absent from the base: `tools\out\v_audio_new.txt` (939 lines).

### Verdicts

| id | verdict | note |
|---|---|---|
| audio-001 | confirmed | Migration DDL at `0xa2ae075..0xa2ae17a`. The 10-field delta and 11-field final structs and the 11-column `INSERT OR IGNORE` are all present. Base `events.rs:154/167` has 9 and 10 fields and no `stream_id` / `session_speaker_id` anywhere. |
| audio-002 | confirmed | Light NOCASE query (`0x5147a16`) and paginated `NamedSpeakers` (`0x5147b96`) verified. Base `search_speakers(name_prefix)` has no limit, offset or NOCASE, and `SearchSpeakersRequest` has only `name`. FE `3308` @37302 sends `limit` 5/20 and `include_samples=false` with a 3 s timeout. The route surface overlaps api-013; the DB behaviour is owned here. |
| audio-003 | confirmed | Four CTE copies carry `'audio_chunk_id', rap.audio_chunk_id` and the `NOT IN ('', '.')` filter. Neither occurs in the base. |
| audio-004 | partially_in_base | The undo half is already in the base: `undo_speaker_reassign` (`db.rs:8429`) already uses `SELECT EXISTS…` + `INSERT INTO speakers (id, name) VALUES (?, '')` to recreate missing ids as nameless rows. The genuinely new parts are: (a) the repoint covers 5 tables (base `merge_speakers` `db.rs:6051` and reassign phase 3 touch only `audio_transcriptions` + `speaker_embeddings`); (b) per-table reference counts gate the `DELETE FROM speakers` (base deletes unconditionally); (c) undo also targets `meeting_transcript_segments` through the templated `UPDATE <table> … WHERE id = ?2`. |
| audio-005 | confirmed | Route at `0x5258848` and every handler string verified. FE settings @615879 builds `/speakers/sample/<id>?start=&end=`, and @624714 caps the duration at 3 s. Absent from the base. Overlaps api-013 (route summary only). |
| audio-006 | confirmed | Request `scope`, the response fields, and the trimmed / case-insensitive / meeting-segment SQL (`0x5145b40..0x5146015`) verified. The base request has no `scope` and the base lookup is `s.name = ?`. FE 1491 @14511 (`scope:"chunk"`) and @254250 (`scope:"auto"`, undo through `/speakers/update` or `/speakers/undo-reassign`) verified. |
| audio-007 | confirmed | Modes, gate logs, top-2 SQL (`LIMIT 2`) and the 400-sample guard verified. Base `get_speaker_from_embedding` (`db.rs:2002`) returns the single nearest match under 0.55, and the base always learns. |
| audio-008 | confirmed | Guarded UPDATE `0x4984f11..0x4985952` verified. Base backfill calls the unconditional `update_transcriptions_speaker` (`db.rs:8141`, `reconciliation.rs:1213`). The cited log lines (`backfilled speaker_id=`, `mapped N live meeting segment(s)`) are base-identical, so the SQL is the real evidence. |
| audio-009 | confirmed | SQL `0x6ab0f21..0x6ab1424` verified against base `get_dominant_unnamed_input_speaker` (`db.rs:2295`, a plain count). Precision fix: the target requires the recent input window to contain exactly ONE distinct speaker id of any kind, named or not, and that speaker must be unnamed. The spec's wording "exactly one unnamed input speaker" is looser than that. |
| audio-010 | confirmed | Every cited string and the FE enum, default and restart set were verified. Nothing in the base mentions `audioCaptureMode`. Two corrections. (1) The disabled-mode error belongs to audio-026, not audio-029. (2) "Existing installs behave as `always`" is uncertain. The FE defaults object sets `"meetings-only"`, and store loads normally merge defaults, so the `"always"` fallback may only apply in the UI. Extra evidence: a log that ignores microphone permission events while audio capture is disabled (`0x514a980`). |
| audio-011 | confirmed | Gate log fields, the Bluetooth skip and gate messages, and the PnP-walk strings were verified. FE settings @351965 has the toggle. Nothing in the base matches. |
| audio-012 | confirmed | sonora checkout path, `core/aec.rs` logs, the recorder diagnostics and the `AecMode` enum were verified. The FE zod enum `["off","screenpipe","macos","windows"]` is at 7451 @56082. Nothing in the base matches. Correction: the `aec-mode-control` policy is a PostHog feature flag, not an enterprise policy (see audio-029). |
| audio-013 | confirmed | The full WASAPI loopback / process-loopback string set was verified. The base has no `ActivateAudioInterfaceAsync`, `process-tree` or `default-render-endpoint`, and base `core/process_tap.rs` is macOS-only. |
| audio-014 | confirmed | A changed item over a macOS-only base. The base already has `audio_exclusions.rs` with the file, the env override and the read/write commands, plus `check_coreaudio_process_tap_available` (`permissions.rs:682`, which returns false off macOS). The Windows engine path (`0x6af8497`, `0x6af9b39`), the `.exe` metadata reader (`0x498df7a`) and the FE `exe` filter (@301423) are new. |
| audio-015 | duplicate | Duplicates meetings_memory-030. That item already specifies the same `meeting_processes.rs` WASAPI session snapshot (IAudioSessionManager2, `wasapi:` / `wasapi-session:`, RDP/VDI flag) from the same evidence `0x6af6f18..0x6af7460`, and its verifier noted the overlap. Only the per-pid input resolution (`core/meeting_audio/windows.rs`, `0x6af9e10`) is unique; fold it into audio-016. Implement the snapshot once, in `screenpipe-audio`. |
| audio-016 | confirmed | Piggyback strings, outcomes, telemetry structs and device-ownership guards were verified. Nothing in the base mentions piggyback. Corrections: the target has NO settings-page toggle. `experimentalMeetingPiggyback` appears only in the defaults object (378 @18225) and the remote-control applier (layout chunk). Its shipped default is ON (`smart-recording-control` `shippedDefault:!0`, 378 @66916). A Freenpipe toggle is a local addition. |
| audio-017 | confirmed | `[AUDIO_FOLLOW]` / `[SPEAKER_WATCHDOG]` strings (`0x49a2bb0..0x49a2dd3`) and `0x6afa840` verified. Absent from the base. |
| audio-018 | confirmed | Event structs and names, plus the Tauri kebab mappings (`0x5220d26`, `0x528703d`), verified. FE home @823834 has the listener and the 10 s poll. The base already has fallback `engaged` / `cleared` end to end (`custom_events/audio_devices.rs`, `engine_events/audio_device.rs`). New: `unavailable`, `audio_device_status_changed`, and all `audio_capture_health_*`. |
| audio-019 | confirmed | Whisper GPU→CPU recovery strings and `GGML_VK_*` verified; `vulkan-1.dll` is shipped. Base: `model.rs:70` already calls `use_gpu(true)` and logs "Vulkan on Windows". But the base `vulkan` feature maps only to `audiopipe?/vulkan-ggml` (not `whisper-rs/vulkan`), and the Windows release (`release-app.yml:181`) does not enable it. So whisper-on-Vulkan and the recovery path are the delta. |
| audio-020 | confirmed | The `parakeet_windows.rs` adapter picker, `SCREENPIPE_DIRECTML`, DML recycle/recovery, `ort-2.0.0-rc.12` and `audiopipe … b099a2e` were verified. Note that the base Windows release already builds with `--features directml` (audiopipe DML EP), pins `ort =2.0.0-rc.10` and audiopipe `2fd5c36`, and bundles only the CPU `onnxruntime.dll`. New: adapter selection, the env switch, the recovery logic, `DirectML.dll` + `onnxruntime_providers_shared.dll`. |
| audio-021 | confirmed | `ort-watchdog` statuses and the `health-startup.json` component vocabulary verified; neither is in the base. Correction: `0x49988da` (the restart deferral that avoids an onnxruntime teardown race, `#3622`) is base code (`src-tauri/src/updates.rs:239`), not delta evidence. Gap: no spec owns `health-startup.json` (see audio-M06). |
| audio-022 | confirmed | The install dir really ships `models\segmentation-3.0.onnx`, `models\wespeaker_en_voxceleb_CAM++.onnx`, `vad\silero_vad_v5.onnx`, `DirectML.dll`, `vulkan-1.dll` and `libopenblas.dll`; base `tauri.windows.conf.json` bundles none of the models. Caveat: the loader strings are base-identical and no exe-dir lookup string exists, so the lookup order stays *inferred*. |
| audio-023 | confirmed | The 15-field `PendingTranscription` (base `reconciliation.rs:50` has 12), the `pending-chunks` markers, sha256 staging and the durable-dir refusal are new. The base already has `pending-transcriptions/`, the pending-cache fallback log for a failed DB write, and the consecutive-DB-error pause in the batch sweep, so those cited lines are not delta. |
| audio-024 | confirmed | 429 and backoff strings verified; the base OpenAI-compatible and Deepgram batch clients have no retry loop. Note: the `reset_in` JSON hint is the screenpipe gateway's 429 shape (base `packages/ai-gateway/src/utils/rate-limiter.ts`, `core/agents/pi.rs`). Direct Deepgram should rely on `Retry-After`. |
| audio-025 | duplicate | Duplicates api-039 (same two response fields and the `meeting_summary_refresh_requested` emit), and overlaps meetings_memory-027. Evidence `0x6207a80` / `0x620814b` is valid. |
| audio-026 | confirmed | The disabled-in-settings error `Audio capture is disabled in settings` (exact string, client-visible), the status-change publish failure, and the gated-start message are new. `user paused audio device` is base (`manager.rs:545`). |
| audio-027 | confirmed | `last_stream_timeout_at` (`0x62061ba`) is new and unique to this item. `audio_capture_mode` in `/health` overlaps api-033, and `mic_capture_active` overlaps api-015 and shell.md:453; implement those once. |
| audio-028 | confirmed | The field order (`0x52305f0`) and FE defaults verified: `filterMusic:!0` and `experimentalCoreaudioSystemAudio:!0`, where base `use-settings.tsx:596/601` has false; `aecMode:"off"`; `alwaysRecordBluetoothMic:!1`. The key list is mirrored in shell-001 (which defers to this dimension) and frontend.md. Correction: the target's effective default for `experimentalMeetingPiggyback` is true (shipped default of the `smart-recording-control` flag). |
| audio-029 | duplicate | Duplicates cloud-033, and the item is misclassified. `remoteControlPolicy` is not an enterprise policy: 378 @66916 shows `{flagKey:"smart-recording-control", settingKey:"experimentalMeetingPiggyback", shippedDefault:!0}` and @70617 `aec-mode-control`. Both are PostHog feature-flag payloads (cloud-033). Treat it as decloud (delete the applier and use the shipped defaults locally), not as an ee-stub. |
| audio-030 | duplicate | Duplicates cloud-017, which covers `/v1/listen`, the account codes and the BYO Deepgram / local fallback. The base already carries the same proxy and the `_proCloudAudioDefaultsAppliedV2` migration (`use-settings.tsx:488-504, 842`). Evidence is valid. |
| audio-031 | duplicate | Duplicates cloud-017, which covers the realtime URL, the env names and the selected-engine fallback. Base `meeting_streaming/config.rs:10-12,77` is identical, so there is no version delta here. |
| audio-032 | duplicate | Duplicates cloud-024, which covers pyannote `raw/refs/heads/main` → Freenpipe or bundling. Base `speaker/models.rs:95/101` is identical. |
| audio-033 | duplicate | Duplicates shell-031 (piggyback forwarder), cloud-040 (`piggyback_telemetry.rs`) and cloud-010 (PostHog / `setting_*`). |

Tally: 25 confirmed, 1 partially_in_base, 7 duplicate, 0 unsupported. Every cited offset resolved to the claimed text.

### Missed items (independent sweep)

* **audio-M01 (changed, S) Strict meeting-segment → global speaker mapping.**
  * Base `backfill_meeting_segment_speakers` (`db.rs:9599`) gives a segment the speaker of the nearest identified row in the same direction within 15 s. The target replaces this with a single set-based UPDATE (`0x4983d71..0x498445a`).
  * A segment matches only mirrored `live` rows with the identical capture instant, device name, direction and text.
  * Segments with an identical-text twin at the same instant and device are skipped.
  * Exactly one distinct `speaker_id` is required.
  * The matching uses `INDEXED BY idx_audio_transcriptions_timestamp` with a ±window range and `LIMIT ?3`.
* **audio-M02 (changed, M) Audio device lifecycle hardening.**
  * `device_manager.rs` / `core/stream.rs` (`0x49a6aa9..0x49a6df5`, `0x6afa526..0x6afa58b`, `0x6b22238`, `0x6b226c4`).
    * `stop_device` tears down idempotently even when the device is already marked stopped, so the stream and any tap are released. Base `device_manager.rs:119` returns "already stopped" instead.
    * Operations are serialized per device (a new request is refused while the previous one is still completing).
    * Operations and teardown have timeouts (logs cover a teardown timeout, a worker that did not exit, and native teardown or startup still holding ownership).
    * Streams reporting 0 channels or a 0 Hz rate are refused.
    * `0x80070005` is classified as access denied.
  * `device_monitor.rs`:
    * the default-input switch backs off when the new default is not enumerated yet (`0x499fc12..0x499fd15`);
    * the user-facing notice that a device has disappeared and capture stays stopped until it is reconnected comes with retry backoff (`0x49a07e7..0x49a08c4`);
    * the `[PINNED_FALLBACK]` total-loss / user-disabled-default states (`0x49a0d38..0x49a0eeb`) feed audio-018's `unavailable`.
* **audio-M03 (changed, S) Shared transcription engine and failure-tolerant receiver.**
  * Reuse the shared engine across handler restarts (a log reports the receiver handler picking up the shared engine).
  * Build engines in a panic-isolated factory task (logs cover a factory panic, and a build task that ended without publishing its result).
  * When session creation fails, keep receiving and leave audio pending for reconciliation (`0x49a3cb0..0x49a3d4e`, `0x49a4678`, `0x49a52a0`).
  * Base `manager.rs:736-751` builds a new engine on every start and aborts the handler on error.
* **audio-M04 (changed, S) Cross-device dedup scoped and fail-open.**
  * The target query (`0x4981468..0x498154f`) anchors the window to the chunk's capture time range (`?1..?2`) instead of wall-clock now.
  * It excludes the same device and direction (`device != ?3 OR is_input_device != ?4`) and reads `start_time` / `end_time`.
  * On lookup failure it keeps the transcription (`0x49818f8`), with a read-pool stall retry (`0x498187c`).
  * The transcript handler has an explicit `duplicate` outcome (`0x49a5198`).
  * Base `has_similar_recent_transcription` (`db.rs:1458`) compares against all devices, including the same one, over the last 45 s of wall-clock time. That breaks dedup for reconciled old chunks and can drop repeated same-device phrases.
* **audio-M05 (changed, S) Timeline audio rows: `is_live`, per-segment ids.**
  * The timeline audio query adds `transcription_engine = 'live' AS is_live`.
  * Live meeting segments come back with `is_live=1` and `audio_chunk_id = -mts.id`, where base `db.rs:5407` uses a constant `-1`.
  * New guard: when the timeline audio offsets are invalid, a log notes that the transcript is kept at the chunk timestamp (`0x4982278..0x4982bc8`, duplicate copy at `0x49dc42d`).
  * Interop: timeline consumers that key on `audio_chunk_id` need the unique negative ids.
* **audio-M06 (new, M; owner unassigned) Startup health diagnostics file `health-startup.json`.**
  * `crates/screenpipe-core/src/health_diagnostics.rs` (`0xa0e4df0..0xa0e5288`) holds:
    * per-component status for `transcription`, `vad`, `speaker_segmentation`, `speaker_embedding`, `dependencies` and `runtime`;
    * preflight checks `data_dir_writable` / `path_has_ffmpeg` / `env_overrides`;
    * a `last_failed_start` record and phase tracking;
    * a process snapshot with `audio_encodes_*` counters.
  * audio-021 defers the file format to a "health dimension", but no spec (shell / api / vision) defines it. The coordinator should assign it (probably shell or api).
