// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

// The enterprise and plan-gate commands are registered so a frontend written
// against screenpipe's bindings compiles. Each one answers for a personal
// install. These tests drive the generated bindings through a stand-in for
// the Tauri IPC bridge that answers exactly as the Rust stubs do (their own
// tests in src-tauri/src/enterprise_policy.rs pin those values), and check
// that every binding reaches the right command with the right argument names
// and hands the answer back in the right shape.
//
// Only describe/it/expect are used, so the file runs under vitest (CI) and
// under `bun test` alike.

import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { commands } from "@/lib/utils/tauri";

type Call = { cmd: string; args: Record<string, unknown> };

// What each Rust command sends back over IPC. `Result<(), String>` commands
// resolve to null on success.
const RUST_ANSWERS: Record<string, unknown> = {
  is_history_access_restricted: false,
  apply_enterprise_ui_visibility: false,
  get_enterprise_host_identity: { machine_id_hash: null, os_user_id_hash: null },
  list_managed_team_skills: [],
  set_enterprise_recording_authorized: null,
  sync_managed_team_skills: null,
  set_activity_sync_enabled: null,
};

const calls: Call[] = [];
const g = globalThis as any;
let hadWindow = false;
let savedInternals: unknown;
let savedFetch: unknown;

beforeAll(() => {
  hadWindow = typeof g.window !== "undefined";
  if (!hadWindow) g.window = g;
  savedInternals = g.window.__TAURI_INTERNALS__;
  g.window.__TAURI_INTERNALS__ = {
    invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
      calls.push({ cmd, args });
      if (!(cmd in RUST_ANSWERS)) throw new Error(`unexpected command ${cmd}`);
      return RUST_ANSWERS[cmd];
    },
  };
  savedFetch = g.fetch;
  g.fetch = () => Promise.reject(new Error("network must not be used"));
});

afterAll(() => {
  g.fetch = savedFetch;
  g.window.__TAURI_INTERNALS__ = savedInternals;
  if (!hadWindow) delete g.window;
});

beforeEach(() => {
  calls.length = 0;
});

describe("history access binding", () => {
  it("isHistoryAccessRestricted resolves false", async () => {
    await expect(commands.isHistoryAccessRestricted()).resolves.toBe(false);
    expect(calls).toEqual([{ cmd: "is_history_access_restricted", args: {} }]);
  });

  it("is generated as a plain boolean promise, not a Result", () => {
    const src = readFileSync(join(__dirname, "..", "..", "utils", "tauri.ts"), "utf8");
    expect(src).toMatch(/async isHistoryAccessRestricted\(\)\s*:\s*Promise<boolean>/);
  });
});

describe("enterprise bindings answer for a personal install", () => {
  it("leaves the UI visible", async () => {
    await expect(commands.applyEnterpriseUiVisibility()).resolves.toBe(false);
    expect(calls).toEqual([{ cmd: "apply_enterprise_ui_visibility", args: {} }]);
  });

  it("reports no host identity", async () => {
    await expect(commands.getEnterpriseHostIdentity()).resolves.toEqual({
      machine_id_hash: null,
      os_user_id_hash: null,
    });
    expect(calls[0].cmd).toBe("get_enterprise_host_identity");
  });

  it("lists no managed team skills", async () => {
    await expect(commands.listManagedTeamSkills()).resolves.toEqual([]);
    expect(calls[0].cmd).toBe("list_managed_team_skills");
  });

  it("accepts a recording verdict with the target's argument names", async () => {
    const res = await commands.setEnterpriseRecordingAuthorized(
      false,
      "license_key",
      "ENT-0000-0000-0000-0000",
    );
    expect(res).toEqual({ status: "ok", data: null });
    expect(calls).toEqual([
      {
        cmd: "set_enterprise_recording_authorized",
        args: {
          authorized: false,
          credentialType: "license_key",
          credential: "ENT-0000-0000-0000-0000",
        },
      },
    ]);
  });

  it("accepts a managed skill sync and changes nothing", async () => {
    const res = await commands.syncManagedTeamSkills([{ name: "org-skill" }], true);
    expect(res).toEqual({ status: "ok", data: null });
    expect(calls[0]).toEqual({
      cmd: "sync_managed_team_skills",
      args: { skills: [{ name: "org-skill" }], pruneUnlisted: true },
    });
    await expect(commands.listManagedTeamSkills()).resolves.toEqual([]);
  });

  it("accepts the activity sync switch", async () => {
    const res = await commands.setActivitySyncEnabled(true);
    expect(res).toEqual({ status: "ok", data: null });
    expect(calls).toEqual([{ cmd: "set_activity_sync_enabled", args: { enabled: true } }]);
  });
});
