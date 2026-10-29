// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

import { describe, expect, it } from "vitest";
import { isAccountDeepLink } from "../account-deep-link";

describe("isAccountDeepLink", () => {
  it("recognises the sign-in, subscription and purchase hand-backs", () => {
    for (const link of [
      "screenpipe://login?api_key=abc",
      "screenpipe://?api_key=abc",
      "screenpipe://settings?api_key=abc",
      "screenpipe://subscription-success?purchase_token=t",
      "screenpipe://x/subscription-success",
      "screenpipe://purchase-successful",
      "https://screenpi.pe/purchase-successful?session=1",
    ]) {
      expect(isAccountDeepLink(new URL(link)), link).toBe(true);
    }
  });

  it("leaves every other deep link alone", () => {
    for (const link of [
      "screenpipe://settings",
      "screenpipe://timeline?timestamp=2026-01-01T00:00:00Z",
      "screenpipe://frame/12345",
      "screenpipe://meeting/7?live=1",
      "screenpipe://auth/google-calendar?success=true",
      "screenpipe://onboarding",
      // A nested URL that happens to carry api_key= is not a sign-in link.
      "screenpipe://install-pipe?url=" +
        encodeURIComponent("https://example.test/pipe.md?api_key=x"),
    ]) {
      expect(isAccountDeepLink(new URL(link)), link).toBe(false);
    }
  });
});
