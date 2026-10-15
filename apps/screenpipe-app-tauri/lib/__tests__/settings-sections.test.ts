// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

import { describe, expect, it } from "vitest";
import {
  DEFAULT_SETTINGS_SECTION,
  RETIRED_SETTINGS_SECTIONS,
  SETTINGS_SECTION_IDS,
  isSettingsSectionId,
  parseSettingsSection,
} from "../settings-sections";

describe("settings sections", () => {
  it("has no account, usage, team or referral section", () => {
    for (const retired of RETIRED_SETTINGS_SECTIONS) {
      expect(SETTINGS_SECTION_IDS as readonly string[]).not.toContain(retired);
      expect(isSettingsSectionId(retired)).toBe(false);
    }
  });

  it("sends links to retired sections to the default section", () => {
    for (const retired of RETIRED_SETTINGS_SECTIONS) {
      expect(parseSettingsSection(retired)).toBe(DEFAULT_SETTINGS_SECTION);
    }
  });

  it("keeps every real section and rejects junk", () => {
    for (const id of SETTINGS_SECTION_IDS) {
      expect(parseSettingsSection(id)).toBe(id);
    }
    expect(parseSettingsSection(null)).toBe(DEFAULT_SETTINGS_SECTION);
    expect(parseSettingsSection("")).toBe(DEFAULT_SETTINGS_SECTION);
    expect(parseSettingsSection("connections")).toBe(DEFAULT_SETTINGS_SECTION);
  });
});
