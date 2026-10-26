# Environment variables

Freenpipe reads and sets the same environment variables as screenpipe, so
existing scripts, pipes and MCP configs keep working. The names live in
`crates/screenpipe-core/src/env.rs`; a unit test there fails when a name
is missing from this page. Variables that only screenpipe's hosted
services used are left out.

## Data directory, ports and local API access

| Variable | Meaning |
|---|---|
| `SCREENPIPE_DATA_DIR` | Overrides the data directory (default `~/.screenpipe`). |
| `SCREENPIPE_PORT` | Port of the local HTTP API (default 3030). |
| `SCREENPIPE_FOCUS_PORT` | Port of the desktop app's focus/control server (default 11435). |
| `SCREENPIPE_API_KEY` | API key the engine requires on authenticated requests. |
| `SCREENPIPE_API_AUTH_KEY` | Older name for the API key, still accepted. |
| `SCREENPIPE_LOCAL_API_URL` | Base URL of the local API, given to child processes. |
| `SCREENPIPE_LOCAL_API_PORT` | Port of the local API, given to child processes. |
| `SCREENPIPE_LOCAL_API_KEY` | Key for the local API, given to child processes. |
| `SCREENPIPE_API_URL` | Base URL agents use to reach the API. |
| `SCREENPIPE_API_BASE_URL` | Base URL of a user-configured pipe registry. Unset means no registry. |
| `SCREENPIPE_WEB_URL` | Base URL of a user-configured web service. Unset by default. |
| `SCREENPIPE_TRUSTED_HOSTS` | Extra hosts allowed to reach the API. |
| `SCREENPIPE_IGNORE_DB_LOCK` | Skips the database lock check at startup (diagnostics only). |
| `SCREENPIPE_WAL_MAINTENANCE_INTERVAL_SECS` | Seconds between the database's background WAL checkpoints (tests and diagnostics; default 30). |
| `SCREENPIPE_PLANNER_MAINTENANCE_INTERVAL_SECS` | Seconds before the first and between later query-planner statistics runs, which write `sqlite-planner-maintenance.json` (tests and diagnostics; default 10 minutes, then every 6 hours). |
| `SCREENPIPE_ENCRYPT_STORE` | Encrypts the settings store at rest. |
| `SCREENPIPE_STARTUP_AUTHENTICATION_STATUS` | How the app's startup account check resolved: `authenticated`, `logged_out` or `not_required` (Freenpipe always sets `not_required`). |

## App startup and process behaviour

| Variable | Meaning |
|---|---|
| `SCREENPIPE_SKIP_ONBOARDING` | Marks onboarding as complete at startup. |
| `SCREENPIPE_RELAUNCH_HOME_VISIBLE` | `true` or `false`, set by a safe relaunch to say whether Home was open. |
| `SCREENPIPE_E2E_SEED` | Seed flags for end-to-end test runs. |
| `SCREENPIPE_DISTRIBUTION` | Distribution tag of the running build (for example `desktop-app`). |
| `SCREENPIPE_DIST` | Short form of the distribution tag. |
| `SCREENPIPE_FD_LIMIT` | Open-file limit the engine raises itself to (default 8192). |
| `SCREENPIPE_KEEP_NORMAL_PRIORITY` | Keeps the process at normal priority instead of lowering it. |
| `SCREENPIPE_LOG` | Log filter for the CLI. |
| `SCREENPIPE_NO_UPDATE_CHECK` | Turns off the update check. |
| `SCREENPIPE_UPDATER_MANIFEST_URL` | Overrides where the updater reads its release manifest. |
| `SCREENPIPE_NO_REMINDERS` | Turns off CLI reminder messages. |
| `SCREENPIPE_BUN_PATH` | Path of the bun binary to use. |
| `SCREENPIPE_FFMPEG_PATH` | Path of the ffmpeg binary to use. |
| `SCREENPIPE_ENABLE_MDNS` | Enables mDNS advertising of the API. |

## Telemetry, diagnostics and identity tags (telemetry stays off in Freenpipe)

