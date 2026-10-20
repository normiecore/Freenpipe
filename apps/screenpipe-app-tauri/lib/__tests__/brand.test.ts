// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
  ACCEPTED_DEEP_LINK_SCHEMES,
  PRODUCT_NAME,
  appDeepLink,
  deepLinkRoute,
  isAppDeepLink,
  isAppDeepLinkProtocol,
  isDeepLinkTo,
} from "../brand";

describe("deep-link schemes", () => {
  it("accepts the primary scheme and the screenpipe alias", () => {
    for (const url of [
      "freenpipe://frame/1",
      "freenpipe-beta://timeline",
      "screenpipe://meeting/12?live=0",
      " FreenPipe://settings",
    ]) {
      expect(isAppDeepLink(url), url).toBe(true);
    }
  });

  it("rejects web, file and malformed links", () => {
    for (const url of [
      "https://example.test/?u=freenpipe://x",
      "file:///C:/freenpipe/a.md",
      "freenpipe:frame/1",
      "screenpipe-beta://frame/1",
      "",
      null,
      undefined,
    ]) {
      expect(isAppDeepLink(url), String(url)).toBe(false);
      expect(deepLinkRoute(url)).toBeNull();
    }
  });

  it("matches routes independent of scheme", () => {
    expect(deepLinkRoute("screenpipe://meeting/12?live=1")).toBe("meeting/12?live=1");
    expect(deepLinkRoute("freenpipe://meeting/12?live=1")).toBe("meeting/12?live=1");
    expect(isDeepLinkTo("freenpipe://meeting/3", "meeting")).toBe(true);
    expect(isDeepLinkTo("screenpipe://meeting?id=3", "meeting")).toBe(true);
    expect(isDeepLinkTo("freenpipe://meetings", "meeting")).toBe(false);
    expect(isDeepLinkTo("https://example.test/meeting/3", "meeting")).toBe(false);
  });

  it("recognises parsed protocols", () => {
    expect(isAppDeepLinkProtocol(new URL("freenpipe://view?path=a").protocol)).toBe(true);
    expect(isAppDeepLinkProtocol(new URL("screenpipe://view?path=a").protocol)).toBe(true);
    expect(isAppDeepLinkProtocol("https:")).toBe(false);
  });

  it("builds links with the primary scheme", () => {
    expect(appDeepLink("frame/42")).toBe("freenpipe://frame/42");
    expect(appDeepLink("/timeline")).toBe("freenpipe://timeline");
    expect(PRODUCT_NAME).toBe("Freenpipe");
  });

  it("registers only Freenpipe schemes with the OS", () => {
    const dir = join(__dirname, "..", "..", "src-tauri");
    for (const file of ["tauri.conf.json", "tauri.prod.conf.json", "tauri.beta.conf.json"]) {
      const conf = JSON.parse(readFileSync(join(dir, file), "utf8"));
      const schemes: string[] = conf.plugins["deep-link"].desktop.schemes;
      expect(schemes.length, file).toBeGreaterThan(0);
      for (const scheme of schemes) {
        expect(scheme.startsWith("freenpipe"), `${file}: ${scheme}`).toBe(true);
        expect(ACCEPTED_DEEP_LINK_SCHEMES).toContain(scheme);
      }
      expect(conf.productName.startsWith("Freenpipe"), file).toBe(true);
      expect(conf.identifier.startsWith("com.normiecore.freenpipe"), file).toBe(true);
    }
  });
});
