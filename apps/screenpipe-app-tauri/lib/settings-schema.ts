// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

/**
 * Settings schema v2: defaults for the keys added since the base schema and
 * the load rules the settings hook applies to every stored settings object.
 *
 * The Rust side (`src-tauri/src/store.rs`, `SettingsStore`) models the same
 * keys and applies the same load rules under the same sentinel keys, so a
 * store is migrated once, by whichever side reads it first. Keys only the
 * frontend uses (activities, sidebar layout, notification preferences, ...)
 * live here alone and land in the Rust struct's `extra` map.
 *
 * Nothing in this file imports Tauri, so it runs under `bun test`.
 *
 * Vendor-only features are accepted for round-trip but kept off: remote
 * support-log collection is forced off, a Workflows-sharing record is
 * dropped and the remote-control policy keys are kept but never read.
 */

import type { DomainRule, UrlRule } from "./utils/tauri";

export type { DomainRule, UrlRule };

export type SettingsPlatformName = "windows" | "macos" | "linux" | string;

// ── Value sets ───────────────────────────────────────────────────────────

export type UpdateChannel = "stable" | "pre-release";

/** The two release channels; `beta` is the older name of `pre-release`. */
export function normalizeUpdateChannel(value: unknown): UpdateChannel {
  const v = typeof value === "string" ? value.trim().toLowerCase() : "";
  return v === "pre-release" || v === "prerelease" || v === "beta" ? "pre-release" : "stable";
}

export type AudioCaptureMode = "always" | "meetings-only" | "disabled";

/** Written for brand-new installs. */
export const NEW_INSTALL_AUDIO_CAPTURE_MODE: AudioCaptureMode = "meetings-only";

/**
 * The capture mode a stored value stands for. A store without the key (or
 * with an unknown value) predates the setting and keeps recording always.
 */
export function effectiveAudioCaptureMode(value: unknown): AudioCaptureMode {
  if (value === "always" || value === "meetings-only" || value === "disabled") return value;
  return "always";
}

export type AecMode = "off" | "screenpipe" | "macos" | "windows";

export interface AecSettings {
  aecMode: AecMode;
  screenpipeAecEnabled: boolean;
  macosInputVpioEnabled: boolean;
  windowsInputAecEnabled: boolean;
}

/**
 * The stored AEC keys for a chosen mode. The software canceller runs
 * everywhere; an OS canceller only on its own platform, anything else is off.
 */
export function aecSettingsFor(mode: AecMode, platform: SettingsPlatformName): AecSettings {
  const effective: AecMode =
    mode === "screenpipe" ||
    (mode === "macos" && platform === "macos") ||
    (mode === "windows" && platform === "windows")
      ? mode
      : "off";
  return {
    aecMode: effective,
    screenpipeAecEnabled: effective === "screenpipe",
    macosInputVpioEnabled: effective === "macos",
    windowsInputAecEnabled: effective === "windows",
  };
}

export type RecordingDetail = "auto" | "low_impact" | "balanced" | "more_detail";
export type SemanticContextMode = "memory" | "computerUse" | "both";

/**
 * A pattern string for one block-list entry (`ignoredUrls` holds legacy
 * strings and structured domain rules side by side).
 */
export function urlRulePattern(rule: UrlRule): string {
  return typeof rule === "string" ? rule : rule.domain.trim().toLowerCase().replace(/\.$/, "");
}

/** Pattern strings for an editor that only works on strings. */
export function urlRulePatterns(rules: readonly UrlRule[] | null | undefined): string[] {
  return (rules ?? []).map(urlRulePattern);
}

/**
 * Apply an edit made in a string-based editor to the stored block list.
 * `values` is the full list of patterns the editor now shows. Patterns that
 * are new are appended as strings; entries whose pattern is gone are
 * removed, structured or not; the others keep their stored shape. Matching
 * ignores case. Returns `null` when nothing changed.
 */
export function editUrlRules(
  rules: readonly UrlRule[] | null | undefined,
  values: string[],
): UrlRule[] | null {
  const current = [...(rules ?? [])];
  const shown = new Set(values.map((v) => v.toLowerCase()));
  const kept = current.filter((rule) => shown.has(urlRulePattern(rule).toLowerCase()));
  const known = new Set(kept.map((rule) => urlRulePattern(rule).toLowerCase()));
  const added: string[] = [];
  for (const value of values) {
    const key = value.toLowerCase();
    if (!known.has(key)) {
      known.add(key);
      added.push(value);
    }
  }
  if (added.length === 0 && kept.length === current.length) return null;
  return [...kept, ...added];
}

export type ShortcutOverlayAnchor = "top-center" | "right-center" | "bottom-center" | "left-center";

export type SidebarNavSection =
  | "home"
  | "meetings"
  | "timeline"
  | "activity"
  | "brain"
  | "pipes"
  | "connections";

export interface SidebarNavLayout {
  order: SidebarNavSection[];
  hidden: SidebarNavSection[];
}

export function defaultSidebarNavLayout(): SidebarNavLayout {
  return {
    order: ["home", "meetings", "timeline", "activity", "brain", "pipes", "connections"],
    hidden: ["brain"],
  };
}