| Variable | Meaning |
|---|---|
| `SCREENPIPE_DISABLE_TELEMETRY` | Disables engine analytics. `CI` and `GITHUB_ACTIONS` do the same. |
| `SCREENPIPE_ANALYTICS_ID` | Anonymous analytics id. |
| `SCREENPIPE_RESOURCE_BREAKDOWN` | Logs a per-process breakdown from the resource monitor. |
| `SAVE_RESOURCE_USAGE` | Persists resource monitor samples (no `SCREENPIPE_` prefix upstream either). |
| `SCREENPIPE_SUPPORT_ID` | Support id shown in diagnostics. |
| `SCREENPIPE_TELEMETRY_ID` | Telemetry identity tag. |
| `SCREENPIPE_TELEMETRY_HOST_APP` | Host application name tag. |
| `SCREENPIPE_TELEMETRY_HOST_VERSION` | Host application version tag. |
| `SCREENPIPE_TELEMETRY_CUSTOMER_ID` | Customer tag. |
| `SCREENPIPE_TELEMETRY_DEPLOYMENT_ID` | Deployment tag. |
| `SCREENPIPE_HOST_APP` | Host application name tag (short form). |
| `SCREENPIPE_HOST_VERSION` | Host application version tag (short form). |
| `SCREENPIPE_CUSTOMER_ID` | Customer tag (short form). |
| `SCREENPIPE_ORG_ID` | Organisation tag. |
| `SCREENPIPE_DEPLOYMENT_ID` | Deployment tag (short form). |

## Capture, media and models

| Variable | Meaning |
|---|---|
| `SCREENPIPE_DIRECTML` | DirectML for the ONNX Parakeet engine (Windows). Unset or empty: automatic, DirectML when an adapter qualifies and a `DirectML.dll` sits next to the executable. `1` or `true`: on, also with the `DirectML.dll` in System32. Any other value (`0`, `false`, `off`, typos): CPU. |
| `SCREENPIPE_COMPACTION_ENCODER` | Encoder for snapshot compaction: `software`, `hardware`, or a named ffmpeg encoder; anything else falls back to the default probe order. |
| `SCREENPIPE_FILTER_PII` | Turns on PII filtering of captured text. |
| `SCREENPIPE_AUDIO_EXCLUSIONS_PATH` | Path of the audio exclusion list. |
| `SCREENPIPE_EMBEDDER` | Embedding model selection. |
| `SCREENPIPE_EMBEDDER_VERSION` | Embedding model version. |
| `SCREENPIPE_SKIP_ONNX_DOWNLOAD` | Skips the ONNX Runtime download. |
| `SCREENPIPE_ENABLE_EXPERIMENTAL_RFDETR_MLX` | Enables the experimental RF-DETR MLX detector (macOS). |
| `SCREENPIPE_PYANNOTE_BASE_URL` | Base URL for pyannote model downloads. |
| `SCREENPIPE_PYANNOTE_DIR` | Local directory holding pyannote models. |
| `SCREENPIPE_PII_REDACTOR_REPO` | Model repository of the combined PII redactor. |
| `SCREENPIPE_PII_TEXT_REDACTOR_REPO` | Model repository of the text PII redactor. |
| `SCREENPIPE_PII_IMAGE_REDACTOR_REPO` | Model repository of the image PII redactor. |
| `SCREENPIPE_PRIVACY_FILTER_API_KEY` | Key for a user-configured privacy-filter enclave. |
| `SCREENPIPE_PRIVACY_FILTER_ENCLAVE` | Selects the privacy-filter enclave. |
| `SCREENPIPE_PRIVACY_FILTER_REPO` | Model repository of the privacy filter. |
| `SCREENPIPE_PRIVACY_FILTER_URL` | Base URL of a user-configured privacy-filter service. |

## Meetings

