// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

import { describe, expect, test } from "bun:test";
import { isMissingCommandError, resolveDirWith } from "../data-dirs";

const fallback = async () => "/home/someone/.screenpipe";

describe("resolveDirWith", () => {
  test("uses the directory the backend returns", async () => {
    const dir = await resolveDirWith(async () => ({ status: "ok", data: "/dev/data" }), fallback);
    expect(dir).toBe("/dev/data");
  });

  test("falls back outside Tauri, where the invoke throws", async () => {
    const dir = await resolveDirWith(async () => {
      throw new TypeError("window.__TAURI_INTERNALS__ is undefined");
    }, fallback);
    expect(dir).toBe("/home/someone/.screenpipe");
  });

  test("falls back when an older backend has no such command", async () => {
    const dir = await resolveDirWith(
      async () => ({ status: "error", error: "Command get_screenpipe_base_dir not found" }),
      fallback,
    );
    expect(dir).toBe("/home/someone/.screenpipe");
  });

  test("a backend error is reported, not replaced by the home directory", async () => {
    await expect(
      resolveDirWith(
        async () => ({
          status: "error",
          error: "failed to resolve the settings directory: permission denied",
        }),
        fallback,
      ),
    ).rejects.toThrow("permission denied");
    await expect(
      resolveDirWith(async () => ({ status: "ok", data: "" }), fallback),
    ).rejects.toThrow();
  });
});

describe("isMissingCommandError", () => {
  test("recognises Tauri's unknown-command rejections only", () => {
    expect(isMissingCommandError("Command get_active_data_dir not found")).toBe(true);
    expect(isMissingCommandError("get_active_data_dir not allowed. Command not found")).toBe(true);
    expect(isMissingCommandError("failed to resolve the data directory: not found")).toBe(false);
    expect(isMissingCommandError(new Error("Command x not found"))).toBe(false);
  });
});
