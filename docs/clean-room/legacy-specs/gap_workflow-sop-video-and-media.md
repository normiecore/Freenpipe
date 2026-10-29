# Gap spec: Workflows desktop services (narrated SOP video, captured-moment replay, workflow-ready notifications)

Dimension key: `gap_workflow-sop-video-and-media`. Target: screenpipe v2.7.104 (Windows build, static evidence only).
Base: Freenpipe @ upstream 892199f. Clean-room: behaviour below is paraphrased. Interop identifiers (commands, events,
file and directory names, JSON fields, env vars, CLI flags, ffmpeg option names and numeric parameters) are quoted
exactly. UI and error copy is described, not reproduced, except for a few short substrings that the frontend matches on
(marked **interop substring**).

This spec closes three holes in the other specs:
- `agents_pipes.md:467` assigns the `video-sop` tools to a "workflow-video dimension" that does not exist. **This file is
  that dimension.**
- `frontend.md:207-214, 786-794` lists the command names and a one-line UI mention but no backend. Correction:
  `release_workflow_recording` takes `{url}`, not zero arguments (chunk 8001 binding `releaseWorkflowRecording(r)` sends
  `{url:r}`).
- `cloud.md` omits a vendor dependency: SOP narration calls a hosted speech-generation endpoint (`/tts`) on the account
  gateway. Item -narration-decloud below handles it.

Evidence notation: `S:0x...` = hex file offset in `app_strings.tsv`. `R:0x...` = raw printable run read directly from the
exe at that offset (runs shorter than 6 characters, missing from the TSV). `FE:` = frontend asset path (+char offset).
REA evidence: frontend parent `ev_57f23857f31e9207a9f71e1d612412e78f9c5dc0d29af8c68f97372786b6216d`; module views
`ev_5c9396a820d7503917a63f666e400c41bbaeb79ac004af8ba91a059ac8e5d973` (`4656-37ad501298baf0a7.js`) and
`ev_141a2f48f0327b47d2ac645a25e391263717eb30016d63b0b4e41bf6a125a13b` (`755-b4c485e5a5bfbc14.js`); feature traces for
`create_workflow_video` (`ev_21e1d5e794592958ccedd8a8174ba6626c0aa8211c49af8cb9db681bae981f0b`) and
`workflow-review-ready` (`ev_21bf10a2229278fdc38e2bd1c0fcecf0e2e0d4b02489f5d6e262b22e1157b60a`) returned *no-match*
because the REA graph is partial. Everything below was therefore confirmed by grepping the chunks directly.

---

## 1. Base vs target inventory

| Area | BASE (892199f) | TARGET (v2.7.104) | Delta id (prefix `gap_workflow-sop-video-and-media-`) |
|---|---|---|---|
| SOP video renderer (core) | none. `screenpipe-core/src/ffmpeg.rs` only does ffmpeg discovery and `ffmpeg_cmd[_async]` (CREATE_NO_WINDOW). No drawtext/zoompan/atempo/mov_text anywhere (grep: 0 hits) | `crates/screenpipe-core/src/workflow_video.rs` (S:0x524f5f8, S:0x512a698, S:0xa0ec3d8): types `Scene`/`Focus`, validation, ffmpeg preflight, per-scene parts, WebVTT captions, concat + mux, 20-min/size/time limits, progress, cancel | `core-renderer` |
| Narration speech | none | hosted `/tts` on account gateway, header `x-screenpipe-narration-profile`, bearer `SCREENPIPE_API_KEY`, quota/entitlement error codes (R:0x524fbea, S:0x524fcb0, S:0xa0ebe90) | `narration-decloud` |
| App video job model | none (closest analogue: `meeting_export.rs` `export_recording`) | `src\workflow_video.rs` (S:0x498bcc8, S:0x49f0400): commands `create_workflow_video`, `cancel_workflow_video`, `discard_workflow_video`; dir `workflow-videos`; temp `.render-<id>`; events `workflow-video-<id>`; single-flight; preview quota | `app-video-jobs` |
| Renderer CLI | only macOS `--check-arc-automation` argv mode (main.rs:427) | multicall `--render-workflow-video <dir>` in `src\main.rs` (S:0x49d88d0 SIMD fragments, S:0x52d9658), `src\workflow_video_cli.rs` (S:0x49f2eb0): `render.lock`, `render-scenes.json`, `rendered/`, NDJSON stdout; env `SCREENPIPE_VIDEO_CLI` given to Pi (S:0x4991920) | `render-cli` |
| Pi `video-sop` tools | base Pi extensions: web-search, mcp-bridge (pi.rs:957, :997). No video tools | embedded `workflow-video.ts` (S:0x52a1ae7..0x52a57a7) + skill `.pi/skills/video-sop/SKILL.md` (S:0x52a0b70); tools `read_video_sop`, `edit_video_sop`, `render_video_sop`; install failure log (S:0x52a5855) | `pi-video-sop-extension` |
| Video project workspace / draft model | none | `<base>/pi-workflows-guide/video-projects/<uuid>/video-project.json` + `<sourceId>.image`; SOP `video` draft (`version`, `sourceHash`, `scenes`) (FE 4656 @21136..22900, FE 755 @45800..49600, @252000..272500) | `video-project-workspace` |
| Captured-moment replay | none (engine has `/frames/:id` only; no thumbnail/preview-media routes) | `src\workflows_media.rs` (S:0x498cf98, S:0x514c1b3..0x514c38c, S:0x49f0cb8): `load_workflow_recording`, `release_workflow_recording`, a local loopback replay HTTP server `GET /media/:token` with byte ranges | `recording-replay` |
| Open captured moment | base `search_navigate_to_timeline` (commands.rs:1756) and `screenpipe://frame/<id>` deeplink (deeplink-handler.tsx:215) | command `open_workflow_captured_moment(frameId, timestamp)` (S:0x5111c78+1139, S:0x49f2d50..0x49f2e5e) | `captured-moment-open` |
| Workflow-ready notification | `/notify` v1 + `notifications.json` history (notifications/routes.rs, store.rs); no ledger | `src\notifications\workflow.rs` (S:0x498a128..0x498a3a0, S:0x52871e4..0x5287570) + workflow branch of `notifications/routes.rs` (S:0x514b0d8..0x514b960): `notification-ledger.json`, suppression reasons, `workflow-review-ready` event, `workflow-review:<key>` ids, `screenpipe://workflows?...` deeplink | `workflow-ready-notification` |
| Entitlement gates and analytics | none | paid-tier (Business) narration gate, sign-in gate, `/api/sops/access` (FE 4656 `M()`), PostHog `workflow_video_*`, `workflow_notification_*` | `entitlement-and-telemetry` (ee-stub) |

Grep of BASE for every new identifier (`workflow_video`, `workflows_media`, `video-sop`, `narration`,
`notification-ledger`, `workflow-video`, `SCREENPIPE_VIDEO_CLI`, `captured_moment`, `workflow_recording`, `mov_text`,
`zoompan`, `atempo`, `drawtext`, `workflow-review-ready`, `preview-media`, `get_screenpipe_base_dir`) found 0 hits in code.
The only "narration" hit is a test-case label in `crates/screenpipe-audio/examples/bench_quality.rs`. Everything below
is new except where marked "changed".

### End-to-end data flow (target)

```
SOP editor (FE 755)                    Workflows platform (FE 4656)              Rust app / core
 build VideoDraft from SOP  ──►  direct path: resolve screenshots (/frames/<id>?fallback=false)
 (version 1, sourceHash,          → data URLs → create_workflow_video(id, scenes) ─────────► app workflow_video.rs
  scenes section-N)                 listen "workflow-video-<id>" (progress strings)           → core workflow_video::render
                                    abort → cancel_workflow_video(id)                         → <appData>/workflow-videos/<id>/
                                    release → discard_workflow_video(id)                         video.mp4 + captions.vtt
                              agent path: write <base>/pi-workflows-guide/video-projects/<uuid>/
                                    video-project.json + <src>.image, start Pi session (tools
                                    read/edit/render_video_sop) ──► Pi ext workflow-video.ts
                                                                     render: write render-scenes.json, spawn
                                                                     $SCREENPIPE_VIDEO_CLI --render-workflow-video <dir>
                                                                     ──► same exe, workflow_video_cli.rs → core render
                                                                         → <dir>/rendered/video.mp4 + captions.vtt
                                    FE copies rendered/* into <appData>/workflow-videos/<uuid>/
 keeps last 3 previews; download MP4 / VTT via save dialog
```

