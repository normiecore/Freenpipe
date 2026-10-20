// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Names of the environment variables the engine, the desktop app and the
//! agent children read or set.
//!
//! The names are interoperability identifiers: they match screenpipe exactly,
//! so scripts, pipes and MCP configs written for screenpipe keep working.
//! New code should refer to these constants rather than repeating the string.
//! Call sites that predate this module still use literals; they move over when
//! the work that owns them touches them.
//!
//! [`ALL`] lists every name, and `docs/env.md` documents each one (a unit test
//! keeps the two in step).

// ---------------------------------------------------------------------------
// Data directory, ports and local API access
// ---------------------------------------------------------------------------

/// Overrides the data directory (default `~/.screenpipe`).
pub const SCREENPIPE_DATA_DIR: &str = "SCREENPIPE_DATA_DIR";
/// Port of the local HTTP API (default 3030).
pub const SCREENPIPE_PORT: &str = "SCREENPIPE_PORT";
/// Port of the desktop app's focus/control server (default 11435).
pub const SCREENPIPE_FOCUS_PORT: &str = "SCREENPIPE_FOCUS_PORT";
/// API key the engine requires on authenticated requests.
pub const SCREENPIPE_API_KEY: &str = "SCREENPIPE_API_KEY";
/// Older name for the API key, still accepted.
pub const SCREENPIPE_API_AUTH_KEY: &str = "SCREENPIPE_API_AUTH_KEY";
/// Base URL of the local API, given to child processes.
pub const SCREENPIPE_LOCAL_API_URL: &str = "SCREENPIPE_LOCAL_API_URL";
/// Port of the local API, given to child processes.
pub const SCREENPIPE_LOCAL_API_PORT: &str = "SCREENPIPE_LOCAL_API_PORT";
/// Key for the local API, given to child processes.
pub const SCREENPIPE_LOCAL_API_KEY: &str = "SCREENPIPE_LOCAL_API_KEY";
/// Base URL agents use to reach the API.
pub const SCREENPIPE_API_URL: &str = "SCREENPIPE_API_URL";
/// Base URL of a user-configured pipe registry. Unset means no registry.
pub const SCREENPIPE_API_BASE_URL: &str = "SCREENPIPE_API_BASE_URL";
/// Base URL of a user-configured web service. Unset by default.
pub const SCREENPIPE_WEB_URL: &str = "SCREENPIPE_WEB_URL";
/// Extra hosts allowed to reach the API.
pub const SCREENPIPE_TRUSTED_HOSTS: &str = "SCREENPIPE_TRUSTED_HOSTS";
/// Skips the database lock check at startup (diagnostics only).
pub const SCREENPIPE_IGNORE_DB_LOCK: &str = "SCREENPIPE_IGNORE_DB_LOCK";
/// Encrypts the settings store at rest.
pub const SCREENPIPE_ENCRYPT_STORE: &str = "SCREENPIPE_ENCRYPT_STORE";
/// How the app's startup account check resolved: `authenticated`,
/// `logged_out` or `not_required` (Freenpipe always sets `not_required`).
pub const SCREENPIPE_STARTUP_AUTHENTICATION_STATUS: &str =
    "SCREENPIPE_STARTUP_AUTHENTICATION_STATUS";

// ---------------------------------------------------------------------------
// App startup and process behaviour
// ---------------------------------------------------------------------------

