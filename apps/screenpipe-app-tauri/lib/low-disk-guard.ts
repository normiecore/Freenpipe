// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

/**
 * Text and logic behind the two power/disk switches: the low-disk recording
 * guard (`stopRecordingOnLowDisk`, storage settings) and keep-awake
 * (`keepComputerAwake`, recording settings).
 */

/**
 * Label of the `stopRecordingOnLowDisk` switch. The low-disk notices
 * (`GUARD_SWITCH_LABEL` in `src-tauri/src/disk_pressure.rs`) name the switch
 * by this text; a Rust test checks that the two match.
 */
export const LOW_DISK_GUARD_LABEL = "Pause recording on low disk";

/** Label of the `keepComputerAwake` switch. */
export const KEEP_AWAKE_LABEL = "Keep computer awake";

/** Bytes as a short size with binary units, the way the notices print it ("2.0 GiB"). */
export function formatBinaryBytes(bytes: number): string {
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  if (!Number.isFinite(bytes) || bytes < 1024) {
    return `${Math.max(0, Math.floor(Number.isFinite(bytes) ? bytes : 0))} B`;
  }
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(1)} ${units[unit]}`;
}

/**
 * One-line description under the low-disk switch. `thresholdBytes` comes from
 * `get_low_disk_guard_config`; `null` while it loads or when it failed.
 */
export function lowDiskGuardDescription(thresholdBytes: number | null | undefined): string {
  if (typeof thresholdBytes !== "number" || thresholdBytes <= 0) {
    return "Pause recording while the disk is nearly full; it resumes once there is room";
  }
  return `Pause recording at ${formatBinaryBytes(thresholdBytes)} free or less; it resumes once there is room`;
}

/** A command result in the shape the generated bindings return. */
export type CommandResult = { status: "ok"; data: unknown } | { status: "error"; error: string };

/**
 * Turn keep-awake on or off and save the setting. Turning it on is saved only
 * once the hold is taken, so a failure leaves the switch off (and the engine
 * undoes an enable it could not confirm in time). Turning it off
 * is always saved, so a hold that could not be released is not taken again at
 * the next start.
 */
export async function applyKeepAwake(
  enabled: boolean,
  setKeepAwake: (enabled: boolean) => Promise<CommandResult>,
  save: (enabled: boolean) => void | Promise<void>,
): Promise<{ ok: true } | { ok: false; error: string }> {
  let result: CommandResult;
  try {
    result = await setKeepAwake(enabled);
  } catch (e) {
    result = { status: "error", error: e instanceof Error ? e.message : String(e) };
  }
  if (result.status === "error") {
    if (!enabled) await save(false);
    return { ok: false, error: String(result.error) };
  }
  await save(enabled);
  return { ok: true };
}
