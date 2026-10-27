// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

// Runs under both vitest and `bun test` (bun maps the vitest import).
import { describe, expect, it } from "vitest";
import { isPiBusyRefusal, waitForPiIdle, type PiBusyState } from "../pi-busy";

const busyError =
  "Pi start failed at stage 'waiting_for_pool': Pi session 'chat-1' is busy (2 queued prompt(s) are waiting). Let the current reply end, or stop the session first.";

describe("isPiBusyRefusal", () => {
  it("recognises the runtime's busy refusal in every error shape", () => {
    expect(isPiBusyRefusal(busyError)).toBe(true);
    expect(isPiBusyRefusal(new Error(busyError))).toBe(true);
    expect(isPiBusyRefusal({ status: "error", error: busyError })).toBe(true);
  });

  it("does not mistake other start failures for it", () => {
    for (const other of [
      "Pi start failed at stage 'requested': Project directory is required",
      "pi pool full (20 active sessions, all busy) — close one before opening a new chat",
      "the line is busy (try later)",
      "",
      null,
      undefined,
    ]) {
      expect(isPiBusyRefusal(other)).toBe(false);
    }
  });
});

describe("waitForPiIdle", () => {
  // A fake clock: every sleep moves time forward instead of waiting.
  function fakeClock() {
    let t = 0;
    return {
      now: () => t,
      sleep: async (ms: number) => {
        t += ms;
      },
    };
  }

  function states(...sequence: Array<PiBusyState | null | Error>) {
    let i = 0;
    const reads = { count: 0 };
    const read = async () => {
      reads.count += 1;
      const next = sequence[Math.min(i++, sequence.length - 1)];
      if (next instanceof Error) throw next;
      return next;
    };
    return { read, reads };
  }

  it("resolves once the session is idle", async () => {
    const clock = fakeClock();
    const { read, reads } = states(
      { running: true, busy: true },
      null,
      new Error("ipc down"),
      { running: true, busy: false },
    );
    expect(await waitForPiIdle(read, { ...clock, intervalMs: 500 })).toBe(true);
    expect(reads.count).toBe(4);
  });

  it("counts a stopped session as idle", async () => {
    const { read } = states({ running: false, busy: true });
    expect(await waitForPiIdle(read, fakeClock())).toBe(true);
  });

  it("gives up after the timeout", async () => {
    const clock = fakeClock();
    const { read, reads } = states({ running: true, busy: true });
    expect(await waitForPiIdle(read, { ...clock, timeoutMs: 3_000, intervalMs: 1_000 })).toBe(false);
    expect(reads.count).toBe(4);
  });
});
