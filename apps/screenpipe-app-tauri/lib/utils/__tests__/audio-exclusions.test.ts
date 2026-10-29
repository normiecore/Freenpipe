// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

import { describe, it, expect } from "vitest";
import {
  canAddAudioExclusion,
  exclusionPickerOptions,
  maxAudioExclusions,
  showAudioExclusionsPanel,
} from "../audio-exclusions";

describe("exclusionPickerOptions", () => {
  it("filters .exe files from Program Files on Windows", () => {
    expect(exclusionPickerOptions("windows")).toEqual({
      filters: [{ name: "Application", extensions: ["exe"] }],
      defaultPath: "C:\\Program Files",
    });
  });

  it("keeps the .app picker in /Applications on macOS", () => {
    expect(exclusionPickerOptions("macos")).toEqual({
      filters: [{ name: "Application", extensions: ["app"] }],
      defaultPath: "/Applications",
    });
  });
});

describe("exclusion limit", () => {
  it("allows a single app on Windows", () => {
    expect(maxAudioExclusions("windows")).toBe(1);
    expect(canAddAudioExclusion("windows", 0)).toBe(true);
    expect(canAddAudioExclusion("windows", 1)).toBe(false);
    expect(canAddAudioExclusion("windows", 3)).toBe(false);
  });

  it("has no limit on macOS", () => {
    expect(maxAudioExclusions("macos")).toBeNull();
    expect(canAddAudioExclusion("macos", 12)).toBe(true);
  });
});

describe("showAudioExclusionsPanel", () => {
  it("needs a per-app tap", () => {
    for (const platform of ["windows", "macos", "linux"]) {
      expect(
        showAudioExclusionsPanel({
          platform,
          tapAvailable: false,
          coreaudioSystemAudioEnabled: true,
        })
      ).toBe(false);
      expect(
        showAudioExclusionsPanel({
          platform,
          tapAvailable: null,
          coreaudioSystemAudioEnabled: true,
        })
      ).toBe(false);
    }
  });

  it("shows on Windows without the macOS CoreAudio toggle", () => {
    expect(
      showAudioExclusionsPanel({
        platform: "windows",
        tapAvailable: true,
        coreaudioSystemAudioEnabled: false,
      })
    ).toBe(true);
  });

  it("follows the CoreAudio toggle on macOS", () => {
    expect(
      showAudioExclusionsPanel({
        platform: "macos",
        tapAvailable: true,
        coreaudioSystemAudioEnabled: false,
      })
    ).toBe(false);
    expect(
      showAudioExclusionsPanel({
        platform: "macos",
        tapAvailable: true,
        coreaudioSystemAudioEnabled: true,
      })
    ).toBe(true);
  });
});
