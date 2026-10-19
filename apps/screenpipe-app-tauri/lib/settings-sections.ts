// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

/**
 * The sections of the Settings window, in nav order, and how a `?section=`
 * value from a URL or deep link maps onto them.
 *
 * Upstream also had Account, Usage (hosted AI credits), Team and Referral
 * sections. They only fronted the vendor's sign-in, billing, team console and
 * referral programme, none of which exist in Freenpipe. Links that still
 * carry those ids land on the default section instead of a blank page.
 */

export const SETTINGS_SECTION_IDS = [
  "display",
  "general",
  "ai",
  "recording",
  "shortcuts",
  "notifications",
  "privacy",
  "storage",
  "speakers",
] as const;

export type SettingsSectionId = (typeof SETTINGS_SECTION_IDS)[number];

export const DEFAULT_SETTINGS_SECTION: SettingsSectionId = "display";

/** Section ids from upstream builds that have no Freenpipe counterpart. */
export const RETIRED_SETTINGS_SECTIONS = ["account", "usage", "team", "referral"] as const;

export function isSettingsSectionId(value: unknown): value is SettingsSectionId {
  return typeof value === "string" && (SETTINGS_SECTION_IDS as readonly string[]).includes(value);
}

/** Map a raw `?section=` value to a section that exists. */
export function parseSettingsSection(raw: string | null | undefined): SettingsSectionId {
  return isSettingsSectionId(raw) ? raw : DEFAULT_SETTINGS_SECTION;
}
