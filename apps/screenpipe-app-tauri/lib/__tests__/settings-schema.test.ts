// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
  FILTER_MUSIC_SENTINEL,
  aecSettingsFor,
  applySchemaV2Migrations,
  assertSavableSettings,
  defaultNotificationPrefs,
  editUrlRules,
  effectiveAudioCaptureMode,
  normalizeUpdateChannel,
  settingsForSave,
  settingsV2Defaults,
  urlRulePattern,
  urlRulePatterns,
  type UrlRule,
} from "../settings-schema";
import {
  createDefaultSettingsObject,
  migrateStoredSettings,
  type Settings,
  type SettingsLoadEnvironment,
} from "../hooks/use-settings";

/** The same screenpipe-shaped store the Rust store tests load. */
function screenpipeStore(): { settings: Record<string, any>; onboarding: Record<string, any> } {
  const path = join(import.meta.dir, "../../src-tauri/testdata/screenpipe-v2-store.json");
  return JSON.parse(readFileSync(path, "utf8"));
}

const windowsEnv: SettingsLoadEnvironment = {
  platform: () => "windows",
  isAiGatewayConfigured: async () => false,
};

async function load(stored: Record<string, any>) {
  return migrateStoredSettings(stored as Settings, windowsEnv);
}

describe("fresh-install defaults (settings schema v2)", () => {
  const d = createDefaultSettingsObject() as Record<string, any>;

  test("changed defaults: PII removal, auto-update and music filter are on", () => {
    expect(d.usePiiRemoval).toBe(true);
    expect(d.autoUpdate).toBe(true);
    expect(d.filterMusic).toBe(true);
  });

  test("vendor-only features are off and absent", () => {
    expect(d.remoteLogCollectionEnabled).toBe(false);
    expect(d.remoteLogCollectionUserId).toBeNull();
    expect("workflowSharing" in d).toBe(false);
    expect("enterpriseManagedSettings" in d).toBe(false);
    expect(d.analyticsEnabled).toBe(false);
  });

  test("new keys carry their documented defaults", () => {
    expect(d.uiLocale).toBe("system");
    expect(d.uiLocalizationEnabled).toBe(false);
    expect(d.updateChannel).toBe("stable");
    expect(d.audioCaptureMode).toBe("meetings-only");
    expect(d.experimentalMeetingPiggyback).toBe(true);
    expect(d.alwaysRecordBluetoothMic).toBe(false);
    expect(d.aecMode).toBe("off");
    expect(d.disableScreenshots).toBe(false);
    expect(d.recordingDetail).toBe("auto");
    expect(d.includedUrls).toEqual([]);
    expect(d.hideAppInScreenShare).toBe(true);
    expect(d.stopRecordingOnLowDisk).toBe(true);
    expect(d.keepSearchAvailableAfterQuit).toBe(true);
    expect(d.headless).toBe(false);
    expect(d.activitiesEnabled).toBe(false);
    expect(d.activitiesIntervalMinutes).toBe(15);
    expect(d.sidebarNavLayout).toEqual({
      order: ["home", "meetings", "timeline", "activity", "brain", "pipes", "connections"],
      hidden: ["brain"],
    });
    expect(d.hideThinkingBlocks).toBe(true);
    expect(d[FILTER_MUSIC_SENTINEL]).toBe(true);
    // Privacy-first capture defaults.
    expect(d.disableClipboardCapture).toBe(true);
    expect(d.disableKeyboardCapture).toBe(true);
    expect(d.piiBackend).toBe("local");
  });

  test("the star-session shortcut follows the platform", () => {
    expect(settingsV2Defaults("windows").starSessionShortcut).toBe("Alt+Shift+B");
    expect(settingsV2Defaults("macos").starSessionShortcut).toBe("Control+Super+B");
  });
});