/// Marks onboarding as complete at startup.
pub const SCREENPIPE_SKIP_ONBOARDING: &str = "SCREENPIPE_SKIP_ONBOARDING";
/// `true` or `false`, set by a safe relaunch to say whether Home was open.
pub const SCREENPIPE_RELAUNCH_HOME_VISIBLE: &str = "SCREENPIPE_RELAUNCH_HOME_VISIBLE";
/// Seed flags for end-to-end test runs.
pub const SCREENPIPE_E2E_SEED: &str = "SCREENPIPE_E2E_SEED";
/// Distribution tag of the running build (for example `desktop-app`).
pub const SCREENPIPE_DISTRIBUTION: &str = "SCREENPIPE_DISTRIBUTION";
/// Short form of the distribution tag.
pub const SCREENPIPE_DIST: &str = "SCREENPIPE_DIST";
/// Open-file limit the engine raises itself to (default 8192).
pub const SCREENPIPE_FD_LIMIT: &str = "SCREENPIPE_FD_LIMIT";
/// Keeps the process at normal priority instead of lowering it.
pub const SCREENPIPE_KEEP_NORMAL_PRIORITY: &str = "SCREENPIPE_KEEP_NORMAL_PRIORITY";
/// Log filter for the CLI.
pub const SCREENPIPE_LOG: &str = "SCREENPIPE_LOG";
/// Turns off the update check.
pub const SCREENPIPE_NO_UPDATE_CHECK: &str = "SCREENPIPE_NO_UPDATE_CHECK";
/// Overrides where the updater reads its release manifest.
pub const SCREENPIPE_UPDATER_MANIFEST_URL: &str = "SCREENPIPE_UPDATER_MANIFEST_URL";
/// Turns off CLI reminder messages.
pub const SCREENPIPE_NO_REMINDERS: &str = "SCREENPIPE_NO_REMINDERS";
/// Path of the bun binary to use.
pub const SCREENPIPE_BUN_PATH: &str = "SCREENPIPE_BUN_PATH";
/// Path of the ffmpeg binary to use.
pub const SCREENPIPE_FFMPEG_PATH: &str = "SCREENPIPE_FFMPEG_PATH";
/// Enables mDNS advertising of the API.
pub const SCREENPIPE_ENABLE_MDNS: &str = "SCREENPIPE_ENABLE_MDNS";

// ---------------------------------------------------------------------------
// Telemetry, diagnostics and identity tags (telemetry stays off in Freenpipe)
// ---------------------------------------------------------------------------

/// Disables engine analytics. `CI` and `GITHUB_ACTIONS` do the same.
pub const SCREENPIPE_DISABLE_TELEMETRY: &str = "SCREENPIPE_DISABLE_TELEMETRY";
/// Anonymous analytics id.
pub const SCREENPIPE_ANALYTICS_ID: &str = "SCREENPIPE_ANALYTICS_ID";
/// Logs a per-process breakdown from the resource monitor.
pub const SCREENPIPE_RESOURCE_BREAKDOWN: &str = "SCREENPIPE_RESOURCE_BREAKDOWN";
/// Persists resource monitor samples (no `SCREENPIPE_` prefix upstream either).
pub const SAVE_RESOURCE_USAGE: &str = "SAVE_RESOURCE_USAGE";
/// Support id shown in diagnostics.
pub const SCREENPIPE_SUPPORT_ID: &str = "SCREENPIPE_SUPPORT_ID";
/// Telemetry identity tag.
pub const SCREENPIPE_TELEMETRY_ID: &str = "SCREENPIPE_TELEMETRY_ID";
/// Host application name tag.
pub const SCREENPIPE_TELEMETRY_HOST_APP: &str = "SCREENPIPE_TELEMETRY_HOST_APP";
/// Host application version tag.
pub const SCREENPIPE_TELEMETRY_HOST_VERSION: &str = "SCREENPIPE_TELEMETRY_HOST_VERSION";
/// Customer tag.
pub const SCREENPIPE_TELEMETRY_CUSTOMER_ID: &str = "SCREENPIPE_TELEMETRY_CUSTOMER_ID";
/// Deployment tag.
pub const SCREENPIPE_TELEMETRY_DEPLOYMENT_ID: &str = "SCREENPIPE_TELEMETRY_DEPLOYMENT_ID";
/// Host application name tag (short form).
pub const SCREENPIPE_HOST_APP: &str = "SCREENPIPE_HOST_APP";
/// Host application version tag (short form).
pub const SCREENPIPE_HOST_VERSION: &str = "SCREENPIPE_HOST_VERSION";
/// Customer tag (short form).
pub const SCREENPIPE_CUSTOMER_ID: &str = "SCREENPIPE_CUSTOMER_ID";
/// Organisation tag.
pub const SCREENPIPE_ORG_ID: &str = "SCREENPIPE_ORG_ID";
/// Deployment tag (short form).
pub const SCREENPIPE_DEPLOYMENT_ID: &str = "SCREENPIPE_DEPLOYMENT_ID";

