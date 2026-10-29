// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

import { describe, expect, it } from "vitest";
import {
  DEFAULT_ENTERPRISE_APP_UPDATE_POLICY,
  describeEnterpriseUpdateMode,
  normalizeEnterpriseAppUpdatePolicy,
  type EnterpriseAppUpdatePolicy,
} from "../app-update-policy";

describe("app update policy", () => {
  it("falls back to the default for non-objects", () => {
    for (const raw of [undefined, null, 42, "manual", [], true]) {
      expect(normalizeEnterpriseAppUpdatePolicy(raw)).toEqual(
        DEFAULT_ENTERPRISE_APP_UPDATE_POLICY,
      );
    }
  });

  it("returns a fresh object, never the frozen default", () => {
    const policy = normalizeEnterpriseAppUpdatePolicy(undefined);
    expect(policy).not.toBe(DEFAULT_ENTERPRISE_APP_UPDATE_POLICY);
    expect(Object.isFrozen(DEFAULT_ENTERPRISE_APP_UPDATE_POLICY)).toBe(true);
  });

  it("keeps valid fields and lower-cases the mode", () => {
    expect(
      normalizeEnterpriseAppUpdatePolicy({
        mode: " MDM ",
        default_auto_update: false,
        allow_employee_override: false,
        channel: "beta",
      }),
    ).toEqual({
      mode: "mdm",
      default_auto_update: false,
      allow_employee_override: false,
      channel: "beta",
    });
  });

  it("repairs each bad field independently", () => {
    expect(
      normalizeEnterpriseAppUpdatePolicy({
        mode: "intune",
        default_auto_update: "yes",
        allow_employee_override: false,
        channel: "   ",
      }),
    ).toEqual({
      mode: "screenpipe",
      default_auto_update: true,
      allow_employee_override: false,
      channel: "stable",
    });
  });

  it("describes every mode with a non-empty label", () => {
    const modes: EnterpriseAppUpdatePolicy["mode"][] = [
      "screenpipe",
      "auto_detect",
      "mdm",
      "manual",
    ];
    const labels = modes.map((mode) =>
      describeEnterpriseUpdateMode({ ...DEFAULT_ENTERPRISE_APP_UPDATE_POLICY, mode }),
    );
    expect(labels.every((l) => l.length > 0)).toBe(true);
    expect(new Set(labels).size).toBe(modes.length);
  });
});