describe("loading a store written by screenpipe v2.7.104", () => {
  test("remote log collection is off and Workflows sharing is gone after load", async () => {
    const stored = screenpipeStore().settings;
    expect(stored.remoteLogCollectionEnabled).toBe(true);
    expect(stored.workflowSharing).toBeDefined();

    const { settings, changed } = await load(stored);
    const s = settings as Record<string, any>;
    expect(changed).toBe(true);
    expect(s.remoteLogCollectionEnabled).toBe(false);
    expect("workflowSharing" in s).toBe(false);
    // The PII backend Workflows sharing had forced comes back.
    expect(s.piiBackend).toBe("local");
    // The stored object itself is not modified.
    expect(stored.workflowSharing).toBeDefined();
  });

  test("unknown and frontend-only keys survive the load", async () => {
    const stored = screenpipeStore().settings;
    const s = (await load(stored)).settings as Record<string, any>;
    expect(s.remoteControlPolicy).toEqual(stored.remoteControlPolicy);
    expect(s.remoteControlPreferences).toEqual(stored.remoteControlPreferences);
    expect(s.workflowSharingPromptSeen).toEqual(stored.workflowSharingPromptSeen);
    expect(s.localRetentionMode).toBe("lean");
    expect(s.activitiesIntervalMinutes).toBe(30);
    expect(s.ignoredUrls).toEqual(stored.ignoredUrls);
    expect(s.user.subscription_plan).toBe("pro");
    expect(s.updateChannel).toBe("pre-release");
    // screenpipe already ran the music-filter step; the user's choice stays.
    expect(s.filterMusic).toBe(false);
    // The imported store's enhanced AI is switched off once.
    expect(s.enhancedAI).toBe(false);
  });

  test("a coding-agent preset with an unknown provider is kept as stored", async () => {
    const stored = screenpipeStore().settings;
    const agent = stored.aiPresets.find((p: any) => p.provider === "acp");
    expect(agent?.acpAgent).toBeDefined();
    const s = (await load(stored)).settings as Record<string, any>;
    expect(s.aiPresets).toEqual(stored.aiPresets);
    expect(settingsForSave(s as Settings, true).aiPresets).toEqual(stored.aiPresets);
  });

  test("a second load changes nothing", async () => {
    const first = await load(screenpipeStore().settings);
    const second = await load(first.settings as Record<string, any>);
    expect(second.changed).toBe(false);
  });

  test("the account token is written back only until the secret store holds it", async () => {
    const { settings } = await load(screenpipeStore().settings);
    expect(settings.user?.token).toBe("opaque-account-token");
    expect(settingsForSave(settings, false).user?.token).toBe("opaque-account-token");
    const saved = settingsForSave(settings, true);
    expect(saved.user?.token).toBeNull();
    expect(saved.user?.email).toBe("someone@example.com");
    expect(settings.user?.token).toBe("opaque-account-token");
  });
});

describe("loading an older (base) store", () => {
  const baseStore = () => ({
    aiPresets: [
      {
        id: "default",
        provider: "native-ollama",
        url: "http://localhost:11434/v1",
        model: "",
        defaultPreset: true,
        prompt: "",
        maxContextChars: 200000,
      },
    ],
    deviceId: "device-1",
    platform: "windows",
    filterMusic: false,
    updateChannel: "beta",
    showChatShortcut: "Alt+L",
    startAudioShortcut: "Alt+Shift+A",
    stopAudioShortcut: "Alt+Shift+Z",
    usePiiRemoval: false,
    notificationPrefs: {
      captureStalls: false,
      appUpdates: true,
      pipeSuggestions: true,
      pipeNotifications: true,
      meetingLiveNotes: false,
      mutedPipes: ["noisy-pipe"],
    },
  });

  test("music filtering is switched on once, then left to the user", async () => {
    const first = (await load(baseStore())).settings as Record<string, any>;
    expect(first.filterMusic).toBe(true);
    expect(first[FILTER_MUSIC_SENTINEL]).toBe(true);
    const second = (await load({ ...first, filterMusic: false })).settings as Record<string, any>;
    expect(second.filterMusic).toBe(false);
  });

  test("gaps are filled without overriding stored choices", async () => {
    const s = (await load(baseStore())).settings as Record<string, any>;
    expect(s.updateChannel).toBe("pre-release");
    expect(s.starSessionShortcut).toBe("Alt+Shift+B");
    expect(s.includedUrls).toEqual([]);
    expect(s.disableKeyboardCapture).toBe(true);
    expect(s.disableClipboardCapture).toBe(true);
    expect(s.piiBackend).toBe("local");
    expect(s.remoteLogCollectionEnabled).toBe(false);
    // A stored choice is not a gap.
    expect(s.usePiiRemoval).toBe(false);
    // Notification preferences gain the new switches; reminders follow the
    // existing live-notes choice.
    expect(s.notificationPrefs.meetingReminders).toBe(false);
    expect(s.notificationPrefs.notificationsEnabled).toBe(true);
    expect(s.notificationPrefs.quietHours).toEqual({ enabled: false, start: "22:00", end: "08:00" });
    expect(s.notificationPrefs.mutedPipes).toEqual(["noisy-pipe"]);
    // A store without audioCaptureMode keeps recording always.
    expect(effectiveAudioCaptureMode(s.audioCaptureMode)).toBe("always");
  });
});

