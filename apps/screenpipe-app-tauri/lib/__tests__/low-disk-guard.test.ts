// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

import { describe, expect, it } from "vitest";
import {
  LOW_DISK_GUARD_LABEL,
  applyKeepAwake,
  formatBinaryBytes,
  lowDiskGuardDescription,
  type CommandResult,
} from "../low-disk-guard";
import { isSettingsSectionId } from "../settings-sections";

const GIB = 1024 * 1024 * 1024;

describe("low-disk guard text", () => {
  it("prints sizes the way the notices do", () => {
    expect(formatBinaryBytes(2 * GIB)).toBe("2.0 GiB");
    expect(formatBinaryBytes(1.5 * GIB)).toBe("1.5 GiB");
    expect(formatBinaryBytes(10 * GIB)).toBe("10.0 GiB");
    expect(formatBinaryBytes(512)).toBe("512 B");
    expect(formatBinaryBytes(Number.NaN)).toBe("0 B");
  });

  it("quotes the threshold once it is known", () => {
    expect(lowDiskGuardDescription(2 * GIB)).toContain("2.0 GiB");
    for (const unknown of [null, undefined, 0]) {
      const text = lowDiskGuardDescription(unknown);
      expect(text).not.toContain("GiB");
      expect(text.length).toBeGreaterThan(0);
    }
  });

  it("keeps the label the notices name", () => {
    expect(LOW_DISK_GUARD_LABEL).toBe("Pause recording on low disk");
  });

  it("points the notice button at a real settings section", () => {
    expect(isSettingsSectionId(new URL("freenpipe://settings?section=storage").searchParams.get("section"))).toBe(true);
    expect(isSettingsSectionId(new URL("freenpipe://settings").searchParams.get("section"))).toBe(false);
  });
});

describe("applyKeepAwake", () => {
  const run = async (enabled: boolean, outcome: CommandResult | Error) => {
    const saved: boolean[] = [];
    const calls: boolean[] = [];
    const result = await applyKeepAwake(
      enabled,
      async (on) => {
        calls.push(on);
        if (outcome instanceof Error) throw outcome;
        return outcome;
      },
      (on) => {
        saved.push(on);
      },
    );
    return { result, saved, calls };
  };

  it("saves the setting once the hold is applied", async () => {
    for (const enabled of [true, false]) {
      const { result, saved, calls } = await run(enabled, { status: "ok", data: null });
      expect(result).toEqual({ ok: true });
      expect(calls).toEqual([enabled]);
      expect(saved).toEqual([enabled]);
    }
  });

  it("leaves the switch off when the hold cannot be taken", async () => {
    const { result, saved } = await run(true, { status: "error", error: "systemd-inhibit not found" });
    expect(result).toEqual({ ok: false, error: "systemd-inhibit not found" });
    expect(saved).toEqual([]);
  });

  it("still saves off when the release fails", async () => {
    const { result, saved } = await run(false, new Error("thread gone"));
    expect(result).toEqual({ ok: false, error: "thread gone" });
    expect(saved).toEqual([false]);
  });
});
