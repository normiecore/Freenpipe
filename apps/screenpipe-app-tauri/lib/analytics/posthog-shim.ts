// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

/**
 * Inert stand-in for `posthog-js` and `posthog-js/react`.
 *
 * Freenpipe sends no telemetry, so the analytics SDK is not installed.
 * `tsconfig.json` (and `vitest.config.ts`, which does not read tsconfig paths)
 * resolves both module ids to this file, so the existing
 * `import posthog from "posthog-js"` call sites keep compiling while every
 * call does nothing: nothing is recorded, queued or sent, feature flags are
 * always off, and the user always counts as opted out.
 *
 * Only the part of the SDK surface the app uses is modelled. Add a member
 * here (as a no-op) rather than reintroducing the real package.
 */

import { createElement, Fragment, type ReactNode } from "react";

type Properties = Record<string, any>;
type FeatureFlagsCallback = (
  flags: string[],
  variants: Record<string, string | boolean>,
) => void;

export interface PostHog {
  /** Always false: the client is never initialised. */
  __loaded: boolean;
  init(token?: string, config?: Properties, name?: string): void;
  capture(eventName: string, properties?: Properties | null, options?: Properties): void;
  identify(distinctId?: string, userPropertiesToSet?: Properties, userPropertiesToSetOnce?: Properties): void;
  alias(alias: string, original?: string): void;
  reset(resetDeviceId?: boolean): void;
  register(properties: Properties, days?: number): void;
  register_once(properties: Properties, defaultValue?: unknown, days?: number): void;
  unregister(property: string): void;
  group(groupType: string, groupKey: string, groupPropertiesToSet?: Properties): void;
  people: {
    set(properties: Properties | string, value?: unknown): void;
    set_once(properties: Properties | string, value?: unknown): void;
  };
  opt_in_capturing(options?: Properties): void;
  opt_out_capturing(): void;
  has_opted_in_capturing(): boolean;
  has_opted_out_capturing(): boolean;
  get_distinct_id(): string | undefined;
  isFeatureEnabled(flag: string, options?: Properties): boolean | undefined;
  getFeatureFlag(flag: string, options?: Properties): string | boolean | undefined;
  getFeatureFlagPayload(flag: string): unknown;
  /** Never calls back; returns an unsubscribe function that is also a no-op. */
  onFeatureFlags(callback: FeatureFlagsCallback): () => void;
  reloadFeatureFlags(): void;
}

function noop(): void {}

export const posthog: PostHog = {
  __loaded: false,
  init: noop,
  capture: noop,
  identify: noop,
  alias: noop,
  reset: noop,
  register: noop,
  register_once: noop,
  unregister: noop,
  group: noop,
  people: { set: noop, set_once: noop },
  opt_in_capturing: noop,
  opt_out_capturing: noop,
  has_opted_in_capturing: () => false,
  has_opted_out_capturing: () => true,
  get_distinct_id: () => undefined,
  isFeatureEnabled: () => false,
  getFeatureFlag: () => undefined,
  getFeatureFlagPayload: () => undefined,
  onFeatureFlags: () => noop,
  reloadFeatureFlags: noop,
};

export default posthog;

// ---- `posthog-js/react` surface -------------------------------------------

/** Renders its children; there is no client to provide. */
export function PostHogProvider({
  children,
}: {
  client?: unknown;
  apiKey?: string;
  options?: Properties;
  children?: ReactNode;
}) {
  return createElement(Fragment, null, children);
}

export function usePostHog(): PostHog {
  return posthog;
}

export function useFeatureFlagEnabled(_flag: string): boolean {
  return false;
}
