// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

/**
 * Which cloud storage features the settings UI may offer.
 *
 * Cloud sync and cloud archive both need a sync server. Handvel ships
 * without a hosted one and has no setting for a self-hosted one yet, so both
 * are unavailable: the storage settings hide their tabs, and the Tauri
 * commands answer "not configured" (`sync_server_url()` in
 * `src-tauri/src/sync.rs` is the Rust side of the same switch). Local-network
 * sync is a separate feature and is not affected.
 */

export type StorageTab = "local" | "archive" | "sync";

export interface CloudStorageAvailability {
  /** Cross-device cloud sync. */
  sync: boolean;
  /** Archiving old data to cloud storage. */
  archive: boolean;
}

export const cloudStorageAvailability: Readonly<CloudStorageAvailability> =
  Object.freeze({ sync: false, archive: false });

/** Tabs of the storage settings section, in display order. */
export function visibleStorageTabs(
  availability: CloudStorageAvailability = cloudStorageAvailability,
): StorageTab[] {
  const tabs: StorageTab[] = ["local"];
  if (availability.archive) tabs.push("archive");
  if (availability.sync) tabs.push("sync");
  return tabs;
}

/** User-facing explanation shown if a cloud storage panel is reached anyway. */
export function cloudStorageUnavailableMessage(feature: "sync" | "archive"): string {
  const name = feature === "sync" ? "cloud sync" : "cloud archive";
  return `${name} is not available: there is no sync server configured. your data stays on this device.`;
}