// ---------------------------------------------------------------------------
// Capture, media and models
// ---------------------------------------------------------------------------

/// Set to a false value to turn DirectML acceleration off.
pub const SCREENPIPE_DIRECTML: &str = "SCREENPIPE_DIRECTML";
/// Encoder for snapshot compaction: `software`, `hardware`, or a named ffmpeg
/// encoder; anything else falls back to the default probe order.
pub const SCREENPIPE_COMPACTION_ENCODER: &str = "SCREENPIPE_COMPACTION_ENCODER";
/// Turns on PII filtering of captured text.
pub const SCREENPIPE_FILTER_PII: &str = "SCREENPIPE_FILTER_PII";
/// Path of the audio exclusion list.
pub const SCREENPIPE_AUDIO_EXCLUSIONS_PATH: &str = "SCREENPIPE_AUDIO_EXCLUSIONS_PATH";
/// Embedding model selection.
pub const SCREENPIPE_EMBEDDER: &str = "SCREENPIPE_EMBEDDER";
/// Embedding model version.
pub const SCREENPIPE_EMBEDDER_VERSION: &str = "SCREENPIPE_EMBEDDER_VERSION";
/// Skips the ONNX Runtime download.
pub const SCREENPIPE_SKIP_ONNX_DOWNLOAD: &str = "SCREENPIPE_SKIP_ONNX_DOWNLOAD";
/// Enables the experimental RF-DETR MLX detector (macOS).
pub const SCREENPIPE_ENABLE_EXPERIMENTAL_RFDETR_MLX: &str =
    "SCREENPIPE_ENABLE_EXPERIMENTAL_RFDETR_MLX";
/// Base URL for pyannote model downloads.
pub const SCREENPIPE_PYANNOTE_BASE_URL: &str = "SCREENPIPE_PYANNOTE_BASE_URL";
/// Local directory holding pyannote models.
pub const SCREENPIPE_PYANNOTE_DIR: &str = "SCREENPIPE_PYANNOTE_DIR";
/// Model repository of the combined PII redactor.
pub const SCREENPIPE_PII_REDACTOR_REPO: &str = "SCREENPIPE_PII_REDACTOR_REPO";
/// Model repository of the text PII redactor.
pub const SCREENPIPE_PII_TEXT_REDACTOR_REPO: &str = "SCREENPIPE_PII_TEXT_REDACTOR_REPO";
/// Model repository of the image PII redactor.
pub const SCREENPIPE_PII_IMAGE_REDACTOR_REPO: &str = "SCREENPIPE_PII_IMAGE_REDACTOR_REPO";
/// Key for a user-configured privacy-filter enclave.
pub const SCREENPIPE_PRIVACY_FILTER_API_KEY: &str = "SCREENPIPE_PRIVACY_FILTER_API_KEY";
/// Selects the privacy-filter enclave.
pub const SCREENPIPE_PRIVACY_FILTER_ENCLAVE: &str = "SCREENPIPE_PRIVACY_FILTER_ENCLAVE";
/// Model repository of the privacy filter.
pub const SCREENPIPE_PRIVACY_FILTER_REPO: &str = "SCREENPIPE_PRIVACY_FILTER_REPO";
/// Base URL of a user-configured privacy-filter service.
pub const SCREENPIPE_PRIVACY_FILTER_URL: &str = "SCREENPIPE_PRIVACY_FILTER_URL";

// ---------------------------------------------------------------------------
// Meetings
// ---------------------------------------------------------------------------

/// Overrides the meeting detector endpoint.
pub const SCREENPIPE_MEETING_DETECTOR: &str = "SCREENPIPE_MEETING_DETECTOR";
/// Streaming transcription provider for live meetings.
pub const SCREENPIPE_MEETING_STREAMING_PROVIDER: &str = "SCREENPIPE_MEETING_STREAMING_PROVIDER";
/// Transcription model for live meetings.
pub const SCREENPIPE_MEETING_TRANSCRIPTION_MODEL: &str = "SCREENPIPE_MEETING_TRANSCRIPTION_MODEL";
/// Transcription language for live meetings.
pub const SCREENPIPE_MEETING_TRANSCRIPTION_LANGUAGE: &str =
    "SCREENPIPE_MEETING_TRANSCRIPTION_LANGUAGE";
