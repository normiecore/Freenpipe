// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

import { describe, expect, test } from "bun:test";
import {
  CLOUD_PRESET_MIGRATION_FLAG,
  DEFAULT_OLLAMA_URL,
  PRIVACY_DEFAULTS_MIGRATION_FLAG,
  applyFreenpipeMigrations,
  gatewayUrl,
  isGatewayProvider,
  makeDefaultPresets,
  normalizeGatewayUrl,
  pendingFreenpipeMigrations,
  pickGatewayModel,
  resolveAudioEngine,
} from "../ai-gateway";
import { createDefaultSettingsObject } from "../hooks/use-settings";

const VENDOR_HOSTS = /screenpipe\.com|screenpi\.pe|tinfoil\.(dev|sh)/i;

describe("fresh-install settings", () => {
  test("the default preset is a local Ollama (OpenAI-compatible) preset", () => {
    const settings = createDefaultSettingsObject();
    const presets = settings.aiPresets as any[];
    expect(presets).toHaveLength(1);
    const [preset] = presets;
    expect(preset.provider).toBe("native-ollama");
    expect(preset.url).toBe(DEFAULT_OLLAMA_URL);
    expect(preset.defaultPreset).toBe(true);
    expect(preset.model).toBe("");
    expect(isGatewayProvider(preset.provider)).toBe(false);
  });

  test("analytics, remote PII and media analysis are off; transcription is local", () => {
    const settings = createDefaultSettingsObject() as any;
    expect(settings.analyticsEnabled).toBe(false);
    expect(settings.piiBackend).toBe("local");
    expect(settings.cloudMediaAnalysisEnabled).toBe(false);
    expect(settings.aiGatewayUrl).toBe("");
    expect(settings.audioTranscriptionEngine).toBe("whisper-large-v3-turbo-quantized");
    expect(settings.meetingLiveTranscriptionProvider).toBe("selected-engine");
  });

  test("fresh settings need no migration and mention no vendor host", () => {
    const settings = createDefaultSettingsObject();
    const { changed } = applyFreenpipeMigrations(settings as any, {
      gatewayConfigured: false,
    });
    expect(changed).toBe(false);
    expect(JSON.stringify(settings)).not.toMatch(VENDOR_HOSTS);
  });

  test("makeDefaultPresets returns a fresh copy every time", () => {
    const a = makeDefaultPresets();
    a[0].model = "mutated";
    expect(makeDefaultPresets()[0].model).toBe("");
  });
});

describe("one-time migrations of a screenpipe store", () => {
  const screenpipeStore = () => ({
    piiBackend: "tinfoil",
    analyticsEnabled: true,
    enhancedAI: true,
    cloudMediaAnalysisEnabled: true,
    aiPresets: [
      {
        id: "chat",
        provider: "screenpipe-cloud",
        url: "",
        model: "claude-opus-4-8",
        defaultPreset: true,
        prompt: "keep it short",
        maxContextChars: 200000,
      },
      {
        id: "legacy",
        provider: "pi",
        url: "",
        model: "auto",
        defaultPreset: false,
        prompt: "",
        maxContextChars: 200000,
      },
      {
        id: "mine",
        provider: "openai",
        url: "https://api.openai.com/v1",
        model: "gpt-4o",
        apiKey: "sk-test",
        defaultPreset: false,
        prompt: "",
        maxContextChars: 128000,
      },
    ],
  });

  test("a stored piiBackend 'tinfoil' migrates to 'local'", () => {
    const { settings, changed } = applyFreenpipeMigrations(screenpipeStore(), {
      gatewayConfigured: false,
    });
    expect(changed).toBe(true);
    expect(settings.piiBackend).toBe("local");
    expect((settings as any)[PRIVACY_DEFAULTS_MIGRATION_FLAG]).toBe(true);
  });

  test("analytics, enhanced AI and media analysis are switched off", () => {
    const { settings } = applyFreenpipeMigrations(screenpipeStore(), {
      gatewayConfigured: true,
    });
    expect(settings.analyticsEnabled).toBe(false);
    expect(settings.enhancedAI).toBe(false);
    expect(settings.cloudMediaAnalysisEnabled).toBe(false);
  });

  test("without a gateway, hosted presets become local Ollama presets", () => {
    const input = screenpipeStore();
    const { settings } = applyFreenpipeMigrations(input, { gatewayConfigured: false });
    const [chat, legacy, mine] = settings.aiPresets as any[];
    expect(chat).toMatchObject({
      id: "chat",
      provider: "native-ollama",
      url: DEFAULT_OLLAMA_URL,
      model: "",
      defaultPreset: true,
      prompt: "keep it short",
    });
    expect(legacy.provider).toBe("native-ollama");
    expect(mine).toEqual(input.aiPresets[2]);
    expect((settings as any)[CLOUD_PRESET_MIGRATION_FLAG]).toBe(true);
    // The input object is left untouched.
    expect(input.aiPresets[0].provider).toBe("screenpipe-cloud");
  });

  test("with a gateway, hosted presets stay and legacy 'pi' is renamed", () => {
    const { settings } = applyFreenpipeMigrations(screenpipeStore(), {
      gatewayConfigured: true,
    });
    const [chat, legacy] = settings.aiPresets as any[];
    expect(chat.provider).toBe("screenpipe-cloud");
    expect(chat.model).toBe("claude-opus-4-8");
    expect(legacy.provider).toBe("screenpipe-cloud");
  });

  test("migrations run once; later user choices are respected", () => {
    const first = applyFreenpipeMigrations(screenpipeStore(), { gatewayConfigured: false });
    const reselected = { ...first.settings, piiBackend: "tinfoil", analyticsEnabled: true };
    const second = applyFreenpipeMigrations(reselected, { gatewayConfigured: false });
    expect(second.changed).toBe(false);
    expect(second.settings.piiBackend).toBe("tinfoil");
  });
});

