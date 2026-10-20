// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

/**
 * Bring-your-own AI gateway: fresh-install defaults, the one-time migrations
 * for stores written by screenpipe, and small pure helpers shared by the
 * settings hook and the AI, privacy and recording settings pages.
 *
 * The `screenpipe-cloud` provider id is kept so old stores and pipes load,
 * but it now means "the AI gateway the user configured" (`aiGatewayUrl`, or
 * `SCREENPIPE_AI_GATEWAY_URL` on the engine side). There is no hosted
 * default anywhere: without a gateway every gateway-only feature is hidden.
 *
 * Nothing in this file imports Tauri, so it runs under `bun test`. The Rust
 * side (`src-tauri/src/store.rs`) applies the same migrations under the same
 * flags; whichever loads the store first does the work.
 */

/** Provider id that resolves to the user's AI gateway. */
export const GATEWAY_PROVIDER = "screenpipe-cloud";

/** Local Ollama server, the fresh-install AI provider. */
export const DEFAULT_OLLAMA_URL = "http://localhost:11434/v1";

/** Id of the fresh-install preset. */
export const DEFAULT_PRESET_ID = "default";

/** Store flag: hosted-provider presets were migrated. */
export const CLOUD_PRESET_MIGRATION_FLAG = "_freenpipeCloudPresetMigrated";

/** Store flag: PII backend, analytics and media-analysis defaults applied. */
export const PRIVACY_DEFAULTS_MIGRATION_FLAG = "_freenpipePrivacyDefaultsApplied";

/** Local engine used when a remote transcription engine cannot run. */
export const FALLBACK_TRANSCRIPTION_ENGINE = "whisper-large-v3-turbo-quantized";

/** The subset of an AI preset these helpers read and write. */
export interface PresetShape {
  id: string;
  provider: string;
  url: string;
  model: string;
  defaultPreset: boolean;
  prompt: string;
  maxContextChars: number;
  maxTokens?: number;
  apiKey?: string;
}

/**
 * Presets for a fresh install: one local Ollama preset. The model is left
 * empty on purpose; onboarding and Settings > AI ask the user to pick one.
 */
export function makeDefaultPresets(): PresetShape[] {
  return [
    {
      id: DEFAULT_PRESET_ID,
      provider: "native-ollama",
      url: DEFAULT_OLLAMA_URL,
      model: "",
      maxContextChars: 200000,
      defaultPreset: true,
      prompt: "",
    },
  ];
}

/** Whether a preset provider means "the user's AI gateway". */
export function isGatewayProvider(provider: string | null | undefined): boolean {
  return provider === GATEWAY_PROVIDER || provider === "pi";
}

/** `piiBackend` values that select a remote enclave. */
export function isRemotePiiBackend(value: unknown): boolean {
  return value === "tinfoil" || value === "cloud" || value === "enclave";
}

/** Trim, drop trailing slashes and accept only absolute http(s) URLs. */
export function normalizeGatewayUrl(raw: string | null | undefined): string | null {
  const value = raw?.trim().replace(/\/+$/, "");
  if (!value) return null;
  if (!/^https?:\/\/[^/\s]+(\/\S*)?$/i.test(value)) return null;
  return value;
}

/** Append a path to a gateway base with exactly one slash between them. */
export function gatewayUrl(base: string, path: string): string {
  return `${base.replace(/\/+$/, "")}/${path.replace(/^\/+/, "")}`;
}

/** The model to use for a one-off call through the gateway. */
export function pickGatewayModel(presets: readonly PresetShape[] | null | undefined): string {
  const gatewayPresets = (presets ?? []).filter((p) => isGatewayProvider(p?.provider));
  const preferred = gatewayPresets.find((p) => p.defaultPreset) ?? gatewayPresets[0];
  return preferred?.model?.trim() || "auto";
}

/**
 * Which one-time migrations a loaded settings object still needs. Callers
 * skip the gateway lookup (a Tauri call) when the preset step has run.
 */
export function pendingFreenpipeMigrations(settings: Record<string, any>): {
  presets: boolean;
  privacy: boolean;
} {
  return {
    presets: !settings[CLOUD_PRESET_MIGRATION_FLAG],
    privacy: !settings[PRIVACY_DEFAULTS_MIGRATION_FLAG],
  };
}

export interface MigrationContext {
  /** An AI gateway URL is configured (setting or engine environment). */
  gatewayConfigured: boolean;
}