---

## 2. Delta items

### 2.1 `gap_workflow-sop-video-and-media-core-renderer` — narrated SOP video renderer (new, XL, observed)

**Module.** `crates/screenpipe-core/src/workflow_video.rs` (path S:0x524f5f8, S:0x512a698, S:0x59f8e66, S:0xa0ec3d8;
tracing target `screenpipe_core::workflow_video`, event at line 224 with fields `message`, `status`, `attempt`,
S:0xa0ec241). Core serde types are named `Scene` (5 fields) and `Focus` (3 fields) (S:0x52b8fa0, S:0x52b8fd0,
S:0x6186fa8). The app re-exports or wraps them as `WorkflowVideoScene` / `WorkflowVideoFocus` (S:0x52ca810,
S:0x52ca850, S:0x61e24b8; field list `x y zoom` / `title narration image pace focus` at R:0x5a630a0..0x5a630d8).

**Data shapes (observed field names).**

| Type | Field | Type | Constraint |
|---|---|---|---|
| `Focus` / `WorkflowVideoFocus` | `x` | f64 | 0..=1, normalised horizontal centre of interest |
| | `y` | f64 | 0..=1, normalised vertical centre |
| | `zoom` | f64 | 1..=1.6 |
| `Scene` / `WorkflowVideoScene` | `title` | string | non-blank, <= 140 Unicode scalar values |
| | `narration` | string | non-blank; total across scenes <= 18000 scalar values |
| | `image` | string or null | app path: a `data:image/(png|jpeg|webp);base64,...` URL; CLI path: an absolute file path inside the project dir |
| | `pace` | f64 | 0.85..=1.25 (default 1) |
| | `focus` | `Focus` or null | only meaningful when `image` is present |

Scene count: 1..=50. Validation failures map to four distinct errors (S:0xa0ec558): too long overall (tell the user to
split the SOP); a section lacks a title (<=140) or narration; pace or focus out of range; scene count outside 1..50.

**Pipeline (all ffmpeg invocations share common flags `-hide_banner -loglevel error -nostdin -y -threads 2
-filter_threads <n>`, S:0xa0ec241; launched with base `ffmpeg_cmd_async` so no console window appears).** Ordering is
reconstructed from literal order in `.rdata` (S:0x524f5f8..0x524fb68).

1. **Workspace.** Create a private temp dir (prefix `screenpipe-desktop-video`, S:0x524f7f8; inferred
   `tempfile::Builder`). All intermediates live there and are removed on exit, success or failure.
2. **Preflight (capability check).** Write `title.txt` (brand text), `caption.txt` (a short label) and `preflight.vtt`
   (a WEBVTT file with one 42 ms cue `00:00:00.000 --> 00:00:00.042`, i.e. one frame at 24 fps) (S:0x524f680..0x524f6ca).
   Render exactly one frame: input 0 = `-f lavfi -i color=c=0xf7f7f3:s=1280x720:r=24`, input 1 = `preflight.vtt`;
   `-map 0:v:0 -map 1:s:0 -c:s mov_text -vf <text-card filter> -frames:v 1 -c:v libx264 -threads ... preflight.mp4`
   (S:0x524f7f8). This proves the local ffmpeg build has lavfi, drawtext, libx264 and mov_text. On failure, return the
   generic ffmpeg-failure error, which points the user at the local video tooling (S:0x524fb68).
3. **Screenshot normalisation** (per scene with an image). Decode with the image-decoder guard `-max_pixels 40000000`,
   downscale with `scale=w='min(iw,2048)':h='min(ih,998)':force_original_aspect_ratio=decrease`, and write
   `image-<n>.png` (S:0x524f7f8..0x524f8f4). If decoding fails, return an undecodable-screenshot error
   (S:0x524fa2a).
4. **Narration** (per scene, progress stage `Narrating`, S:0x524f90e). Get speech audio for `narration` from the speech
   provider (item 2.2) and save it as `speech.mp3`. Retime and decode it with `-i speech.mp3 -vn -af atempo=<pace> -f
   s16le -ac 1 -ar 16000 pipe:1` (S:0x524f92b..0x524f934; the arg table also holds `-t`/`61` next to these, read as a
   per-request decode cap of about 61 s, *inferred*). Append the PCM bytes to `soundtrack.pcm` (S:0x524f7f8). Scene
   duration = bytes / 32000 s. If the decoded duration is zero, or exceeds the cap, fail with a
   bad-narration-length error (S:0x524f934). *Inferred consequence:* long narration must be sent to TTS in chunks
   (sentence-aligned, roughly <= 700 characters each) so that no single response exceeds the cap. The SOP editor
   already caps per-step narration at 800 characters (FE 755 @34800).
5. **Duration limit.** Keep a running total of scene durations. If it exceeds 20 minutes (1200 s), abort with the
   video-too-long error (20-minute ceiling) (S:0x524fa2a).
6. **Captions.** Build `captions.vtt`: a `WEBVTT` header, then one cue per scene. The cue runs from the cumulative start
   to the end of that scene, formatted `HH:MM:SS.mmm --> HH:MM:SS.mmm` (separator literal ` --> `, R:0x524f98e). The cue
   text is the scene narration with `&`, `<`, `>` escaped as `&amp;`, `&lt;`, `&gt;` (S:0x524f934). *Inferred:* one cue
   per scene. Splitting a scene into sentence cues is allowed if timing is proportional.
7. **Per-scene video part** (progress stage `Rendering`, S:0x524f99b). The canvas is 1280x720 at 24 fps, background
   `0xf7f7f3`.
   - *Screenshot scene:* input `-loop 1 -framerate 24 -i image-<n>.png`, duration = scene duration. Video filter:
     - without focus: `scale=1280:624:force_original_aspect_ratio=decrease,pad=1280:624:(ow-iw)/2:(oh-ih)/2:color=0xf7f7f3`
       (S:0xa0ec241);
     - with focus: first letterbox onto an exact 2048x998 canvas (`scale=2048:998:force_original_aspect_ratio=decrease,
       pad=2048:998:(ow-iw)/2:(oh-ih)/2:color=0xf7f7f3`), then `zoompan` with `d=1:s=1280x624:fps=24` (S:0xa0ec159..
       0xa0ec21c). Zoom expression: `z = 1 + (zoom-1) * rampIn * rampOut`. `rampIn = min(max((on/24-0.8)/0.3,0),1)`
       holds wide for 0.8 s, then zooms in over 0.3 s. `rampOut = min(max((E-on/24)/0.3,0),1)` zooms back out over the
       final 0.3 s before time `E` (S:0xa0ebe90 tail, S:0xa0ec13c..0xa0ec147). *Inferred:* `E` = scene duration minus a
       short tail (about 0.3 s). Pan: `x = max(0, min(iw-iw/zoom, iw*fx - iw/zoom/2))`, and the same for `y` with `ih`,
       `fy`. The resulting camera motion starts wide, pushes in, holds, then pulls back out.
     - then `pad=1280:720:0:96:color=0xf7f7f3` (screenshot area 1280x624 sits below a 96 px header band, S:0x524f9b6),
       then the title drawtext: `textfile=title.txt:expansion=none:fontsize=20:fontcolor=0x171714:x=32:y=12:
       line_spacing=4` (S:0x524f6d7).
   - *Text-only scene* (no image): input `-f lavfi -i color=c=0xf7f7f3:s=1280x720:r=24`. Draw three text layers: the
     title (as above); a small grey label at (48,112), fontsize 18, colour `0x77776f`, that identifies a text
     walkthrough; and the narration from `caption.txt` at x=48, vertically centred `y=(h-text_h)/2`, fontsize 24, colour
     `0x171714`, `line_spacing=8` (S:0x524f73d).
   - Encode: `-c:v libx264 -preset veryfast -crf 23 -pix_fmt yuv420p -an -movflags +faststart part-<n>.mp4`
     (S:0x524f9e3, R:0x524fa1e). Parts carry video only. `drawtext` does not wrap, so Rust must pre-wrap
     `title.txt` and `caption.txt`. *Inferred* widths: about 100 chars per line for the title at size 20 and about 85
     for captions at size 24, truncating the caption with an ellipsis if it would overflow 1280x(720-150).
     `expansion=none` disables `%{}` expansion. Pass text through files, never inline.
