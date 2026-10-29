// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

import { describe, it, expect } from "vitest";
import { readFileSync } from "fs";
import { join } from "path";
import { cliCandidates } from "./cli-locator";

describe("cliCandidates", () => {
  it("prefers SCREENPIPE_CLI, then the install folder of the bundled script", () => {
    const list = cliCandidates({
      platform: "win32",
      home: "C:\\Users\\me",
      env: {
        SCREENPIPE_CLI: "D:\\dev\\screenpipe.exe",
        LOCALAPPDATA: "C:\\Users\\me\\AppData\\Local",
      },
      scriptPath: "C:\\Program Files\\Handvel\\mcp\\screenpipe-mcp.js",
    });
    expect(list[0]).toBe("D:\\dev\\screenpipe.exe");
    expect(list[1]).toBe("C:\\Program Files\\Handvel\\screenpipe.exe");
    expect(list).toContain("C:\\Users\\me\\AppData\\Local\\Handvel\\screenpipe.exe");
  });

  it("ignores a relative SCREENPIPE_CLI and returns only absolute paths", () => {
    const list = cliCandidates({
      platform: "darwin",
      home: "/Users/me",
      env: { SCREENPIPE_CLI: "screenpipe" },
      scriptPath: "/Applications/Handvel.app/Contents/Resources/mcp/screenpipe-mcp.js",
    });
    expect(list).not.toContain("screenpipe");
    expect(list.every((p) => p.startsWith("/"))).toBe(true);
    expect(list).toContain("/Applications/Handvel.app/Contents/MacOS/screenpipe");
  });

  it("never points at a package runner", () => {
    const list = cliCandidates({ platform: "linux", home: "/home/me", env: {} });
    expect(list.some((p) => /npx|bunx|@latest/.test(p))).toBe(false);
  });
});

describe("API key discovery source", () => {
  it("does not resolve the CLI from a package registry", () => {
    const src = readFileSync(join(__dirname, "index.ts"), "utf8");
    expect(src).not.toMatch(/screenpipe(-mcp)?@latest/);
    expect(src).not.toMatch(/["'`]npx["'`]|npx screenpipe/);
  });
});