/// Realtime transcription endpoint for live meetings.
pub const SCREENPIPE_MEETING_REALTIME_URL: &str = "SCREENPIPE_MEETING_REALTIME_URL";
/// Deepgram key for live meetings (bring your own).
pub const SCREENPIPE_MEETING_DEEPGRAM_API_KEY: &str = "SCREENPIPE_MEETING_DEEPGRAM_API_KEY";
/// Deepgram live endpoint for live meetings.
pub const SCREENPIPE_MEETING_DEEPGRAM_LIVE_URL: &str = "SCREENPIPE_MEETING_DEEPGRAM_LIVE_URL";
/// Token for a hosted live-meeting service.
pub const SCREENPIPE_MEETING_CLOUD_TOKEN: &str = "SCREENPIPE_MEETING_CLOUD_TOKEN";
/// Display name of the local speaker in meeting transcripts.
pub const SCREENPIPE_MEETING_LOCAL_SPEAKER_NAME: &str = "SCREENPIPE_MEETING_LOCAL_SPEAKER_NAME";
/// Endpoint of a user-configured event classifier.
pub const SCREENPIPE_EVENT_CLASSIFIER_URL: &str = "SCREENPIPE_EVENT_CLASSIFIER_URL";
/// Key of a user-configured event classifier.
pub const SCREENPIPE_EVENT_CLASSIFIER_KEY: &str = "SCREENPIPE_EVENT_CLASSIFIER_KEY";

// ---------------------------------------------------------------------------
// Connections, sync and remote access
// ---------------------------------------------------------------------------

/// How long a connection's OAuth flow waits for its callback, in seconds.
pub const SCREENPIPE_OAUTH_TIMEOUT_SECS: &str = "SCREENPIPE_OAUTH_TIMEOUT_SECS";
/// Sync token.
pub const SCREENPIPE_SYNC_TOKEN: &str = "SCREENPIPE_SYNC_TOKEN";
/// Sync encryption password.
pub const SCREENPIPE_SYNC_PASSWORD: &str = "SCREENPIPE_SYNC_PASSWORD";
/// Remote host for LAN sync.
pub const SCREENPIPE_REMOTE_HOST: &str = "SCREENPIPE_REMOTE_HOST";
/// Remote user for LAN sync.
pub const SCREENPIPE_REMOTE_USER: &str = "SCREENPIPE_REMOTE_USER";
/// Remote key file for LAN sync.
pub const SCREENPIPE_REMOTE_KEY: &str = "SCREENPIPE_REMOTE_KEY";
/// Remote path for LAN sync.
pub const SCREENPIPE_REMOTE_PATH: &str = "SCREENPIPE_REMOTE_PATH";
/// Base URL of the user's AI gateway (bring your own).
pub const SCREENPIPE_AI_GATEWAY_URL: &str = "SCREENPIPE_AI_GATEWAY_URL";
/// Key for the user's AI gateway.
pub const SCREENPIPE_AI_GATEWAY_KEY: &str = "SCREENPIPE_AI_GATEWAY_KEY";

// ---------------------------------------------------------------------------
// Pipes and agents (given to agent child processes)
// ---------------------------------------------------------------------------

