// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

// The team, organisation-policy and hosted-usage hooks used to talk to the
// vendor's services. In Freenpipe they answer for a personal install and must
// never issue a request.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  ENTERPRISE_UNAVAILABLE_MESSAGE,
  useEnterprisePolicy,
} from "../use-enterprise-policy";
import { TEAM_UNAVAILABLE_MESSAGE, useTeam } from "../use-team";
import {
  formatResetTime,
  messagesLeftForModel,
  shouldWarnLowQuota,
  useUsageStatus,
} from "../use-usage-status";

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
    expect(p.policy.aiPresetPolicy.managed_presets).toEqual([]);

    const res = await p.submitLicenseKey("ENT-0000-0000-0000-0000");
    expect(res).toEqual({ ok: false, error: ENTERPRISE_UNAVAILABLE_MESSAGE });
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("returns the same functions every time", () => {
    const a = useEnterprisePolicy();
    const b = useEnterprisePolicy();
    expect(a.isSectionHidden).toBe(b.isSectionHidden);
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

describe("useUsageStatus", () => {
  it("has no hosted allowance to report", () => {
    expect(useUsageStatus()).toBeNull();
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("shows no low-allowance chip without a snapshot", () => {
    expect(shouldWarnLowQuota(null, 2)).toBe(false);
    expect(messagesLeftForModel(null, 2)).toBeNull();
  });

  it("keeps the helper arithmetic for a supplied snapshot", () => {
    const snap = {
      tier: "subscribed" as const,
      used_today: 80,
      limit_today: 100,
      remaining: 20,
      resets_at: "",
    };
    expect(messagesLeftForModel(snap, 4)).toBe(5);
    expect(shouldWarnLowQuota(snap, 4)).toBe(true);
    expect(shouldWarnLowQuota({ ...snap, remaining: 90 }, 4)).toBe(false);
    expect(formatResetTime("")).toBe("");
    expect(formatResetTime("not a date")).toBe("");
  });
});
