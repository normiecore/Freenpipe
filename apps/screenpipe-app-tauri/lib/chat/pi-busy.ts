// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

/**
 * The chat runtime refuses to restart a Pi session that is still busy (a
 * reply streaming, follow-ups queued, a compaction, an open extension
 * dialog). `pi_start` then fails with
 * `Pi start failed at stage 'waiting_for_pool': Pi session '<id>' is busy (<reason>). ...`
 * (see `busy_refusal` in `src-tauri/src/pi.rs`).
 *
 * A caller that only wanted to apply new settings (a preset switch) waits
 * until the session is idle and tries again; a caller that means to cut the
 * turn short stops the session first.
 */

/** The text of a command error, whatever shape it arrived in. */
function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "error" in error) {
    const inner = (error as { error?: unknown }).error;
    if (typeof inner === "string") return inner;
  }
  return String(error ?? "");
}

/** Whether `error` is the runtime's refusal to restart a busy session. */
export function isPiBusyRefusal(error: unknown): boolean {
  return /\bPi session '[^']*' is busy \(/.test(errorText(error));
}

/** The part of `pi_request_state`'s answer this module reads. */
export type PiBusyState = { running: boolean; busy: boolean };

export type WaitForPiIdleOptions = {
  /** Give up after this long (default 2 minutes). */
  timeoutMs?: number;
  /** Time between two state reads (default 1 s). */
  intervalMs?: number;
  /** Replaceable for tests. */
  sleep?: (ms: number) => Promise<void>;
  /** Replaceable for tests. */
  now?: () => number;
};

const defaultSleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/**
 * Read the session state until it is idle (not busy, or not running at
 * all). Resolves `true` once it is, `false` when `timeoutMs` passes first.
 * A failed read counts as "still busy" and is retried.
 */
export async function waitForPiIdle(
  readState: () => Promise<PiBusyState | null>,
  options: WaitForPiIdleOptions = {},
): Promise<boolean> {
  const timeoutMs = options.timeoutMs ?? 120_000;
  const intervalMs = options.intervalMs ?? 1_000;
  const sleep = options.sleep ?? defaultSleep;
  const now = options.now ?? Date.now;
  const deadline = now() + timeoutMs;
  for (;;) {
    let state: PiBusyState | null = null;
    try {
      state = await readState();
    } catch {
      state = null;
    }
    if (state && (!state.running || !state.busy)) return true;
    if (now() >= deadline) return false;
    await sleep(intervalMs);
  }
}