| Variable | Meaning |
|---|---|
| `SCREENPIPE_MEETING_DETECTOR` | Overrides the meeting detector endpoint. |
| `SCREENPIPE_MEETING_STREAMING_PROVIDER` | Streaming transcription provider for live meetings. |
| `SCREENPIPE_MEETING_TRANSCRIPTION_MODEL` | Transcription model for live meetings. |
| `SCREENPIPE_MEETING_TRANSCRIPTION_LANGUAGE` | Transcription language for live meetings. |
| `SCREENPIPE_MEETING_REALTIME_URL` | Realtime transcription endpoint for live meetings. |
| `SCREENPIPE_MEETING_DEEPGRAM_API_KEY` | Deepgram key for live meetings (bring your own). |
| `SCREENPIPE_MEETING_DEEPGRAM_LIVE_URL` | Deepgram live endpoint for live meetings. |
| `SCREENPIPE_MEETING_CLOUD_TOKEN` | Token for a hosted live-meeting service. |
| `SCREENPIPE_MEETING_LOCAL_SPEAKER_NAME` | Display name of the local speaker in meeting transcripts. |
| `SCREENPIPE_EVENT_CLASSIFIER_URL` | Endpoint of a user-configured event classifier. |
| `SCREENPIPE_EVENT_CLASSIFIER_KEY` | Key of a user-configured event classifier. |

## Connections, sync and remote access

| Variable | Meaning |
|---|---|
| `SCREENPIPE_OAUTH_TIMEOUT_SECS` | How long a connection's OAuth flow waits for its callback, in seconds. |
| `SCREENPIPE_SYNC_TOKEN` | Sync token. |
| `SCREENPIPE_SYNC_PASSWORD` | Sync encryption password. |
| `SCREENPIPE_REMOTE_HOST` | Remote host for LAN sync. |
| `SCREENPIPE_REMOTE_USER` | Remote user for LAN sync. |
| `SCREENPIPE_REMOTE_KEY` | Remote key file for LAN sync. |
| `SCREENPIPE_REMOTE_PATH` | Remote path for LAN sync. |
| `SCREENPIPE_AI_GATEWAY_URL` | Base URL of the user's AI gateway (bring your own). |
| `SCREENPIPE_AI_GATEWAY_KEY` | Key for the user's AI gateway. |
| `SCREENPIPE_WEB_SEARCH_PROVIDER` | Web search provider behind `/v1/web-search` and the agents' `sp_web_search` tool: `searxng`, `brave`, `tavily`, `gateway` (the AI gateway's `/web-search`) or `off`. Unset: `searxng` when `SCREENPIPE_WEB_SEARCH_URL` is set, else the AI gateway when one is configured, else web search is off. |
| `SCREENPIPE_WEB_SEARCH_URL` | Base URL of your SearXNG instance (its JSON output must be enabled), or a replacement endpoint for `brave`/`tavily`. |
| `SCREENPIPE_WEB_SEARCH_API_KEY` | Your own key for `brave` or `tavily`; sent as a bearer token to a SearXNG instance when set. |

## Pipes and agents (given to agent child processes)

