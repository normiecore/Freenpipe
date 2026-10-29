// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

import { describe, expect, it } from "vitest";
import {
  cloudStorageAvailability,
  cloudStorageUnavailableMessage,
  visibleStorageTabs,
} from "../cloud-storage";

describe("cloud storage availability", () => {
  it("offers neither cloud sync nor cloud archive by default", () => {
    expect(cloudStorageAvailability).toEqual({ sync: false, archive: false });
    expect(Object.isFrozen(cloudStorageAvailability)).toBe(true);
  });

  it("shows only the local storage tab by default", () => {
    expect(visibleStorageTabs()).toEqual(["local"]);
  });

  it("adds archive and sync tabs only when they are available", () => {
    expect(visibleStorageTabs({ sync: false, archive: true })).toEqual(["local", "archive"]);
    expect(visibleStorageTabs({ sync: true, archive: true })).toEqual([
      "local",
      "archive",
      "sync",
    ]);
  });

  it("explains the unavailable state without naming a vendor service", () => {
    for (const feature of ["sync", "archive"] as const) {
      const message = cloudStorageUnavailableMessage(feature);
      expect(message).toContain("not available");
      expect(message).not.toMatch(/screenpi\.pe|screenpipe\.com|pro|subscri/i);
    }
  });
});