/// Directory pipe trajectories are written to (JSONL, opt-in).
pub const SCREENPIPE_TRAJECTORIES_DIR: &str = "SCREENPIPE_TRAJECTORIES_DIR";
/// How many trajectory runs to keep.
pub const SCREENPIPE_TRAJECTORIES_MAX_RUNS: &str = "SCREENPIPE_TRAJECTORIES_MAX_RUNS";
/// Port of the tools HTTP server offered to ACP agents.
pub const SCREENPIPE_TOOLS_HTTP_PORT: &str = "SCREENPIPE_TOOLS_HTTP_PORT";
/// Session id of the agent run.
pub const SCREENPIPE_SESSION_ID: &str = "SCREENPIPE_SESSION_ID";
/// Set when the process is a sub-agent.
pub const SCREENPIPE_SUBAGENT: &str = "SCREENPIPE_SUBAGENT";
/// Pid of the parent process; children exit when it goes away.
pub const SCREENPIPE_PARENT_PID: &str = "SCREENPIPE_PARENT_PID";
/// Name of the pipe a child process runs for.
pub const SCREENPIPE_PIPE_NAME: &str = "SCREENPIPE_PIPE_NAME";
/// Path of a pre-resolved `screenpipe` CLI binary.
pub const SCREENPIPE_CLI: &str = "SCREENPIPE_CLI";
/// Path of the workflow video CLI.
pub const SCREENPIPE_VIDEO_CLI: &str = "SCREENPIPE_VIDEO_CLI";
/// Chat session the agent belongs to.
pub const SCREENPIPE_CHAT_SESSION_ID: &str = "SCREENPIPE_CHAT_SESSION_ID";
/// Address of the chat control channel.
pub const SCREENPIPE_CHAT_CONTROL_ADDR: &str = "SCREENPIPE_CHAT_CONTROL_ADDR";
/// Token for the chat control channel.
pub const SCREENPIPE_CHAT_CONTROL_TOKEN: &str = "SCREENPIPE_CHAT_CONTROL_TOKEN";
/// Turns the chat control channel off.
pub const SCREENPIPE_CHAT_CONTROL_DISABLED: &str = "SCREENPIPE_CHAT_CONTROL_DISABLED";
/// Pi's configuration directory (default `<data dir>/pi-agent/pi-config`).
/// Read to override the default and given to Pi processes.
pub const SCREENPIPE_PI_AGENT_DIR: &str = "SCREENPIPE_PI_AGENT_DIR";
/// Freenpipe addition: `1` makes Windows x64 run Pi with bun's baseline
/// (non-AVX2) build even when the CPU has AVX2. For testing that path.
pub const SCREENPIPE_FORCE_BASELINE_BUN: &str = "SCREENPIPE_FORCE_BASELINE_BUN";
/// Freenpipe addition: `1` leaves the installed Pi runtime unpatched and
/// removes the runtime patch if it was applied.
pub const SCREENPIPE_DISABLE_PI_PATCHES: &str = "SCREENPIPE_DISABLE_PI_PATCHES";
/// Home directory searched for AI tool configs (`.claude`, `.codex`,
/// `.cursor`, `.gemini`, `.openclaw`, `.hermes`) when auto-connecting MCP.
pub const SCREENPIPE_AI_TOOLS_HOME: &str = "SCREENPIPE_AI_TOOLS_HOME";

/// ACP runtime: agent id.
pub const SCREENPIPE_ACP_ID: &str = "SCREENPIPE_ACP_ID";
/// ACP runtime: working directory.
pub const SCREENPIPE_ACP_CWD: &str = "SCREENPIPE_ACP_CWD";
/// ACP runtime: command to launch.
pub const SCREENPIPE_ACP_COMMAND: &str = "SCREENPIPE_ACP_COMMAND";
/// ACP runtime: arguments as a JSON array.
pub const SCREENPIPE_ACP_ARGS_JSON: &str = "SCREENPIPE_ACP_ARGS_JSON";
/// ACP runtime: extra environment as a JSON object.
pub const SCREENPIPE_ACP_ENV_JSON: &str = "SCREENPIPE_ACP_ENV_JSON";
/// ACP runtime: authentication method.
pub const SCREENPIPE_ACP_AUTH_METHOD: &str = "SCREENPIPE_ACP_AUTH_METHOD";
/// ACP runtime: system prompt.
pub const SCREENPIPE_ACP_SYSTEM_PROMPT: &str = "SCREENPIPE_ACP_SYSTEM_PROMPT";
/// ACP runtime: session configuration as JSON.
pub const SCREENPIPE_ACP_SESSION_CONFIG_JSON: &str = "SCREENPIPE_ACP_SESSION_CONFIG_JSON";
/// ACP runtime: allowed tools.
pub const SCREENPIPE_ACP_TOOL_ALLOWLIST: &str = "SCREENPIPE_ACP_TOOL_ALLOWLIST";
/// ACP runtime: the user's MCP servers as JSON.
pub const SCREENPIPE_ACP_USER_MCP_JSON: &str = "SCREENPIPE_ACP_USER_MCP_JSON";
/// ACP runtime: session to resume.
pub const SCREENPIPE_ACP_RESUME_SESSION_ID: &str = "SCREENPIPE_ACP_RESUME_SESSION_ID";
/// ACP runtime: run without a user present.
pub const SCREENPIPE_ACP_UNATTENDED: &str = "SCREENPIPE_ACP_UNATTENDED";
/// ACP runtime: the wrapped command (internal).
pub const SCREENPIPE_INTERNAL_ACP_WRAPPED_COMMAND: &str = "SCREENPIPE_INTERNAL_ACP_WRAPPED_COMMAND";
/// ACP runtime: the wrapped command's arguments as JSON (internal).
pub const SCREENPIPE_INTERNAL_ACP_WRAPPED_ARGS_JSON: &str =
    "SCREENPIPE_INTERNAL_ACP_WRAPPED_ARGS_JSON";