| Variable | Meaning |
|---|---|
| `SCREENPIPE_TRAJECTORIES_DIR` | Directory pipe trajectories are written to (JSONL, opt-in). |
| `SCREENPIPE_TRAJECTORIES_MAX_RUNS` | How many trajectory runs to keep. |
| `SCREENPIPE_TOOLS_HTTP_PORT` | Port of the tools HTTP server offered to ACP agents. |
| `SCREENPIPE_SESSION_ID` | Session id of the agent run. |
| `SCREENPIPE_SUBAGENT` | Set when the process is a sub-agent. |
| `SCREENPIPE_PARENT_PID` | Pid of the parent process; children exit when it goes away. |
| `SCREENPIPE_PIPE_NAME` | Name of the pipe a child process runs for. |
| `SCREENPIPE_CLI` | Path of a pre-resolved `screenpipe` CLI binary. |
| `SCREENPIPE_VIDEO_CLI` | Path of the workflow video CLI. |
| `SCREENPIPE_CHAT_SESSION_ID` | Chat session the agent belongs to. |
| `SCREENPIPE_CHAT_CONTROL_ADDR` | Address of the chat control channel. |
| `SCREENPIPE_CHAT_CONTROL_TOKEN` | Token for the chat control channel. |
| `SCREENPIPE_CHAT_CONTROL_DISABLED` | Turns the chat control channel off. |
| `SCREENPIPE_PI_AGENT_DIR` | Pi's configuration directory (default `<data dir>/pi-agent/pi-config`). Read to override the default and given to Pi processes. |
| `SCREENPIPE_FORCE_BASELINE_BUN` | Freenpipe addition: `1` makes Windows x64 run Pi with bun's baseline (non-AVX2) build even when the CPU has AVX2. For testing that path. |
| `SCREENPIPE_DISABLE_PI_PATCHES` | Freenpipe addition: `1` leaves the installed Pi runtime unpatched and removes the runtime patch if it was applied. |
| `SCREENPIPE_AI_TOOLS_HOME` | Home directory searched for AI tool configs (`.claude`, `.codex`, `.cursor`, `.gemini`, `.openclaw`, `.hermes`) when auto-connecting MCP. |
| `SCREENPIPE_ACP_ID` | ACP runtime: agent id. |
| `SCREENPIPE_ACP_CWD` | ACP runtime: working directory. |
| `SCREENPIPE_ACP_COMMAND` | ACP runtime: command to launch. |
| `SCREENPIPE_ACP_ARGS_JSON` | ACP runtime: arguments as a JSON array. |
| `SCREENPIPE_ACP_ENV_JSON` | ACP runtime: extra environment as a JSON object. |
| `SCREENPIPE_ACP_AUTH_METHOD` | ACP runtime: authentication method. |
| `SCREENPIPE_ACP_SYSTEM_PROMPT` | ACP runtime: system prompt. |
| `SCREENPIPE_ACP_SESSION_CONFIG_JSON` | ACP runtime: session configuration as JSON. |
| `SCREENPIPE_ACP_TOOL_ALLOWLIST` | ACP runtime: allowed tools. |
| `SCREENPIPE_ACP_USER_MCP_JSON` | ACP runtime: the user's MCP servers as JSON. |
| `SCREENPIPE_ACP_RESUME_SESSION_ID` | ACP runtime: session to resume. |
| `SCREENPIPE_ACP_UNATTENDED` | ACP runtime: run without a user present. |
| `SCREENPIPE_INTERNAL_ACP_WRAPPED_COMMAND` | ACP runtime: the wrapped command (internal). |
| `SCREENPIPE_INTERNAL_ACP_WRAPPED_ARGS_JSON` | ACP runtime: the wrapped command's arguments as JSON (internal). |

## Pi runtime (Pi's own names, set on Pi processes)

| Variable | Meaning |
|---|---|
| `PI_CODING_AGENT_DIR` | Pi's configuration directory; set to the isolated `pi-config` directory. |
| `PI_SKIP_VERSION_CHECK` | `1` turns off Pi's own check for newer releases (the runtime pins Pi). |
| `JITI_TRY_NATIVE` | `0` makes jiti compile extensions instead of using native TypeScript loading. |
| `PI_TELEMETRY` | `0` keeps Pi's install telemetry off. |

## MCP

| Variable | Meaning |
|---|---|
| `SCREENPIPE_MCP_TRANSPORT` | Transport of the portable MCP server. |
| `SCREENPIPE_MCP_HOST` | Host of the portable MCP server. |
| `SCREENPIPE_MCP_PORT` | Port of the portable MCP server. |
| `SCREENPIPE_MCP_PATH` | Path of the portable MCP server. |
| `SCREENPIPE_MCP_CLIENT` | Name of the MCP client. |
| `SCREENPIPE_MCP_SERVER_ALLOWLIST` | MCP servers a pipe session may call. |

## Freenpipe only

Not read by screenpipe, so they carry a `FREENPIPE_` prefix and are not in
`screenpipe_core::env`.

| Variable | Meaning |
|---|---|
| `FREENPIPE_WHISPER_GPU_FAULT` | Test hook: `inference` (or `1`/`true`) makes GPU whisper inference fail, `state` makes GPU state creation fail, so the CPU recovery can be checked on real hardware. CPU sessions are never affected. |
| `FREENPIPE_VULKAN_RUNTIME_DIR` | Build time (`scripts/stage_bundled_runtime.js`): folder holding the unpacked Vulkan runtime components whose `vulkan-1.dll` the Windows installer ships. |
| `FREENPIPE_BUNDLE_DIRECTML` | Build time: `1` also ships Microsoft's `DirectML.dll` (held back until the release licence audit). |