8. **Concat and mux** (progress stage `Finishing`, S:0x524f90e). Write `parts.txt` with one `file 'part-<n>.mp4'` line
   per part (S:0x524f8fa). Then run the final mux (S:0x524fa2a):
   - inputs: `-f concat -safe 0 -i parts.txt`, then `-f s16le -ar 16000 -ac 1 -i soundtrack.pcm`, then `-i captions.vtt`;
   - maps `-map 0:v:0 -map 1:a:0 -map 2:s:0`;
   - `-c:v copy`, `-c:a aac -ar 48000 -b:a 96k`, `-c:s mov_text -disposition:s:0 default -metadata:s:s:0 title=Narration`;
   - `-shortest -movflags +faststart -xerror video.mp4`.
   Copy `captions.vtt` next to `video.mp4` as a sidecar.
9. **Output checks.** If `video.mp4` exceeds the per-video size cap, delete it and return an
   output-too-large error (S:0x524fa2a). Value not recovered. *Inferred default 256 MiB*, which keeps three previews under the
   700 MiB preview quota of item 2.3.

**Timeouts and cancellation.** Every ffmpeg invocation has a wall-clock timeout (it reports a processing-timeout
error, S:0x524fb68; *inferred* 300 s per invocation). The whole render has an overall deadline (S:0x512a760; *inferred*
570 s, so it finishes inside the Pi tool's 600 s budget in item 2.5). The API takes a cancellation token: on cancel, kill
the running ffmpeg child, abort any in-flight TTS request, and return a cancelled-by-user error (S:0x512a730).
If ffmpeg cannot be spawned, return the same generic ffmpeg-failure error (S:0x524fb68).

**Progress API.** The caller passes a callback `Fn(&str)` that receives stage labels (`Narrating`, `Rendering`,
`Finishing`). *Inferred:* labels may carry a suffix giving the section index and total. The app forwards them as event payloads (item
2.3); the CLI prints them as NDJSON (item 2.4).

**Proposed Rust surface (Freenpipe-authored names).** `pub struct Scene`, `pub struct Focus`,
`pub fn validate(&[Scene]) -> Result<()>`, `pub async fn render(scenes, out_dir, speech: &dyn SpeechProvider, progress,
cancel) -> Result<RenderOutput { video: PathBuf, captions: PathBuf, duration_secs: f64 }>`. Wrap ffmpeg discovery in
the base `find_ffmpeg_path()`.

**Fonts (implementation note).** Windows ffmpeg builds without fontconfig fail `drawtext` when no font is given.
Resolve a system font (`%WINDIR%\Fonts\segoeui.ttf`, else `arial.ttf`) and pass `fontfile=` (escape `:` and `\`).
Preflight then catches a missing font.

**Base files.** New `crates/screenpipe-core/src/workflow_video.rs` (+ optional `workflow_video/tts.rs`),
`crates/screenpipe-core/src/lib.rs` (module export), reuse `crates/screenpipe-core/src/ffmpeg.rs`. `tempfile`, `tokio`,
`reqwest`, `serde_json` and `uuid` are already core deps (`crates/screenpipe-core/Cargo.toml`).

**Tests.** Validation table tests for every limit. A golden test of filter-string construction for focus and no-focus
scenes, including VTT escaping and timestamp formatting. A feature-gated integration test that renders two scenes with a
silent stub speech provider and checks the streams with ffprobe: h264 1280x720, aac 48 kHz, mov_text track titled
`Narration`.

---

### 2.2 `gap_workflow-sop-video-and-media-narration-decloud` — speech generation for narration (decloud, L)

**Target (observed).**
- Endpoint suffix `/tts` (R:0x524fbea, a 4-byte run stored right before the speech error strings). It is appended to
  the account AI gateway base. *Inferred:* full URL `https://api.screenpipe.com/v1/tts`, since the gateway base
  `https://api.screenpipe.com/v1` appears in core at S:0x512ac28 and cloud.md cloud-042 covers the configurable base.
- Request headers: bearer token, `x-screenpipe-narration-profile: <profile>`, and `content-type` (S:0x524fcb0). The
  profile value and JSON body were not recovered. *Inferred* body: the scene text plus voice/format hints. Token source:
  the desktop app's cloud token on the app path, env `SCREENPIPE_API_KEY` on the CLI path (S:0x49f2eb0, S:0x4991920).
- Response handling (S:0x524fbef..0x524fcb0): each failure class has its own error. Network failure → speech service
  unreachable; non-2xx → request failed, with the HTTP status code included; non-audio content type → response was not
  audio; body over a size cap → audio response too large; empty body → no audio returned. A version-mismatch status
  indicates that the narration backend must be upgraded.
- Gateway error codes mapped to user messages (S:0xa0ebe90): `monthly_cost_limit_exceeded`,
  `daily_cost_limit_exceeded`, `hosted_ai_global_spend_limit`, and a `requires Business` marker. The messages cover: service
  busy or rate-limited, hosted AI paused, daily or monthly allowance exhausted, session expired (re-authentication
  needed), narration restricted to the Business tier, authorization failure, and service outage. Most also reassure the
  user that the SOP was not modified.
- Retries: a tracing event with `status` and `attempt` (S:0xa0ec241, line 224) shows a retry loop. *Inferred:* up to 3
  attempts with 1 s / 2 s backoff, on network errors, 429 and 5xx only.
- The app path refuses to start without a signed-in account (S:0x49f0490; CLI S:0x52d8ebc).

**Freenpipe design (replace the vendor call; never contact screenpipe.com).** Add a provider trait in core:
`trait SpeechProvider { async fn synthesize(&self, text: &str, cancel) -> Result<Vec<u8> /*mp3 or wav*/>; fn name(); }`.
The renderer decodes whatever ffmpeg can read, so the provider may return MP3, WAV or OGG. Save the bytes as
`speech.<ext>` and keep the atempo/PCM path unchanged. Implementations, in order of preference:

1. **BYO OpenAI-compatible** (`openai-compatible`): `POST {baseUrl}/audio/speech` with JSON
   `{model, input, voice, response_format:"mp3"}` and header `Authorization: Bearer <key>`. Leave out the vendor
   narration-profile header; the profile maps to `voice`. Reuse the user-configured gateway base from cloud-042 when the
   user chooses "use my AI gateway"; otherwise use a dedicated base URL. Default model `tts-1`, voice `alloy` (*proposal*).
   Input cap 4096 chars per request, so chunking (2.1 step 4) is required.
2. **Local OS TTS** (`local`): on Windows, synthesize to WAV through WinRT `Windows.Media.SpeechSynthesis` (or SAPI via
   `windows` crate COM). On macOS use `AVSpeechSynthesizer` / `say -o`. On Linux use `espeak-ng` if present. No network
   needed.
3. **Captions-only** (`captions-only`, the default when nothing is configured): no speech. Each scene gets silence of
   estimated duration `max(2.5 s, words / (2.5 words/s * pace))`, generated as zero PCM straight into
   `soundtrack.pcm`. Captions and the mov_text track are kept, so the video still reads as a narrated walkthrough. The
   UI labels this mode "captioned (no voice)".

Settings (Freenpipe-original key in the settings store `extra` map): `workflowVideoNarration`:
`{ provider: "captions-only" | "openai-compatible" | "local", baseUrl?: string, model?: string, voice?: string,
useAiGateway?: bool }`. Keep the API key in the secret store under `workflow_video.tts_api_key`, never in store.bin.
Response cap 25 MiB per request (*proposal*). Map 401/403 to "check your narration API key", 429 to "provider busy",
and 5xx/network to "speech provider unavailable". Every message also says the SOP is unchanged. Drop the vendor quota
codes.

CLI path: the app passes provider config to the CLI child through env. Freenpipe-original names: `FREENPIPE_TTS_PROVIDER`,
`FREENPIPE_TTS_BASE_URL`, `FREENPIPE_TTS_MODEL`, `FREENPIPE_TTS_VOICE`, `FREENPIPE_TTS_API_KEY`. Keep honouring
`SCREENPIPE_API_KEY` as the bearer token only when the provider is `openai-compatible` with `useAiGateway`.

**cloud.md addendum.** Add a row: vendor gateway `/tts` (SOP narration), header `x-screenpipe-narration-profile`. Freenpipe
disposition: this item.

**Base files.** Core `workflow_video` (trait + impls), `apps/.../src-tauri/src/store.rs` (settings read),
`apps/.../src-tauri/src/secrets.rs`, a settings UI card in `components/settings/` (frontend dimension).
Confidence: endpoint suffix and header observed; body, profile value, retry policy and URL prefix inferred.

---

### 2.3 `gap_workflow-sop-video-and-media-app-video-jobs` — app-side video job model and commands (new, L)

**Module.** `apps/screenpipe-app-tauri/src-tauri/src/workflow_video.rs` (S:0x498bcc8, S:0x49f0400). Commands (S:0x4993580,
S:0x5111c78+617, S:0x5177a68; bindings FE `8001-00ed120106af2449.js` @1808, @5925, @7370):

| Command | Args (JSON) | Returns | Behaviour |
|---|---|---|---|
| `create_workflow_video` | `id: string` (UUID v4 from the caller), `scenes: WorkflowVideoScene[]` | `{path: string, captionsPath: string}` (camelCase, S:0x5a63148) | render and store a preview; stream progress |
| `cancel_workflow_video` | `id` | `null` | cancel the running render whose id matches; no-op otherwise |
| `discard_workflow_video` | `id` | `null` | delete a finished preview |

All three need `#[tauri::command]` + `#[specta::specta]`. Regenerate `lib/utils/tauri.ts`.

**Storage layout.**
- Root: `<app_data_dir>/workflow-videos/` (Tauri `app.path().app_data_dir()`; the frontend uses `appDataDir()+"/workflow-videos"`,
  FE 4656 @19903). On Windows this is `%APPDATA%\<identifier>\workflow-videos`, which the existing `assetProtocol`
  scope `$APPDATA/**` already covers (tauri.conf.json:113), so `convertFileSrc` playback works.
- Finished preview: `workflow-videos/<id>/video.mp4` and `workflow-videos/<id>/captions.vtt` (S:0x49f059b, S:0x49f06a8).
  Directory names must match `^[a-f0-9]{8}-[a-f0-9-]{27}$` (the frontend filter, FE 4656 @19903). Reject any other `id`
  with an invalid-request error (S:0x49f0490). This also blocks path traversal.
- Working dir: `workflow-videos/.render-<id>/` (S:0x49f0490 `.render-`). Decoded screenshots go to
  `source-<n>.image` (S:0x49f058b..0x49f0594). It is removed on every exit path.

**`create_workflow_video` sequence (observed messages, inferred order).**
1. Validate `id` and scenes (core `validate`; more than 50 scenes → a too-many-sections error, S:0x49f059b). Any other shape
   problem → the invalid-request error.
2. Require a usable narration provider. Target: signed-in account (S:0x49f0490); Freenpipe: item 2.2, where
   captions-only always qualifies.
3. Require ffmpeg (`find_ffmpeg_path()`). Missing → an error stating that the video tooling is missing and the app needs a restart (S:0x49f0490).
4. Single-flight: a global `Mutex<Option<Job{id, cancel}>>`. A second create while one runs fails with a busy error
   (a render is already in progress, S:0x49f0416). A poisoned lock → a job-state-unavailable error (S:0x498bcf8).
5. Resolve app storage (failure → an app-storage-not-found error) and create `workflow-videos` (failure → a
   storage-creation error) (S:0x49f06a8).
6. Quota: sum the sizes of `*/video.mp4` under `workflow-videos`. If over the cap, fail with a preview-quota-full error that
   asks the user to save and dismiss older previews (S:0x49f059b). Cap 734003200 bytes (700 MiB), matching the frontend's
   identical check (FE 4656 @20318, `0x2bc00000`). Before summing, delete UUID-named dirs whose mtime is older than
   24 h, the same pruning rule the frontend uses (`864e5` ms).
7. Emit the first progress event, a preparing-stage label (S:0x49f053e).
8. Materialise screenshots. Each `image` must be a `data:` URL with MIME `image/png`, `image/jpeg` or `image/webp`;
   otherwise fail with an unsupported-format error. Base64 decode failure → an unreadable-screenshot error. Per-image
   cap → a single-screenshot-too-large error, 12 MiB (*inferred*, matching FE 4656 @22663 `12e6` and the frame-fetch cap
   `0xc00000` @19229). Total cap → a combined-screenshots-too-large error, 32 MB (*inferred*, FE `32e6`). I/O failure
   → a screenshot-preparation or storage-preparation error. Deduplicate identical data URLs to one file.
9. Call core `render` with `out_dir = .render-<id>/out`, the progress callback, and the job's cancel token.
10. Move `video.mp4` and `captions.vtt` into `workflow-videos/<id>/`. Failure → a save-failed error (one variant adds
    a hint about free disk space). Remove `.render-<id>`. Return absolute paths.

**Progress event.** Event name `workflow-video-<id>` (prefix S:0x49f052d). Payload: a plain JSON string, the
human-readable stage (the frontend forwards `e.payload` to its status line, FE 4656 @24824). Emit it app-wide
(`app.emit`). The frontend treats the first event as confirmation that the backend took the job; only after that does an abort send
`cancel_workflow_video`.

**`discard_workflow_video`.** Validate `id`. Delete `workflow-videos/<id>` recursively. If the dir is missing or not a
real dir, return a storage-not-found error; if deletion fails, return a removal-failed error (S:0x498bd38).
Discarding the running job's id cancels it first.

**Frontend contract this must satisfy (FE 755 @252000..272500, FE 4656 @24824..26000; owned by frontend-047).**
- Converts `path`/`captionsPath` with `convertFileSrc`. Plays with a `<track kind=captions label=Narration default>`.
- Keeps at most the 3 most recent previews per open SOP and discards the oldest. On unmount it discards all previews.
- Download: save dialog, default name = title stripped to `[a-zA-Z0-9 -]`, max 80 chars, fallback `video-sop`;
  extension `mp4` or `vtt`; copies the file.

**Base files.** New `apps/.../src-tauri/src/workflow_video.rs`. Register in `main.rs` (invoke handler + specta list in
`specta_bindings.rs`) and manage state. Reference: `meeting_export.rs` for the long-running pattern (release locks
before rendering).

---

### 2.4 `gap_workflow-sop-video-and-media-render-cli` — `--render-workflow-video` multicall mode (new, M)

**Is the CLI a multicall mode of the app exe? Yes (observed).** Evidence:
- The SIMD argv-compare fragments `--render-workflo` + `r-workflow-video` (overlapping 16-byte halves of the 23-byte
  literal `--render-workflow-video`, S:0x49d88d0). They sit in the same table as the other multicall flags
  (`--screenpipe-portable-mcp`, `--screenpipe-acp-runtime`, `--screenpipe-acp-process-guard`, R:0xa0ebd00).
- An argument-count error (exactly one project directory must be given) in `src\main.rs` (S:0x52d9658, S:0x49f4f28),
  followed by the literal `error`.
- The extension spawns `env.SCREENPIPE_VIDEO_CLI` with args `["--render-workflow-video", <cwd>]` (S:0x52a4f3c).
- The app sets `SCREENPIPE_VIDEO_CLI` with the other Pi child env vars (S:0x4991920, S:0x5175fd0). *Inferred value:*
  `std::env::current_exe()`.

**shell.md addendum.** Add `--render-workflow-video <dir>` to shell-024's multicall table (shell.md:112, :780).

**Dispatch rule.** In `main.rs`, check argv *before* the single-instance `/focus` handoff (base main.rs:445-474) and
before any Tauri, tray, Sentry or recording initialisation. Otherwise a second process would focus the running app and
exit. If `args[1] == "--render-workflow-video"`, run the CLI and `std::process::exit(code)`. Exactly one further argument
is allowed; anything else prints `{"error":"<message>"}` and exits non-zero.

**Module.** `apps/.../src-tauri/src/workflow_video_cli.rs` (S:0x498fc88, S:0x49f2eb0..0x49f2fe8).

**Protocol.**
- Workspace `<dir>`: must exist, be a directory and canonicalise; otherwise an invalid-workspace error (S:0x49f2eb0).
- Lock: create `<dir>/render.lock` exclusively. If it exists, fail with the render-already-running busy error (S:0x52d8ebc).
  Always remove it on exit. *Inferred:* treat a lock older than 30 min as stale.
- Credentials: target reads `SCREENPIPE_API_KEY` and fails with a sign-in-required error if it is absent. Freenpipe reads the narration
  provider config from env (item 2.2); with captions-only no secret is needed.
- ffmpeg missing → the video-tooling-missing error (S:0x49f2eb0).
- Plan: read `<dir>/render-scenes.json`, a JSON array of core `Scene` (`{title, narration, image: string|null, pace,
  focus: {x,y,zoom}|null}`, the shape the extension writes, S:0x52a4df0). A file above the size cap fails with a
  plan-too-large error (S:0x49f2f68; *inferred* cap 256 KiB). Each non-null `image` must canonicalise to a regular file
  **inside** `<dir>` (otherwise a path-escape error) and must exist (otherwise a missing-screenshot error,
  S:0x49f2fe8). The sum of image sizes must stay under the export limit (otherwise a combined-size error;
  32 MB, as in 2.3).
- Output: core `render` into `<dir>/rendered/` → `rendered/video.mp4`, `rendered/captions.vtt` (S:0x49f2eb0 `rendered`,
  S:0x52a56da).
- **stdout** is NDJSON, one object per line, each line < 64 KiB (the extension aborts on a 65536-char buffer,
  S:0x52a5163):
  - `{"progress":"<stage text>"}` for each progress callback (the extension relays the first 500 chars);
  - `{"error":"<message>"}` once on failure (relayed, first 1000 chars);
  - `{"complete":true}` once on success, after the files are flushed.
  Non-JSON lines are ignored by the consumer, so tracing must go to stderr or a log file (the extension discards
  stderr: stdio `["pipe","pipe","ignore"]`).
- **stdin**: the parent closes stdin to cancel (on abort or on its 600 s timeout). The CLI watches stdin in a thread and
  treats EOF as cancellation. That cancels the core render and prints `{"error":...}` carrying the cancelled-by-user message.
- Exit code 0 only on success with `complete` emitted; otherwise 1.

**Pi env injection (changed).** In `apps/.../src-tauri/src/pi.rs` (base sets `SCREENPIPE_API_KEY` at :1671), also set
`SCREENPIPE_VIDEO_CLI=<current_exe>` on every Pi child that gets the video tool (item 2.5), plus the Freenpipe TTS env
vars. agents_pipes.md:467 already lists the env var.

**Base files.** `apps/.../src-tauri/src/main.rs`, new `workflow_video_cli.rs`, `pi.rs`.

---

### 2.5 `gap_workflow-sop-video-and-media-pi-video-sop-extension` — Pi `video-sop` tools and skill (new, M)

**Install trigger (inferred, strong).** The app's `pi.rs` holds the tool name `render_video_sop` (S:0x4980200), the
embedded extension `workflow-video.ts` (S:0x52a1ae7) and the skill path `.pi/skills/video-sop` + `SKILL.md`
(S:0x52a0b70). It logs a failure to install the video editing tool (S:0x52a5855). The frontend starts the session with
`pi_start(sessionId, projectDir=<video project>, userToken, providerConfig{allowedTools:["read_video_sop",
"edit_video_sop","render_video_sop"], maxTokens:8192}, ...)` (FE 4656 @21136+, @5444). Rule: when the
provider config's `allowedTools` contains `render_video_sop`, write the extension into the session's extension dir and
write the skill to `<projectDir>/.pi/skills/video-sop/SKILL.md`. Set `SCREENPIPE_VIDEO_CLI`.

**Extension contract (write it from scratch; paraphrased).** Module-level state per session: the loaded draft, a
"proposal accepted" flag, a "render attempted" flag, and the set of inspected screenshot source ids.

Data types (field names are interop):
- `VideoFocus = {x, y, zoom}`.
- `VideoDraft = {version: 1, sourceHash: string, scenes: [{id, title, narration, includeImage: bool, imageSourceId?,
  pace?, focus?: VideoFocus|null}]}`.
- `VideoEdit = {changes: [{id, title?, narration?, maxNarrationWords?, includeImage?, imageSourceId?, pace?,
  focus?: VideoFocus|null}], order?: string[], render: bool}`.

Validation (shared with the frontend validator in FE 755 module 13949, @45800):

| Rule | Limit |
|---|---|
| `version` | must be `1` |
| `sourceHash` | non-blank, <= 32 chars |
| scenes | 1..=50, unique ids |
| scene `id`, `imageSourceId`, `order[]` items | pattern `^section-\d+$` |
| `title` | non-blank, <= 140 Unicode scalars |
| `narration` | non-blank, <= 18000 per scene and <= 18000 summed over scenes |
| `pace` | finite, 0.85..=1.25 |
| `focus` | null or exactly the keys `x,y,zoom`; x,y in 0..=1; zoom 1..=1.6 |
| edit top-level keys | only `changes`, `order`, `render`; `render` is required boolean |
| `changes` | <= 50, unique ids, only the listed keys |
| `maxNarrationWords` | integer 1..=18000 |
| `order` | 1..=50 unique ids; when present it also *omits* sections that are not listed |

Edit application rules:
- Every referenced id must exist in the draft. Merge fields; `maxNarrationWords` is never stored.
- Changing `imageSourceId` to a different source without a new `focus` resets `focus` to null.
- With `maxNarrationWords` set, count whitespace-separated words of the resulting narration and reject if over the
  limit. The error cites the section id and the limit and requests a condensed version that retains the mandatory actions.
- Setting `focus` while `includeImage` is false is an error. Turning `includeImage` off clears `focus`.
- Re-validate the whole result as a `VideoDraft`.

Tools:

| Tool | Params | Behaviour |
|---|---|---|
| `read_video_sop` | `{guidance?: bool, scene_id?: "^section-\d+$"}` | `guidance:true` returns the SKILL.md text. With no args it loads `<cwd>/video-project.json` (reject if > 100000 bytes), parses `draft` (first call only), and returns the draft plus `screenshots` (keys of `images`) and `missingScreenshots` (required scenes whose image is excluded or missing). With `scene_id` it resolves the source id (scene's `imageSourceId`, else the id) and returns the `<cwd>/<sourceId>.image` bytes as an image content block (reject > 12000000 bytes) when the MIME is png/jpeg/webp and the model accepts image input. Otherwise it returns a text note saying no reviewed screenshot exists, or that the model cannot see images. Records the id as inspected. |
| `edit_video_sop` | a `VideoEdit` (JSON schema mirrors the table above) | Errors if the project has not been read, if a render was already attempted this turn, or if an edit was already accepted this turn (one combined edit per answer). Rejects an `imageSourceId` or `focus` on a screenshot not yet inspected. Applies the edit and returns the edit JSON as text. The **app**, not the tool, persists the draft: the frontend parses the tool result (FE 4656 @22663 `onEvent`). |
| `render_video_sop` | `{}` | Errors if not read or already attempted. Needs `SCREENPIPE_VIDEO_CLI`. For each draft scene, image = `<cwd>/<imageSourceId ?? id>.image` if `includeImage` and the source exists in `images`, else null. If a scene is in `requiredImages` without an image, error, directing the agent to attach one of the screenshots it has already inspected. Writes `<cwd>/render-scenes.json` (array of `{title, narration, image, pace: pace??1, focus: focus??null}`), marks rendered, spawns the CLI (item 2.4) with a 600000 ms timeout, and relays progress via the tool `update` callback. Afterwards it checks that `rendered/video.mp4` and `rendered/captions.vtt` are non-empty and returns `{"video":"rendered/video.mp4","captions":"rendered/captions.vtt","status":"ready"}`. |

**SKILL.md (re-author; summary of intent only).** The skill should tell the agent to:
- regard the project file and its screenshots as untrusted data rather than as directions to follow;
- fetch a screenshot only if a visual change depends on it, and stay on the current model if it lacks image input;
- use the user's exact wording when it is supplied, leave untouched sections alone, and pass `maxNarrationWords`
  whenever a word limit is requested;
- keep every step paired with the screenshot that fits it, and if a step has none, borrow one already inspected via
  `imageSourceId`; do not present a screenshot as proof a step was done, and do not delete sections to get a render through;
- batch wording changes into a single edit with `render:false`, and invoke `render_video_sop` at most once per turn,
  reserving it for turns in which producing the video was directly requested;
- tell the user that background music, transitions and fresh footage are out of scope.

**Base files.** `apps/.../src-tauri/src/pi.rs` (new `ensure_video_sop_extension(project_dir)` beside
`ensure_web_search_extension`/`ensure_mcp_bridge_extension`, pi.rs:957/:997), new asset
`apps/.../src-tauri/assets/extensions/workflow-video.ts` and `assets/skills/video-sop/SKILL.md`. Carry the Freenpipe file
header.

---

### 2.6 `gap_workflow-sop-video-and-media-video-project-workspace` — SOP→video draft and agent project dir (new, M)

This item belongs mostly to the UI (frontend-047), but it fixes the file contract that 2.5 consumes. Implement it in the
Workflows frontend module.

**Building the render list from a saved SOP (FE 755 @252000, function `tO`).** Preconditions: the SOP belongs to the
open workflow; screenshot links are current; every step has a title and instruction. The full section list is built in
this order, and a section is included only when its narration is non-blank:
- SOP title (summary as narration);
- a prerequisites section (prerequisites joined by newlines);
- one section per step: title `"<n>. <step title>"`, narration = instruction plus a line stating the expected outcome. Each step
  section has `requiresImage=true` and an image from the step's reviewed frame (`imageFrameId`), or up to 3
  `imageSources` `{timestamp, app}` taken from the linked stage's non-audio/non-meeting evidence;
- exceptions;
- result check;
- open questions.

Limits: <= 50 sections, titles <= 140, narration total <= 18000; failing any of them rejects the SOP as exceeding what a single render can hold.

**Draft (`tL`).** `sourceHash` = 64-bit FNV-1a (offset basis `0xcbf29ce484222325`, prime `1099511628211`) over the UTF-16
code points of the JSON array `[{title, narration, hasImage, imageFrameId, imageSources?}]` of all sections, rendered
lower-case hex. Draft scenes are only the `requiresImage` sections, with id `section-<index in the full list>`. If the
SOP already stores a `video` draft whose hash differs, or which references missing sections, the stored draft is stale
and the user has to reset it before continuing. The SOP document persists `video` (a `VideoDraft`) and `videoLayout` (FE 755 @34800). When the user
re-links screenshots, all scene `focus` values reset to null and `sourceHash` is recomputed (@252000 `tW`).

**Direct render (no agent).** Resolve each scene's screenshot: `imageFrameId`, else the first of up to 3
`imageSources` that resolves via local `/frames/<id>/metadata` or `/search?content_type=ocr&app_name&start_time=t-1ms&
end_time=t+1ms&limit=20`. Fetch `GET /frames/<id>?fallback=false` (15 s timeout, <= 12 MiB, png/jpeg/webp), convert to
a data URL, and call `create_workflow_video(uuid, scenes)` (item 2.3).

**Agent project dir (FE 4656 @21136, function `W`).**
- Root: `<get_screenpipe_base_dir()>/pi-workflows-guide/video-projects/`. Delete subdirs older than 24 h (first 100
  only). If there are > 100 dirs, refuse and prompt the user to finish or close earlier edit sessions.
- Create `<root>/<uuid>/`. For each scene, write its screenshot once per source id as `<sourceId>.image`. Limits:
  12000000 bytes per image and 32000000 total (exceeding it yields an error suggesting fewer screenshots).
- Write `video-project.json` = `{draft: VideoDraft, images: {<sourceId>: <mime>}, requiredImages: [<scene ids>]}`.
- Run a Pi session named `guide` (session id `<prefix>workflow-guide-<uuid>`) with `projectDir` = this dir,
  `allowedTools` = the three video tools, `maxTokens` 8192, timeout 900000 ms. The prompt is LLM text and must be
  re-authored. It should carry the draft, the user request and chat history.
- When `render_video_sop` finishes without error, copy `rendered/video.mp4` and `rendered/captions.vtt` into
  `<appData>/workflow-videos/<uuid>/`, subject to the same 24 h pruning and 700 MiB quota as 2.3. Register the result
  so `release` calls `discard_workflow_video(uuid)`.
- Always delete the project dir afterwards.

**Base files.** New `components/workflows/video/*` and `lib/workflows/video-draft.ts` in the app frontend (frontend
dimension). `get_screenpipe_base_dir` comes from shell-003.

---

### 2.7 `gap_workflow-sop-video-and-media-recording-replay` — captured-moment recording replay (new, L)

**Module.** `apps/.../src-tauri/src/workflows_media.rs` (S:0x498cf98, S:0x514c1cf, S:0x514c38c (event line 39, target
`screenpipe_app::workflows_media`), S:0x49f0cb8). The supporting recorder-lookup strings are at S:0x49f2900..0x49f2b40.

**Commands.**

| Command | Args | Returns |
|---|---|---|
| `load_workflow_recording` | `timestamp: string` (RFC 3339), `appName: string` | `null` or `{kind: "video" | "image", url, frameId, timestamp, offsetSeconds, matchDistanceSeconds}` |
| `release_workflow_recording` | `url: string` | `null` (the binding has no status wrapper; errors are ignored by the caller) |

The return shape comes from the dev mocks (FE `3768-c6244961dc4219c4.js` @20884, `9783-939b4f7bce8c6d2f.js` @75125) and
the desktop platform adapter (FE 4656 @34727).

**Lookup (inferred order, observed endpoints).**
1. Validate input: timestamp must parse (else an invalid-timestamp error); app must be non-empty (else a
   missing-application error) (S:0x49f2af8, S:0x49f289a).
2. Talk to the local engine at the configured local API URL (default `http://127.0.0.1:3030`) with the `api_auth_key`
   when set (S:0x49f2670..0x49f26f8). The address must be loopback with a valid port (invalid → an invalid-recorder-address or
   invalid-port error). If unreachable → a recorder-unreachable or recorder-unavailable error.
3. Find the frame nearest the timestamp for that app. Use `GET /frames/preview-samples?start_time=t-30s&end_time=t+30s&
   app_name=<app>&limit=8` (vision-preview-samples, vision.md:448). Pick the sample with the smallest
   |sample.timestamp - t|. `matchDistanceSeconds` = that gap. If nothing is found → `null`; a malformed response →
   a lookup-failed or invalid-frame error. The fields `video_chunk_id`, `video_offset_seconds`
   (S:0x49f2900) come from that response.
4. If the frame source is `video`: download `GET /frames/preview-media/<video_chunk_id>` (vision-preview-media). 404 →
   **interop substring** `no longer available`; interrupted stream → a download-interrupted error; wrong MIME →
   an unsupported-format error. Store the bytes in the replay cache under a fresh UUID token and return
   `{kind:"video", url:"http://127.0.0.1:<port>/media/<token>", frameId, timestamp, offsetSeconds:
   video_offset_seconds, matchDistanceSeconds}`. If the clip exceeds the per-clip cache cap, fail with the **interop
   substring** `inline preview limit` (S:0x514c1cf). The UI then suggests opening the moment in Timeline. If the cache
   cannot be created → a cache-unavailable error.
5. If the source is a snapshot: fetch `GET /frames/<id>/thumbnail?width=1280&quality=80&fallback=false`
   (S:0x49f2994; vision-thumbnail-endpoint) and return `{kind:"image", url:"data:image/jpeg;base64,...", offsetSeconds:0,
   ...}`. If the encoded data URL would exceed the inline cap → message containing **interop substring**
   `too large for inline preview` (S:0x49f2a28).
6. A fallback mode may also return small video clips inline as `data:video/mp4;base64,...`; the frontend accepts this
   (FE 4656 @34727).

Frontend URL checks the backend must pass: a video URL must match `^http://127\.0\.0\.1:\d+/media/[0-9a-f-]{36}$`; a
data URL must be `data:(video/mp4|image/(jpeg|png|webp));base64,`.

**Replay server.**
- An embedded HTTP server bound to `127.0.0.1:0` (ephemeral port), started lazily on the first video load. Bind failure
  → a replay-start-failed error (S:0x49f0d70). Route `GET /media/:token` (S:0x49f0d00).
- Request checks: `Host` must equal `127.0.0.1:<port>` (DNS-rebinding guard, S:0x514c248..0x514c24d). If `Origin` is
  present, it must be an app webview origin (`tauri://localhost`, `http://tauri.localhost`, `https://tauri.localhost`),
  echoed back in `Access-Control-Allow-Origin` (S:0x514c309). Unknown token → 404.
- Response headers: `Content-Type: video/mp4`, `Accept-Ranges: bytes`, `Content-Length`, `Cache-Control: no-store`,
  `X-Content-Type-Options: nosniff` (S:0x514c298).
- Ranges: a single `Range: bytes=a-b` (open-ended allowed) → 206 with `Content-Range: bytes a-b/total`. Unsatisfiable →
  416 with `Content-Range: bytes */total` (S:0x514c265..0x514c306).
- `release_workflow_recording(url)`: parse the token from a URL of exactly that shape and drop the cached clip. When no
  clips remain, shut the server down and log a server-stopped info event (S:0x498cfe0). Also shut down on app exit.
- Cache limits (*inferred*): <= 64 MiB per clip, <= 256 MiB total, <= 8 live tokens (evict oldest). Keep clips in memory
  or in `<app_data>/workflow-replay-cache/` (temp files deleted on release).

UI behaviour that depends on it (FE 755 @171317): the clip seeks to `offsetSeconds` and pauses 12 s later, with a 12 s
decode timeout. If `offsetSeconds >= duration`, an error suggests trying another moment.

**Base files.** New `apps/.../src-tauri/src/workflows_media.rs`, registered in `main.rs`/`specta_bindings.rs`. Reuse
axum 0.6 (already an app dep, Cargo.toml:100). Depends on engine routes from vision.md (`vision-preview-samples`,
`vision-preview-media`, `vision-thumbnail-endpoint`). The base CSP is `null`, so `<video src=http://127.0.0.1:...>` loads;
keep `media-src` open to loopback if a CSP is ever added.

---

### 2.8 `gap_workflow-sop-video-and-media-captured-moment-open` — `open_workflow_captured_moment` (new, S)

**Target.** Command `open_workflow_captured_moment(frameId: i64, timestamp: string)` (S:0x5111c78+1139, binding FE 8001
@23457). Messages: an invalid-moment error (S:0x49f2db4); deeplink prefix `screenpipe://frame/` (S:0x49f2e49); an
app-open-failed error (S:0x49f2e5e). The desktop webview adapter does not call it; it calls
`search_navigate_to_timeline(timestamp, frameId, …)` directly (FE 4656 @35256). The command therefore serves surfaces
that cannot call that command, and it works by opening the frame deeplink.

**Behaviour.** Reject `frameId <= 0` or a timestamp that does not parse (the invalid-moment error). In-process, prefer
the base `search_navigate_to_timeline(app, timestamp, Some(frameId), None, None, None)` (commands.rs:1756). Fallback:
open `screenpipe://frame/<frameId>?timestamp=<urlencoded>` with the OS opener; failure → the app-open-failed error.
The base deeplink handler already routes `screenpipe://frame/<id>` (deeplink-handler.tsx:215).

**Base files.** `apps/.../src-tauri/src/workflows_media.rs` (or `commands.rs`), `main.rs` registration.

---

### 2.9 `gap_workflow-sop-video-and-media-workflow-ready-notification` — "workflow ready to review" notifications (new, L)

**Modules.** New `apps/.../src-tauri/src/notifications/workflow.rs` (S:0x498a21a, S:0x52871e4; event line 141, target
`screenpipe_app::notifications::workflow`). A workflow branch in `notifications/routes.rs` (S:0x514b0d8..0x514b960,
events at routes.rs:482/545/553/571/589). Uses `notifications/client.rs` (S:0x4989f00..0x498a078). Builds on the `/notify`
v2 gates in meetings_memory-037/038 and api-037, which own the generic payload (`workflow_ids` is one of the 22
`NotifyPayload` fields, S:0x52cadd8).

**Files.**
- Catalog (read-only): `catalog.json`, falling back to `catalog.backup.json`, in the workflows storage dir (S:0x5287308;
  storage owned by agents_pipes-026). Fields read: `schemaVersion`, `analysis.workflows[]` → `id` (`wf-...`), `title`,
  `stages`, plus evidence status. Missing or unparsable → reason `catalog_unavailable`.
- Ledger: `notification-ledger.json` in the same `workflows` dir (S:0x514b0d8 has `workflows` immediately followed by
  `notification-ledger.json`; S:0x498a2e0). Unreadable → `ledger_unreadable` (fail closed). Write error →
  `ledger_write_failed`. Proposed shape (*inferred*; only `last_accepted_at` and the reason codes are observed):

```json
{
  "schemaVersion": 1,
  "workflows": { "wf-abc": { "fingerprint": "<hex hash of title+stages>", "last_accepted_at": "2026-10-15T09:00:00Z" } },
  "deliveries": ["2026-10-15T09:00:00Z"]
}
```

**Eligibility and suppression.** Analytics schema `workflow_notification_v1`, category `workflow_review`
(S:0x5287435).
- A workflow is *eligible* when it is a saved catalog workflow with source-backed steps. Reason literal
  `saved_supported_workflow` (S:0x514b13e); the UI equivalent is `evidenceStatus == "supported-steps"` (FE 755 @387174).
- Suppression reasons (each logged as `workflow_notification_suppressed` with a reason):

| Reason | Meaning |
|---|---|
| `no_meaningful_change` | every candidate's fingerprint equals its ledger fingerprint (S:0x514c188) |
| `daily_limit` | accepted deliveries in the trailing 24 h >= limit (*inferred* 3) |
| `ledger_full` | ledger already tracks the max number of workflows (*inferred* 500) and the new id cannot be added |
| `notifications paused` | master / snooze / quiet-hours gate from meetings_memory-037 (S:0x514b458) |
| `already_viewing` | the user is on the Workflows section now; emit `workflow-review-ready` instead of a toast (S:0x514b501) |
| `duplicate` | identical id already shown recently, or already persisted (an already-sent result) |
| `transport_failed` | the local `POST /notify` call failed (client.rs: non-2xx, missing `success`, unreadable body) |
| `display_timeout` | the panel did not confirm display in time (*inferred* 10 s) |

  `display_failed` (panel error) is reported as `workflow_notification_failed`. `inbox_write_failed` means the history
  write failed.
- Coalescing: decisions are coalesced hourly (`decision_coalesced_hourly`, S:0x5287435). *Inferred:* the notification
  key = UTC hour bucket + a hash of the sorted workflow ids, and the notification id = `workflow-review:<key>` (prefix
  S:0x52874b1; the frontend classifies by that prefix, FE `1491-*.js` @554). A second trigger in the same hour for the
  same set dedupes to the already-sent result.

**Payload sent to `/notify`.**
- `id`: `workflow-review:<key>`.
- `title`: a singular form for one workflow, otherwise a plural form that includes the count; both announce that
  workflows await review (copy: re-author, S:0x52874c6/0x52874e5).
- `body`: a brief prompt to look over the detected steps, leave comments, or turn the workflow into an SOP (re-author,
  S:0x5287504).
- `workflow_ids`: the ids; `priority: normal`.
- One action `{type:"deeplink", label, url}` (S:0x5287570), where `label` is a short singular or plural review
  call-to-action (copy: re-author) and
  url = `screenpipe://workflows?workflow=<id>&reviewRequest=<key>&source=notification` for one id, or
  `?workflows=<id1>,<id2>,...&reviewRequest=<key>&source=notification` for several.

Ids must match `^wf-[a-zA-Z0-9_-]{1,128}$` and there may be at most 100 (frontend parser in FE `2037-06384433950ad22c.js`
@17410, onboarding chunk @122671).

**Click and navigation.**
- Opening the deeplink, from a native action (`native_action`) or the webview, shows the main window and emits `navigate`
  with `{url: "/home?section=workflows&reviewRequest=<key>&workflow=<id>"}` (S:0x52871d4 `/home?`, S:0x498a288
  `url`/`navigate`). The home page already parses `section` and `reviewRequest` (FE home page @833457).
- The frontend keeps sessionStorage keys `screenpipe:workflow-review-request`, `screenpipe:last-workflow-review-request`,
  `screenpipe:workflow-review-outcomes`, `screenpipe:workflow-review-attribution` and the window event
  `workflows:review-request` (FE 2037 @16759). These belong to the frontend dimension.

**`workflow-review-ready` event (Rust→JS).** Payload: `string[]` of workflow ids (<= 100). The UI highlights those
workflows (FE home page @807293). Emit it on every accepted delivery and on `already_viewing` suppression.

**Trigger (inferred; not recovered).** Run the check after any `workflow-review` / `workflow-maintain` pipe execution
completes, and at most every 5 minutes on catalog `revision` change (poll `catalog.json` mtime). Compare the eligible set
with the ledger and deliver if anything new or meaningfully changed. Record `last_accepted_at` and the fingerprint only
after an accepted delivery. Log `workflow_notification_delivery_result` with outcome `shown` / `suppressed before
display` / `display_failed` / `display_timeout` (S:0x498a128..0x498a21a).

**Already-viewing detection (Freenpipe design).** True when the main window is visible and focused and its webview URL
has `section=workflows`. Read it via `WebviewWindow::url()`; no new command is needed.

**Base files.**
- new `notifications/workflow.rs`;
- `notifications/mod.rs` (add `pub mod workflow;`);
- `notifications/routes.rs` (route payloads that carry `workflow_ids` through the workflow gates; base `NotifyPayload`
  lines 167-179 gains `workflow_ids` via meetings_memory-037);
- `notifications/client.rs` (return a delivery result instead of fire-and-forget; base lines 27-54);
- `notifications/store.rs` (idempotent push by id);
- the deep-link handling in `main.rs` / `components/deeplink-handler.tsx` for host `workflows`.

---

### 2.10 `gap_workflow-sop-video-and-media-entitlement-and-telemetry` — vendor gates and analytics (ee-stub, S)

**Target.**
- Narration needs a signed-in Screenpipe account (S:0x49f0490, S:0x52d8ebc) and Business access (S:0xa0ebe90
  `requires Business`).
- SOP generate/edit first calls `GET https://screenpipe.com/api/sops/access` with the cloud token and needs
  `{allowed:true}` (FE 4656 @24824, `M()`).
- The web-export action (opens the SOP on the vendor site) posts to `/api/sops` (FE 4656, `openWeb`).
- PostHog events: `workflow_video_started` (props `sections`, `screenshots`), `workflow_video_completed`,
  `workflow_video_failed`, `workflow_video_cancelled`, `workflow_video_downloaded` (`format`),
  `workflow_notification_clicked` / `_opened` / `_open_failed` (`notification_category: workflow_review`).

**Freenpipe.**
- Remove every entitlement and sign-in check from the video and notification paths. Narration availability depends
  only on item 2.2.
- Make `/api/sops/access` an always-allowed local stub, or delete the call.
- Omit the web-export action (frontend-047 already marks it decloud).
- Do not emit analytics. Mirror the event names into local `tracing` debug logs only.

**Base files.** Frontend workflows modules; no Rust changes beyond leaving the checks out.

---

## 3. Numeric defaults summary

| Constant | Value | Source |
|---|---|---|
| canvas | 1280x720 @ 24 fps, bg `0xf7f7f3`, text `0x171714`, label `0x77776f` | observed S:0x524f6d7..0x524f7f8 |
| screenshot area / header | 1280x624 below a 96 px header | observed S:0x524f9b6, S:0xa0ec241 |
| screenshot pre-scale cap / focus canvas | 2048x998 | observed S:0x524f7f8, S:0xa0ec159 |
| decode pixel guard | `-max_pixels 40000000` | observed |
| focus zoom range / ramp | 1..1.6; hold 0.8 s, ramp 0.3 s | observed S:0xa0ebe90 tail |
| pace range (atempo) | 0.85..1.25, default 1 | observed (TS + core message) |
| sections | 1..50 | observed |
| title / narration | <=140 / <=18000 total (step narration in SOP <=800) | observed |
| narration PCM | s16le mono 16 kHz | observed |
| final audio | AAC 48 kHz 96 kb/s | observed |
| video codec | libx264 veryfast crf 23 yuv420p, +faststart | observed |
| max video length | 20 min | observed |
| per-video size cap | 256 MiB | inferred |
| ffmpeg step timeout / overall | 300 s / 570 s | inferred |
| Pi render tool timeout | 600000 ms | observed S:0x52a502a |
| agent edit session timeout | 900000 ms | observed FE 4656 |
| preview quota / pruning | 700 MiB (`0x2bc00000`) / 24 h | observed FE 4656 |
| previews kept per SOP | 3 | observed FE 755 |
| screenshot per-image / total | 12 MB (12000000; frame fetch cap 0xc00000) / 32 MB | observed FE; Rust values inferred equal |
| video-project.json read cap | 100000 bytes | observed S:0x52a338a |
| project dirs | <=100, 24 h prune | observed FE 4656 |
| CLI stdout line buffer | 65536 chars | observed S:0x52a5163 |
| TTS retries | 3 attempts | inferred |
| replay clip cap | 64 MiB per clip, 256 MiB total | inferred |
| replay preview window | offset .. offset+12 s, 12 s decode timeout | observed FE 755 |
| notification daily limit / ledger cap / display timeout | 3 / 500 / 10 s | inferred |

## 4. Implementation order

1. 2.1 core renderer with the captions-only provider (it can be tested offline).
2. 2.3 app commands, then 2.4 CLI multicall, then 2.5 Pi extension, then 2.6 frontend glue.
3. 2.2 BYO and local TTS providers.
4. 2.7 replay (needs the vision engine routes) and 2.8.
5. 2.9 notifications (needs meetings_memory-037/038 and agents_pipes-026 catalog storage).
6. 2.10 is done by omission throughout.

## 5. Open questions

- Exact `x-screenpipe-narration-profile` value and `/tts` JSON body (not recoverable from strings). This does not block
  the BYO design.
- Exact meaning of the adjacent literals `-t` / `61` in the PCM decode arg list (cap per TTS response vs. part duration).
- Per-video size cap, ffmpeg timeout and overall render timeout (compiled constants; not present as strings).
- Trigger for the workflow-ready notification (pipe completion hook vs. catalog watcher) and the daily limit, ledger cap
  and display-timeout values.
- How the target detects "already viewing workflows" (frontend-reported state vs. window URL).
- Whether `load_workflow_recording` ever returns inline `data:video/mp4` in normal operation or only as a fallback.