describe("schema v2 helpers", () => {
  test("release channel names", () => {
    expect(normalizeUpdateChannel("beta")).toBe("pre-release");
    expect(normalizeUpdateChannel("pre-release")).toBe("pre-release");
    expect(normalizeUpdateChannel("stable")).toBe("stable");
    expect(normalizeUpdateChannel(undefined)).toBe("stable");
  });

  test("AEC modes are validated per platform and drive the three flags", () => {
    expect(aecSettingsFor("windows", "windows")).toEqual({
      aecMode: "windows",
      screenpipeAecEnabled: false,
      macosInputVpioEnabled: false,
      windowsInputAecEnabled: true,
    });
    expect(aecSettingsFor("macos", "windows").aecMode).toBe("off");
    expect(aecSettingsFor("screenpipe", "linux").screenpipeAecEnabled).toBe(true);
  });

  test("audio capture mode of a stored value", () => {
    expect(effectiveAudioCaptureMode("meetings-only")).toBe("meetings-only");
    expect(effectiveAudioCaptureMode("disabled")).toBe("disabled");
    expect(effectiveAudioCaptureMode(undefined)).toBe("always");
    expect(effectiveAudioCaptureMode("sometimes")).toBe("always");
  });

  test("URL rule patterns", () => {
    expect(urlRulePattern("bank.example")).toBe("bank.example");
    expect(
      urlRulePattern({ domain: "Mail.Example.com.", includeSubdomains: true, excludedSubdomains: [] }),
    ).toBe("mail.example.com");
  });

  test("a string editor keeps structured rules it did not touch", () => {
    const stored: UrlRule[] = [
      "bank.example",
      { domain: "Mail.Example.com", includeSubdomains: true, excludedSubdomains: [] },
    ];
    expect(urlRulePatterns(stored)).toEqual(["bank.example", "mail.example.com"]);
    expect(urlRulePatterns(undefined)).toEqual([]);

    // Adding a pattern appends a string and keeps the domain rule as it was.
    const added = editUrlRules(stored, ["bank.example", "mail.example.com", "shop.example"]);
    expect(added).toEqual([stored[0], stored[1], "shop.example"]);

    // Removing the structured rule drops it.
    expect(editUrlRules(stored, ["bank.example"])).toEqual(["bank.example"]);

    // Same patterns in another case: nothing to save.
    expect(editUrlRules(stored, ["BANK.example", "mail.example.com"])).toBeNull();
  });

  test("a save keeps at least one AI preset", () => {
    expect(() => assertSavableSettings({ aiPresets: [] })).toThrow();
    expect(() => assertSavableSettings({ aiPresets: [{ id: "x" }] })).not.toThrow();
  });

  test("the migration helper is pure and idempotent", () => {
    const input = { workflowSharing: { priorBackend: "tinfoil" }, piiBackend: "tinfoil" };
    const ctx = { platform: "linux", defaults: { ignoredWindows: ["a"] } };
    const once = applySchemaV2Migrations(input, ctx);
    expect(once.settings.piiBackend).toBe("local");
    expect((once.settings as any).ignoredWindows).toEqual(["a"]);
    expect(input.workflowSharing).toBeDefined();
    expect(applySchemaV2Migrations(once.settings, ctx).changed).toBe(false);
  });

  test("notification defaults cover every switch", () => {
    const prefs = defaultNotificationPrefs();
    expect(prefs.captureStalls).toBe(false);
    expect(prefs.meetingReminders).toBe(true);
    expect(prefs.powerModeChanges).toBe(true);
    expect(prefs.snoozeUntil).toBe(0);
  });
});
