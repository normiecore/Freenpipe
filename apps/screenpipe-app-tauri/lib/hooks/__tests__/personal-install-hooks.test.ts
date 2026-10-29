// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

// The team and organisation-policy hooks used to talk to the vendor's
// services. In Handvel they answer for a personal install and must never
// issue a request. (The hosted-usage hook is gone entirely: W0-06 removed
// the model picker's allowance chip that used it.)

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  ENTERPRISE_UNAVAILABLE_MESSAGE,
  useEnterprisePolicy,
} from "../use-enterprise-policy";
import { TEAM_UNAVAILABLE_MESSAGE, useTeam } from "../use-team";

let fetchSpy: ReturnType<typeof vi.fn>;

beforeEach(() => {
  fetchSpy = vi.fn(() => Promise.reject(new Error("network must not be used")));
  vi.stubGlobal("fetch", fetchSpy);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("useEnterprisePolicy", () => {
  it("answers for a personal install", async () => {
    const p = useEnterprisePolicy();
    expect(p.isEnterprise).toBe(false);
    expect(p.needsLicenseKey).toBe(false);
    expect(p.isSectionHidden("account")).toBe(false);
    expect(p.isSectionHidden("ai")).toBe(false);
    expect(p.isSettingLocked("usePiiRemoval")).toBe(false);
    expect(p.getManagedValue("piiBackend")).toBeUndefined();
    expect(p.policy.hiddenSections).toEqual([]);
    expect(p.policy.lockedSettings).toEqual({});
    expect(p.policy.managedPipes).toEqual([]);
    expect(p.policy.managedSkills).toEqual([]);
    expect(p.policy.recordingAllowed).toBe(true);
    expect(p.policy.requireAccountLogin).toBe(false);
    expect(p.policy.aiPresetPolicy.managed_presets).toEqual([]);
    expect(p.isManagedDeployment).toBe(false);
    expect(p.historyAccessRestricted).toBe(false);
    expect(p.isPipeManaged("daily-summary")).toBe(false);

    const res = await p.submitLicenseKey("ENT-0000-0000-0000-0000");
    expect(res).toEqual({ ok: false, error: ENTERPRISE_UNAVAILABLE_MESSAGE });
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("returns the same functions every time", () => {
    const a = useEnterprisePolicy();
    const b = useEnterprisePolicy();
    expect(a.isSectionHidden).toBe(b.isSectionHidden);
    expect(a.isPipeManaged).toBe(b.isPipeManaged);
    expect(a.getManagedValue).toBe(b.getManagedValue);
    expect(a.policy).toBe(b.policy);
  });
});

describe("useTeam", () => {
  it("reports no team and refuses to share", async () => {
    const t = useTeam();
    expect(t.team).toBeNull();
    expect(t.role).toBeNull();
    expect(t.members).toEqual([]);
    await expect(t.pushConfig("ai_provider", "x", {})).rejects.toThrow(TEAM_UNAVAILABLE_MESSAGE);
    expect(fetchSpy).not.toHaveBeenCalled();
  });
});