/**
 * Apply the one-time Freenpipe migrations to a settings object loaded from a
 * store. Returns a new object (the input is not modified) and whether
 * anything changed, so the caller knows to persist it.
 *
 * - Without a gateway, presets on the hosted provider (`screenpipe-cloud`
 *   or legacy `pi`) become local Ollama presets. Ids, prompts and the
 *   default flag survive so pipes still resolve them; the vendor model name
 *   is cleared for the user to re-pick. With a gateway they stay, and a
 *   legacy `pi` provider is renamed to `screenpipe-cloud`.
 * - A remote PII backend goes back to `local`, the legacy analytics flag
 *   goes off, enhanced AI goes off and cloud media analysis goes off. The
 *   user can turn remote options back on after configuring their own
 *   endpoints.
 */
export function applyFreenpipeMigrations<T extends Record<string, any>>(
  input: T,
  ctx: MigrationContext,
): { settings: T; changed: boolean } {
  const settings: Record<string, any> = { ...input };
  let changed = false;

  if (!settings[CLOUD_PRESET_MIGRATION_FLAG]) {
    if (Array.isArray(settings.aiPresets)) {
      settings.aiPresets = settings.aiPresets.map((preset: any) => {
        if (!preset || !isGatewayProvider(preset.provider)) return preset;
        if (ctx.gatewayConfigured) {
          return preset.provider === "pi"
            ? { ...preset, provider: GATEWAY_PROVIDER }
            : preset;
        }
        const { apiKey: _dropped, ...rest } = preset;
        return {
          ...rest,
          provider: "native-ollama",
          url: DEFAULT_OLLAMA_URL,
          model: "",
        };
      });
    }
    settings[CLOUD_PRESET_MIGRATION_FLAG] = true;
    changed = true;
  }

  if (!settings[PRIVACY_DEFAULTS_MIGRATION_FLAG]) {
    if (isRemotePiiBackend(settings.piiBackend)) {
      settings.piiBackend = "local";
    }
    settings.analyticsEnabled = false;
    // Enhanced AI sends recent activity to the default preset's provider;
    // an imported store turns it back on only by the user's choice.
    settings.enhancedAI = false;
    settings.cloudMediaAnalysisEnabled = false;
    settings[PRIVACY_DEFAULTS_MIGRATION_FLAG] = true;
    changed = true;
  }

  return { settings: settings as T, changed };
}

/**
 * Whether gateway transcription can run: a gateway URL plus a key, either the
 * saved `aiGatewayApiKey` or one the backend reports (environment).
 */
export function isGatewaySttReady(
  gatewayUrl: string | null,
  savedKey: string | null | undefined,
  backendKeyConfigured: boolean,
): boolean {
  return Boolean(gatewayUrl) && (Boolean(savedKey?.trim()) || backendKeyConfigured);
}

export type AudioEngineFallbackReason = "gatewayNotConfigured" | "missingDeepgramKey";

export interface AudioEngineResolution {
  requested: string;
  active: string;
  fallbackReason: AudioEngineFallbackReason | null;
}

/**
 * Which transcription engine actually runs, mirroring the Rust
 * `audio_engine_resolution`. `screenpipe-cloud` needs a gateway URL and a
 * gateway key; Deepgram needs the user's Deepgram key; local engines always
 * run.
 *
 * `gatewayKeyConfigured` is the backend's answer (`get_ai_gateway_key_configured`):
 * it also counts a key the engine reads from `SCREENPIPE_AI_GATEWAY_KEY`,
 * which the settings object cannot see.
 */
export function resolveAudioEngine(
  settings: {
    audioTranscriptionEngine: string;
    deepgramApiKey?: string | null;
    aiGatewayApiKey?: string | null;
  },
  gatewayUrl: string | null,
  gatewayKeyConfigured = false,
): AudioEngineResolution {
  const requested = settings.audioTranscriptionEngine;
  const fallback = FALLBACK_TRANSCRIPTION_ENGINE;
  const gatewayReady = isGatewaySttReady(gatewayUrl, settings.aiGatewayApiKey, gatewayKeyConfigured);
  const hasDeepgramKey = Boolean(
    settings.deepgramApiKey && settings.deepgramApiKey !== "default",
  );

  if (requested === GATEWAY_PROVIDER && !gatewayReady) {
    return { requested, active: fallback, fallbackReason: "gatewayNotConfigured" };
  }
  if (requested === "deepgram" && !hasDeepgramKey) {
    return { requested, active: fallback, fallbackReason: "missingDeepgramKey" };
  }
  return { requested, active: requested, fallbackReason: null };
}
