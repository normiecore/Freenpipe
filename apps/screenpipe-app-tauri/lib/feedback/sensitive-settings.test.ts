// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

import { describe, expect, it } from "vitest";
import rule from "./sensitive-keys.json";
import vectors from "./sensitive-keys.test-vectors.json";
import {
  MASKED_VALUE,
  isSensitiveSettingsKey,
  maskSensitiveSettings,
  normalizeSettingsKey,
} from "./sensitive-settings";

describe("sensitive settings key rule", () => {
  it("normalizes case and separators", () => {
    expect(normalizeSettingsKey("OPENAI-API_Key")).toBe("openaiapikey");
    expect(normalizeSettingsKey("data dir")).toBe("datadir");
  });

  it.each(vectors.sensitiveKeys)("treats %s as sensitive", (key) => {
    expect(isSensitiveSettingsKey(key)).toBe(true);
  });

  it.each(vectors.plainKeys)("leaves %s alone", (key) => {
    expect(isSensitiveSettingsKey(key)).toBe(false);
  });

  it("matches every fragment of the rule on its own", () => {
    for (const fragment of rule.fragments) {
      expect(isSensitiveSettingsKey(`my_${fragment}_value`)).toBe(true);
    }
  });

  it("never masks the listed exceptions", () => {
    for (const key of rule.exceptions) {
      expect(isSensitiveSettingsKey(key)).toBe(false);
    }
  });
});

describe("maskSensitiveSettings", () => {
  it.each(vectors.cases.map((c) => [c.name, c] as const))("%s", (_name, c) => {
    expect(maskSensitiveSettings(c.input)).toEqual(c.expected);
  });

  it("does not modify its input", () => {
    const input = { apiKey: "abc", nested: { password: "pw" } };
    const before = JSON.stringify(input);
    maskSensitiveSettings(input);
    expect(JSON.stringify(input)).toBe(before);
  });

  it("drops values JSON cannot represent", () => {
    const input = { keep: 1, fn: () => 1, gone: undefined, token: "t" };
    expect(maskSensitiveSettings(input)).toEqual({ keep: 1, token: MASKED_VALUE });
  });

  it("leaves no secret value anywhere in the serialized output", () => {
    const secret = "very-secret-value-0123";
    const settings = {
      aiPresets: [{ apiKey: secret, provider: "openai" }],
      deepgramApiKey: secret,
      user: { token: secret },
      pipes: { "obsidian-sync": { vaultPath: secret } },
    };
    expect(JSON.stringify(maskSensitiveSettings(settings))).not.toContain(secret);
  });
});