// ---------------------------------------------------------------------------
// MCP
// ---------------------------------------------------------------------------

/// Transport of the portable MCP server.
pub const SCREENPIPE_MCP_TRANSPORT: &str = "SCREENPIPE_MCP_TRANSPORT";
/// Host of the portable MCP server.
pub const SCREENPIPE_MCP_HOST: &str = "SCREENPIPE_MCP_HOST";
/// Port of the portable MCP server.
pub const SCREENPIPE_MCP_PORT: &str = "SCREENPIPE_MCP_PORT";
/// Path of the portable MCP server.
pub const SCREENPIPE_MCP_PATH: &str = "SCREENPIPE_MCP_PATH";
/// Name of the MCP client.
pub const SCREENPIPE_MCP_CLIENT: &str = "SCREENPIPE_MCP_CLIENT";
/// MCP servers a pipe session may call.
pub const SCREENPIPE_MCP_SERVER_ALLOWLIST: &str = "SCREENPIPE_MCP_SERVER_ALLOWLIST";

// ---------------------------------------------------------------------------
// Pi runtime (names defined by Pi and its loader, set on Pi processes)
// ---------------------------------------------------------------------------

/// Pi's configuration directory; set to the isolated `pi-config` directory.
pub const PI_CODING_AGENT_DIR: &str = "PI_CODING_AGENT_DIR";
/// `1` turns off Pi's own check for newer releases (the runtime pins Pi).
pub const PI_SKIP_VERSION_CHECK: &str = "PI_SKIP_VERSION_CHECK";
/// `0` makes jiti compile extensions instead of using native TypeScript
/// loading.
pub const JITI_TRY_NATIVE: &str = "JITI_TRY_NATIVE";
/// `0` keeps Pi's install telemetry off.
pub const PI_TELEMETRY: &str = "PI_TELEMETRY";

/// The Pi runtime names above. They keep Pi's own spelling, so they are
/// listed apart from [`ALL`].
pub const PI_RUNTIME: &[&str] = &[
    PI_CODING_AGENT_DIR,
    PI_SKIP_VERSION_CHECK,
    JITI_TRY_NATIVE,
    PI_TELEMETRY,
];

