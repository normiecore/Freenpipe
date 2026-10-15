// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

import { describe, expect, it, vi } from "vitest";
import { render, renderHook } from "@testing-library/react";
import posthogFromPackageId from "posthog-js";
import { PostHogProvider, usePostHog } from "posthog-js/react";
import posthog, { useFeatureFlagEnabled } from "../analytics/posthog-shim";

describe("posthog-js shim", () => {
  it("is what the posthog-js module ids resolve to", () => {
    expect(posthogFromPackageId).toBe(posthog);
    const { result } = renderHook(() => usePostHog());
    expect(result.current).toBe(posthog);
  });

  it("accepts every call the app makes and does no network I/O", () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch");
    posthog.init("token-placeholder", { api_host: "http://127.0.0.1:1" });
    posthog.capture("event", { a: 1 });
    posthog.capture("event", null);
    posthog.identify("someone", { email: "x@example.test" });
    posthog.alias("someone-else");
    posthog.people.set({ k: "v" });
    posthog.opt_in_capturing();
    posthog.opt_out_capturing();
    posthog.reset();
    expect(fetchSpy).not.toHaveBeenCalled();
    fetchSpy.mockRestore();
  });

  it("reports the user as opted out with every feature flag off", () => {
    posthog.opt_in_capturing();
    expect(posthog.has_opted_out_capturing()).toBe(true);
    expect(posthog.has_opted_in_capturing()).toBe(false);
    expect(posthog.get_distinct_id()).toBeUndefined();
    expect(posthog.isFeatureEnabled("cloud-sync")).toBe(false);
    expect(posthog.getFeatureFlag("cloud-sync")).toBeUndefined();
    const { result } = renderHook(() => useFeatureFlagEnabled("cloud-sync"));
    expect(result.current).toBe(false);
  });

  it("never fires feature-flag callbacks", () => {
    const callback = vi.fn();
    const unsubscribe = posthog.onFeatureFlags(callback);
    posthog.reloadFeatureFlags();
    unsubscribe();
    expect(callback).not.toHaveBeenCalled();
  });

  it("PostHogProvider renders its children unchanged", () => {
    const { getByText } = render(
      <PostHogProvider client={posthog}>
        <span>child content</span>
      </PostHogProvider>,
    );
    expect(getByText("child content")).toBeTruthy();
  });
});