export type LocalRetentionMode = "off" | "media" | "lean" | "all";

export interface QuietHours {
  enabled: boolean;
  /** Local time `HH:MM`; a range may wrap past midnight. */
  start: string;
  end: string;
}

/** `notificationPrefs`: the base switches plus the v2 additions. */
export interface NotificationPrefs {
  /** Master switch for every notification. */
  notificationsEnabled?: boolean;
  /** Epoch milliseconds until which notifications are held back; 0 = none. */
  snoozeUntil?: number;
  quietHours?: QuietHours;
  /** Pipes that may still notify while notifications are paused. */
  allowDuringPause?: string[];
  captureStalls: boolean;
  appUpdates: boolean;
  pipeSuggestions: boolean;
  pipeNotifications: boolean;
  displayChanges?: boolean;
  meetingReminders?: boolean;
  /** Seconds before a meeting starts at which the reminder fires. */
  meetingReminderLeadSeconds?: number;
  meetingLiveNotes?: boolean;
  audioCaptureStalled?: boolean;
  liveTranscriptStalled?: boolean;
  powerModeChanges?: boolean;
  mutedPipes: string[];
}

export function defaultNotificationPrefs(): NotificationPrefs {
  return {
    notificationsEnabled: true,
    snoozeUntil: 0,
    quietHours: { enabled: false, start: "22:00", end: "08:00" },
    allowDuringPause: [],
    captureStalls: false,
    appUpdates: true,
    pipeSuggestions: true,
    pipeNotifications: true,
    displayChanges: true,
    meetingReminders: true,
    meetingReminderLeadSeconds: 60,
    meetingLiveNotes: true,
    audioCaptureStalled: true,
    liveTranscriptStalled: true,
    powerModeChanges: true,
    mutedPipes: [],
  };
}

// ── Defaults ─────────────────────────────────────────────────────────────

export function starSessionShortcutFor(platform: SettingsPlatformName): string {
  return platform === "windows" ? "Alt+Shift+B" : "Control+Super+B";
}

/**
 * Defaults of the keys added by schema v2 (and of the base keys whose
 * default changed), for a brand-new install on `platform`.
 */
export function settingsV2Defaults(platform: SettingsPlatformName): Record<string, unknown> {
  return {
    // App keys
    uiLocale: "system",
    uiLocalizationEnabled: false,
    starSessionShortcut: starSessionShortcutFor(platform),
    shortcutOverlaySnoozedUntil: null,
    allowHidingShortcutOverlay: false,
    autoUpdate: true,
    updateChannel: "stable" satisfies UpdateChannel,
    remoteLogCollectionEnabled: false,
    remoteLogCollectionUserId: null,
    hideOverlayInScreenRecording: false,
    hideAppInScreenShare: true,
    stopRecordingOnLowDisk: true,
    headless: false,
    headlessRecordOnly: false,
    keepSearchAvailableAfterQuit: true,
    hideThinkingBlocks: true,
    // Recording keys
    audioCaptureMode: NEW_INSTALL_AUDIO_CAPTURE_MODE,
    experimentalMeetingPiggyback: true,
    alwaysRecordBluetoothMic: false,
    screenpipeAecEnabled: false,
    aecMode: "off" satisfies AecMode,
    filterMusic: true,
    disableScreenshots: false,
    enableSemanticContext: true,
    semanticContextMode: "memory" satisfies SemanticContextMode,
    includedUrls: [] as DomainRule[],
    enhancedIncognitoDetection: false,
    disableClickCapture: false,
    usePiiRemoval: true,
    redactAgentSessionSecrets: true,
    piiRedactionPseudonyms: false,
    recordingDetail: "auto" satisfies RecordingDetail,
    keepComputerAwake: false,
    // Frontend-only keys
    activitiesEnabled: false,
    activitiesIntervalMinutes: 15,
    firstRunGuideDone: false,
    sidebarNavLayout: defaultSidebarNavLayout(),
    hideDockIcon: false,
    notificationPrefs: defaultNotificationPrefs(),
    // Fresh settings never need the one-time music-filter step.
    [FILTER_MUSIC_SENTINEL]: true,
  };
}

/**
 * Privacy keys back-filled into stores that lack them: capture of typed text
 * and clipboard stays off and the AI redaction workers stay local. (screenpipe
 * picked the opposite for accounts eligible for its hosted redaction;
 * Handvel has no such accounts.)
 */
export const PRIVACY_BACKFILL: Readonly<Record<string, unknown>> = Object.freeze({
  disableClipboardCapture: true,
  disableKeyboardCapture: true,
  asyncPiiRedaction: false,
  asyncImagePiiRedaction: false,
  piiBackend: "local",
});

// ── Load rules ───────────────────────────────────────────────────────────

/** Sentinel: music filtering was switched on once. */
export const FILTER_MUSIC_SENTINEL = "filterMusicDefaultedOn";
/** Workflows-sharing record written by screenpipe; dropped on load. */
export const WORKFLOW_SHARING_KEY = "workflowSharing";
/** Enterprise-managed overlay written by screenpipe; dropped on load. */
export const ENTERPRISE_MANAGED_SETTINGS_KEY = "enterpriseManagedSettings";