/// Every name above, in declaration order.
pub const ALL: &[&str] = &[
    SCREENPIPE_DATA_DIR,
    SCREENPIPE_PORT,
    SCREENPIPE_FOCUS_PORT,
    SCREENPIPE_API_KEY,
    SCREENPIPE_API_AUTH_KEY,
    SCREENPIPE_LOCAL_API_URL,
    SCREENPIPE_LOCAL_API_PORT,
    SCREENPIPE_LOCAL_API_KEY,
    SCREENPIPE_API_URL,
    SCREENPIPE_API_BASE_URL,
    SCREENPIPE_WEB_URL,
    SCREENPIPE_TRUSTED_HOSTS,
    SCREENPIPE_IGNORE_DB_LOCK,
    SCREENPIPE_ENCRYPT_STORE,
    SCREENPIPE_STARTUP_AUTHENTICATION_STATUS,
    SCREENPIPE_SKIP_ONBOARDING,
    SCREENPIPE_RELAUNCH_HOME_VISIBLE,
    SCREENPIPE_E2E_SEED,
    SCREENPIPE_DISTRIBUTION,
    SCREENPIPE_DIST,
    SCREENPIPE_FD_LIMIT,
    SCREENPIPE_KEEP_NORMAL_PRIORITY,
    SCREENPIPE_LOG,
    SCREENPIPE_NO_UPDATE_CHECK,
    SCREENPIPE_UPDATER_MANIFEST_URL,
    SCREENPIPE_NO_REMINDERS,
    SCREENPIPE_BUN_PATH,
    SCREENPIPE_FFMPEG_PATH,
    SCREENPIPE_ENABLE_MDNS,
    SCREENPIPE_DISABLE_TELEMETRY,
    SCREENPIPE_ANALYTICS_ID,
    SCREENPIPE_RESOURCE_BREAKDOWN,
    SAVE_RESOURCE_USAGE,
    SCREENPIPE_SUPPORT_ID,
    SCREENPIPE_TELEMETRY_ID,
    SCREENPIPE_TELEMETRY_HOST_APP,
    SCREENPIPE_TELEMETRY_HOST_VERSION,
    SCREENPIPE_TELEMETRY_CUSTOMER_ID,
    SCREENPIPE_TELEMETRY_DEPLOYMENT_ID,
    SCREENPIPE_HOST_APP,
    SCREENPIPE_HOST_VERSION,
    SCREENPIPE_CUSTOMER_ID,
    SCREENPIPE_ORG_ID,
    SCREENPIPE_DEPLOYMENT_ID,
    SCREENPIPE_DIRECTML,
    SCREENPIPE_COMPACTION_ENCODER,
    SCREENPIPE_FILTER_PII,
    SCREENPIPE_AUDIO_EXCLUSIONS_PATH,
    SCREENPIPE_EMBEDDER,
    SCREENPIPE_EMBEDDER_VERSION,
    SCREENPIPE_SKIP_ONNX_DOWNLOAD,
    SCREENPIPE_ENABLE_EXPERIMENTAL_RFDETR_MLX,
    SCREENPIPE_PYANNOTE_BASE_URL,
    SCREENPIPE_PYANNOTE_DIR,
    SCREENPIPE_PII_REDACTOR_REPO,
    SCREENPIPE_PII_TEXT_REDACTOR_REPO,
    SCREENPIPE_PII_IMAGE_REDACTOR_REPO,
    SCREENPIPE_PRIVACY_FILTER_API_KEY,
    SCREENPIPE_PRIVACY_FILTER_ENCLAVE,
    SCREENPIPE_PRIVACY_FILTER_REPO,
    SCREENPIPE_PRIVACY_FILTER_URL,
    SCREENPIPE_MEETING_DETECTOR,
    SCREENPIPE_MEETING_STREAMING_PROVIDER,
    SCREENPIPE_MEETING_TRANSCRIPTION_MODEL,
    SCREENPIPE_MEETING_TRANSCRIPTION_LANGUAGE,
    SCREENPIPE_MEETING_REALTIME_URL,
    SCREENPIPE_MEETING_DEEPGRAM_API_KEY,
    SCREENPIPE_MEETING_DEEPGRAM_LIVE_URL,
    SCREENPIPE_MEETING_CLOUD_TOKEN,
    SCREENPIPE_MEETING_LOCAL_SPEAKER_NAME,
    SCREENPIPE_EVENT_CLASSIFIER_URL,
    SCREENPIPE_EVENT_CLASSIFIER_KEY,
    SCREENPIPE_OAUTH_TIMEOUT_SECS,
    SCREENPIPE_SYNC_TOKEN,
    SCREENPIPE_SYNC_PASSWORD,
    SCREENPIPE_REMOTE_HOST,
    SCREENPIPE_REMOTE_USER,
    SCREENPIPE_REMOTE_KEY,
    SCREENPIPE_REMOTE_PATH,
    SCREENPIPE_AI_GATEWAY_URL,
    SCREENPIPE_AI_GATEWAY_KEY,
    SCREENPIPE_TRAJECTORIES_DIR,
    SCREENPIPE_TRAJECTORIES_MAX_RUNS,
    SCREENPIPE_TOOLS_HTTP_PORT,
    SCREENPIPE_SESSION_ID,
    SCREENPIPE_SUBAGENT,
    SCREENPIPE_PARENT_PID,
    SCREENPIPE_PIPE_NAME,
    SCREENPIPE_CLI,
    SCREENPIPE_VIDEO_CLI,
    SCREENPIPE_CHAT_SESSION_ID,
    SCREENPIPE_CHAT_CONTROL_ADDR,
    SCREENPIPE_CHAT_CONTROL_TOKEN,
    SCREENPIPE_CHAT_CONTROL_DISABLED,
    SCREENPIPE_PI_AGENT_DIR,
    SCREENPIPE_FORCE_BASELINE_BUN,
    SCREENPIPE_DISABLE_PI_PATCHES,
    SCREENPIPE_AI_TOOLS_HOME,
    SCREENPIPE_ACP_ID,
    SCREENPIPE_ACP_CWD,
    SCREENPIPE_ACP_COMMAND,
    SCREENPIPE_ACP_ARGS_JSON,
    SCREENPIPE_ACP_ENV_JSON,
    SCREENPIPE_ACP_AUTH_METHOD,
    SCREENPIPE_ACP_SYSTEM_PROMPT,
    SCREENPIPE_ACP_SESSION_CONFIG_JSON,
    SCREENPIPE_ACP_TOOL_ALLOWLIST,
    SCREENPIPE_ACP_USER_MCP_JSON,
    SCREENPIPE_ACP_RESUME_SESSION_ID,
    SCREENPIPE_ACP_UNATTENDED,
    SCREENPIPE_INTERNAL_ACP_WRAPPED_COMMAND,
    SCREENPIPE_INTERNAL_ACP_WRAPPED_ARGS_JSON,
    SCREENPIPE_MCP_TRANSPORT,
    SCREENPIPE_MCP_HOST,
    SCREENPIPE_MCP_PORT,
    SCREENPIPE_MCP_PATH,
    SCREENPIPE_MCP_CLIENT,
    SCREENPIPE_MCP_SERVER_ALLOWLIST,
];

