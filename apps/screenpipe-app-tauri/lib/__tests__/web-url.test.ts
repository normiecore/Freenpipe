// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

import { afterEach, describe, expect, it, vi } from "vitest";
import {
  FREENPIPE_REPO_URL,
  freenpipeLink,
  joinUrl,
  normalizeWebBase,
} from "../web-url";

describe("web-url switchboard", () => {
  afterEach(() => {
    vi.unstubAllEnvs();
    vi.resetModules();
  });

  it("has no website unless one is configured at build time", async () => {
    vi.stubEnv("NEXT_PUBLIC_SCREENPIPE_WEB_URL", "");
    vi.resetModules();
    const mod = await import("../web-url");
    expect(mod.screenpipeWebBase()).toBeNull();
    expect(mod.screenpipeWebUrl("/login")).toBeNull();
  });

  it("uses the configured website when set", async () => {
    vi.stubEnv("NEXT_PUBLIC_SCREENPIPE_WEB_URL", "http://localhost:3000/");
    vi.resetModules();
    const mod = await import("../web-url");
    expect(mod.screenpipeWebBase()).toBe("http://localhost:3000");
    expect(mod.screenpipeWebUrl("/api/user")).toBe("http://localhost:3000/api/user");
    expect(mod.screenpipeWebUrl("login")).toBe("http://localhost:3000/login");
  });

  it("rejects values that are not absolute http(s) URLs", () => {
    for (const bad of [undefined, null, "", "  ", "localhost:3000", "ftp://x", "https://", "http://a b"]) {
      expect(normalizeWebBase(bad)).toBeNull();
    }
    expect(normalizeWebBase(" https://example.test/base// ")).toBe("https://example.test/base");
  });

  it("joins with exactly one slash", () => {
    expect(joinUrl("https://h/", "/x")).toBe("https://h/x");
    expect(joinUrl("https://h", "x")).toBe("https://h/x");
    expect(joinUrl("https://h/", "")).toBe("https://h");
  });

  it("builds project links on the Handvel repository", () => {
    expect(freenpipeLink("repo")).toBe(FREENPIPE_REPO_URL);
    expect(freenpipeLink("releases")).toBe(`${FREENPIPE_REPO_URL}/releases`);
    expect(freenpipeLink("newIssue")).toBe(`${FREENPIPE_REPO_URL}/issues/new`);
  });
});
