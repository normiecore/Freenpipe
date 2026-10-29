// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

/**
 * App-update policy shape shared by the settings UI and the (inert)
 * enterprise-policy hook.
 *
 * The base app imported this from a proprietary directory that is not part
 * of the fork. Handvel has no managed deployments, so nothing fetches a
 * policy; this module only has to keep a valid, well-typed value around for
 * code paths that still read `settings.enterpriseAppUpdatePolicy`. The mode
 * set mirrors what `src-tauri/src/updates.rs` understands.
 */

export type { EnterpriseInstallMetadata } from "@/lib/utils/tauri";

export type EnterpriseAppUpdateMode = "screenpipe" | "auto_detect" | "mdm" | "manual";

export type EnterpriseAppUpdatePolicy = {
  /** Who drives updates: the app itself, auto-detected, MDM, or the user by hand. */
  mode: EnterpriseAppUpdateMode;
  /** Initial value of the auto-update switch. */
  default_auto_update: boolean;
  /** Whether the user may flip the auto-update switch. */
  allow_employee_override: boolean;
  /** Release channel name. */
  channel: string;
};

const UPDATE_MODES: readonly EnterpriseAppUpdateMode[] = [
  "screenpipe",
  "auto_detect",
  "mdm",
  "manual",
];

export const DEFAULT_ENTERPRISE_APP_UPDATE_POLICY: EnterpriseAppUpdatePolicy =
  Object.freeze<EnterpriseAppUpdatePolicy>({
    mode: "screenpipe",
    default_auto_update: true,
    allow_employee_override: true,
    channel: "stable",
  });

function isUpdateMode(value: string): value is EnterpriseAppUpdateMode {
  return (UPDATE_MODES as readonly string[]).includes(value);
}

/**
 * Coerce an arbitrary stored value into a complete policy. Each field is
 * type-checked on its own and falls back to the default when missing or of
 * the wrong type; the mode is compared case-insensitively (the Rust reader
 * lower-cases it too) and unknown modes fall back to "screenpipe".
 */
export function normalizeEnterpriseAppUpdatePolicy(raw: unknown): EnterpriseAppUpdatePolicy {
  const fallback = DEFAULT_ENTERPRISE_APP_UPDATE_POLICY;
  if (raw === null || typeof raw !== "object" || Array.isArray(raw)) {
    return { ...fallback };
  }
  const input = raw as Record<string, unknown>;

  const modeText = typeof input.mode === "string" ? input.mode.trim().toLowerCase() : "";
  const channelText = typeof input.channel === "string" ? input.channel.trim() : "";

  return {
    mode: isUpdateMode(modeText) ? modeText : fallback.mode,
    default_auto_update:
      typeof input.default_auto_update === "boolean"
        ? input.default_auto_update
        : fallback.default_auto_update,
    allow_employee_override:
      typeof input.allow_employee_override === "boolean"
        ? input.allow_employee_override
        : fallback.allow_employee_override,
    channel: channelText || fallback.channel,
  };
}

/** Short label for the settings card that explains who handles updates. */
export function describeEnterpriseUpdateMode(policy: EnterpriseAppUpdatePolicy): string {
  switch (policy.mode) {
    case "auto_detect":
      return "Updates follow your device's package manager when one is detected";
    case "mdm":
      return "Updates are handled by your device management tool";
    case "manual":
      return "Updates are installed by hand";
    case "screenpipe":
    default:
      return "The app checks for and installs its own updates";
  }
}