/// Reads a variable and returns it only when it is set to a non-blank value.
pub fn non_empty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn names_are_unique_and_well_formed() {
        let mut seen = HashSet::new();
        for name in ALL {
            assert!(seen.insert(*name), "{name} is listed twice");
            assert!(
                name.starts_with("SCREENPIPE_") || *name == SAVE_RESOURCE_USAGE,
                "{name} lacks the SCREENPIPE_ prefix"
            );
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
                "{name} is not an upper-case identifier"
            );
        }
    }

    #[test]
    fn names_added_by_the_target_are_present() {
        // A sample of the variables the parity target introduced (shell-025).
        for name in [
            "SCREENPIPE_SKIP_ONBOARDING",
            "SCREENPIPE_RELAUNCH_HOME_VISIBLE",
            "SCREENPIPE_DISABLE_TELEMETRY",
            "SCREENPIPE_DIRECTML",
            "SCREENPIPE_COMPACTION_ENCODER",
            "SCREENPIPE_TRAJECTORIES_DIR",
            "SCREENPIPE_TOOLS_HTTP_PORT",
            "SCREENPIPE_ACP_UNATTENDED",
            "SCREENPIPE_MCP_SERVER_ALLOWLIST",
            "SCREENPIPE_CHAT_CONTROL_TOKEN",
            "SCREENPIPE_AI_TOOLS_HOME",
            "SCREENPIPE_FFMPEG_PATH",
        ] {
            assert!(ALL.contains(&name), "{name} missing from env::ALL");
        }
    }

    #[test]
    fn every_name_is_documented() {
        let doc = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/env.md");
        let text = std::fs::read_to_string(&doc).expect("docs/env.md is readable");
        for name in ALL.iter().chain(PI_RUNTIME) {
            assert!(
                text.contains(&format!("`{name}`")),
                "{name} is not documented in docs/env.md"
            );
        }
    }

    #[test]
    fn non_empty_ignores_blank_values() {
        let key = "SCREENPIPE_W1_03_ENV_PROBE";
        std::env::set_var(key, "  ");
        assert_eq!(non_empty(key), None);
        std::env::set_var(key, "x");
        assert_eq!(non_empty(key).as_deref(), Some("x"));
        std::env::remove_var(key);
        assert_eq!(non_empty(key), None);
    }
}
