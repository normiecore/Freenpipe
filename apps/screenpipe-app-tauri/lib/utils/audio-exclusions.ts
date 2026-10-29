// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

// Platform rules for the "exclude apps from system audio" settings panel.
// macOS excludes any number of `.app` bundles through the CoreAudio Process
// Tap; Windows excludes one `.exe` at a time, because per-process WASAPI
// loopback can leave out a single process tree.

export interface AppPickerFilter {
  name: string;
  extensions: string[];
}

export interface AppPickerOptions {
  filters: AppPickerFilter[];
  defaultPath: string;
}

export function isWindowsPlatform(platform: string | null | undefined): boolean {
  return platform === "windows";
}

/** File-dialog options for choosing the app to exclude. */
export function exclusionPickerOptions(
  platform: string | null | undefined
): AppPickerOptions {
  if (isWindowsPlatform(platform)) {
    return {
      filters: [{ name: "Application", extensions: ["exe"] }],
      defaultPath: "C:\\Program Files",
    };
  }
  return {
    filters: [{ name: "Application", extensions: ["app"] }],
    defaultPath: "/Applications",
  };
}

/** Most apps the panel lets the user exclude, or `null` for no limit. */
export function maxAudioExclusions(
  platform: string | null | undefined
): number | null {
  return isWindowsPlatform(platform) ? 1 : null;
}

export function canAddAudioExclusion(
  platform: string | null | undefined,
  currentCount: number
): boolean {
  const max = maxAudioExclusions(platform);
  return max === null || currentCount < max;
}

/**
 * Whether the exclusions panel is shown. It needs a per-app tap (macOS
 * 14.4+, Windows build 20348+). On macOS the tap is also an opt-in backend,
 * so the panel follows that toggle; on Windows the loopback engine always
 * applies the list.
 */
export function showAudioExclusionsPanel(opts: {
  platform: string | null | undefined;
  tapAvailable: boolean | null;
  coreaudioSystemAudioEnabled: boolean;
}): boolean {
  if (!opts.tapAvailable) return false;
  if (isWindowsPlatform(opts.platform)) return true;
  return opts.platform === "macos" && opts.coreaudioSystemAudioEnabled;
}