function isRemotePiiBackend(value: unknown): boolean {
  return value === "tinfoil" || value === "cloud" || value === "enclave";
}

export interface SchemaV2Context {
  platform: SettingsPlatformName;
  /** Platform defaults (`createDefaultSettingsObject()`), used to back-fill
   *  missing array-valued keys. */
  defaults: Record<string, unknown>;
}

/**
 * Apply the schema v2 load rules to a stored settings object. Returns a new
 * object (the input is not modified) and whether anything changed.
 *
 * - Remote support-log collection is forced off; a Workflows-sharing record
 *   is dropped and the PII backend it had replaced comes back (or `local`);
 *   an enterprise-managed overlay is dropped. Remote-control keys stay as
 *   stored and are never read.
 * - `updateChannel` uses the current names.
 * - `filterMusicDefaultedOn`: music filtering is switched on once.
 * - Missing array-valued keys, the privacy keys and the star-session
 *   shortcut get their defaults.
 * - Existing notification preferences gain the v2 switches; a missing
 *   `meetingReminders` follows `meetingLiveNotes`.
 */
export function applySchemaV2Migrations<T extends Record<string, any>>(
  input: T,
  ctx: SchemaV2Context,
): { settings: T; changed: boolean } {
  const settings: Record<string, any> = { ...input };
  let changed = false;

  if (settings.remoteLogCollectionEnabled !== false) {
    if (settings.remoteLogCollectionEnabled !== undefined) changed = true;
    settings.remoteLogCollectionEnabled = false;
  }

  if (WORKFLOW_SHARING_KEY in settings) {
    const sharing = settings[WORKFLOW_SHARING_KEY];
    const prior = sharing && typeof sharing === "object" ? sharing.priorBackend : undefined;
    if (typeof prior === "string" && prior !== "" && !isRemotePiiBackend(prior)) {
      settings.piiBackend = prior;
    } else if (isRemotePiiBackend(settings.piiBackend)) {
      settings.piiBackend = "local";
    }
    delete settings[WORKFLOW_SHARING_KEY];
    changed = true;
  }

  if (ENTERPRISE_MANAGED_SETTINGS_KEY in settings) {
    delete settings[ENTERPRISE_MANAGED_SETTINGS_KEY];
    changed = true;
  }

  const channel = normalizeUpdateChannel(settings.updateChannel);
  if (settings.updateChannel !== channel) {
    settings.updateChannel = channel;
    changed = true;
  }

  if (!settings[FILTER_MUSIC_SENTINEL]) {
    settings.filterMusic = true;
    settings[FILTER_MUSIC_SENTINEL] = true;
    changed = true;
  }

  for (const [key, value] of Object.entries(ctx.defaults)) {
    if (Array.isArray(value) && !Array.isArray(settings[key])) {
      settings[key] = [...value];
      changed = true;
    }
  }

  for (const [key, value] of Object.entries(PRIVACY_BACKFILL)) {
    if (settings[key] === undefined || settings[key] === null) {
      settings[key] = value;
      changed = true;
    }
  }

  if (typeof settings.starSessionShortcut !== "string" || settings.starSessionShortcut.trim() === "") {
    settings.starSessionShortcut = starSessionShortcutFor(ctx.platform);
    changed = true;
  }

  const prefs = settings.notificationPrefs;
  if (prefs && typeof prefs === "object" && !Array.isArray(prefs)) {
    const filled: Record<string, unknown> = { ...prefs };
    let prefsChanged = false;
    if (filled.meetingReminders === undefined && typeof filled.meetingLiveNotes === "boolean") {
      filled.meetingReminders = filled.meetingLiveNotes;
      prefsChanged = true;
    }
    for (const [key, value] of Object.entries(defaultNotificationPrefs())) {
      if (filled[key] === undefined) {
        filled[key] = Array.isArray(value)
          ? [...value]
          : value && typeof value === "object"
            ? { ...value }
            : value;
        prefsChanged = true;
      }
    }
    if (prefsChanged) {
      settings.notificationPrefs = filled;
      changed = true;
    }
  }

  return { settings: settings as T, changed };
}

/**
 * The object to write to `store.bin`. The cached account token is left out
 * once the secret store holds it (`tokenInSecretStore`); otherwise it stays,
 * so it is never lost. Returns a new object.
 */
export function settingsForSave<T extends Record<string, any>>(
  settings: T,
  tokenInSecretStore: boolean,
): T {
  const user = settings.user;
  if (!tokenInSecretStore || !user || typeof user !== "object" || !user.token) {
    return settings;
  }
  return { ...settings, user: { ...user, token: null } };
}

/** A save must keep at least one AI preset. */
export function assertSavableSettings(settings: Record<string, any>): void {
  if (Array.isArray(settings.aiPresets) && settings.aiPresets.length === 0) {
    throw new Error("settings must keep at least one AI preset");
  }
}

/** Keys `resetSetting` never touches. */
export const NON_RESETTABLE_KEYS: ReadonlySet<string> = new Set(["deviceId"]);