describe("gateway helpers", () => {
  test("gateway URLs must be absolute http(s) URLs", () => {
    expect(normalizeGatewayUrl(" http://127.0.0.1:4000/v1/ ")).toBe("http://127.0.0.1:4000/v1");
    expect(normalizeGatewayUrl("https://gw.example.test")).toBe("https://gw.example.test");
    expect(normalizeGatewayUrl("")).toBeNull();
    expect(normalizeGatewayUrl("localhost:4000")).toBeNull();
    expect(normalizeGatewayUrl("ftp://gw.example.test")).toBeNull();
    expect(gatewayUrl("http://h/v1/", "/models")).toBe("http://h/v1/models");
  });

  test("the gateway model comes from the gateway preset", () => {
    expect(pickGatewayModel([])).toBe("auto");
    expect(
      pickGatewayModel([
        ...makeDefaultPresets(),
        {
          id: "gw",
          provider: "screenpipe-cloud",
          url: "",
          model: "qwen2.5-vl",
          defaultPreset: false,
          prompt: "",
          maxContextChars: 1,
        },
      ]),
    ).toBe("qwen2.5-vl");
  });

  test("pending migrations are reported per flag", () => {
    expect(pendingFreenpipeMigrations({})).toEqual({ presets: true, privacy: true });
    expect(
      pendingFreenpipeMigrations({ [CLOUD_PRESET_MIGRATION_FLAG]: true }),
    ).toEqual({ presets: false, privacy: true });
    expect(
      pendingFreenpipeMigrations({
        [CLOUD_PRESET_MIGRATION_FLAG]: true,
        [PRIVACY_DEFAULTS_MIGRATION_FLAG]: true,
      }),
    ).toEqual({ presets: false, privacy: false });
  });

  test("gateway transcription falls back to local whisper without a gateway and key", () => {
    const base = { audioTranscriptionEngine: "screenpipe-cloud", aiGatewayApiKey: "" };
    expect(resolveAudioEngine(base, null)).toEqual({
      requested: "screenpipe-cloud",
      active: "whisper-large-v3-turbo-quantized",
      fallbackReason: "gatewayNotConfigured",
    });
    expect(resolveAudioEngine(base, "http://127.0.0.1:4000/v1").fallbackReason).toBe(
      "gatewayNotConfigured",
    );
    expect(
      resolveAudioEngine({ ...base, aiGatewayApiKey: "k" }, "http://127.0.0.1:4000/v1"),
    ).toEqual({ requested: "screenpipe-cloud", active: "screenpipe-cloud", fallbackReason: null });
    // A key only in the engine's environment (reported by the backend) counts too.
    expect(
      resolveAudioEngine(base, "http://127.0.0.1:4000/v1", true),
    ).toEqual({ requested: "screenpipe-cloud", active: "screenpipe-cloud", fallbackReason: null });
    expect(resolveAudioEngine(base, null, true).fallbackReason).toBe("gatewayNotConfigured");
    expect(
      resolveAudioEngine({ audioTranscriptionEngine: "deepgram", deepgramApiKey: "" }, null)
        .fallbackReason,
    ).toBe("missingDeepgramKey");
    expect(
      resolveAudioEngine({ audioTranscriptionEngine: "parakeet" }, null).fallbackReason,
    ).toBeNull();
  });
});
