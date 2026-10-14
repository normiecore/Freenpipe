// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

/**
 * Guard tests for scripts/dev/app-dev.sh.
 *
 * The app's webview still writes under ~/.screenpipe, so app-dev.sh refuses
 * to run next to an existing one and moves away one the session created.
 * Every case runs the script with a throwaway HOME and FP_DD, and with
 * FP_APP_DEV_CMD in place of `bun run tauri dev`, so the real home directory
 * and the reserved ports 3030/11435 are never touched (they are only probed
 * with a connect).
 *
 * Run from the repository root:  bun test scripts/dev/app-dev.test.ts
 */

import { afterEach, describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { createConnection, createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";

const SCRIPT = join(import.meta.dir, "app-dev.sh");

/** A POSIX bash: Git Bash on Windows (never the WSL launcher in System32). */
function findBash(): string | null {
  const candidates = [
    process.env.FP_BASH,
    Bun.which("bash"),
    process.platform === "win32" ? "C:\\Program Files\\Git\\bin\\bash.exe" : undefined,
  ];
  for (const c of candidates) {
    if (!c || !existsSync(c)) continue;
    if (/[\\/]system32[\\/]/i.test(c)) continue;
    return c;
  }
  return null;
}
const BASH = findBash();

/** Forward slashes, which both Git Bash and Windows accept. */
const slash = (p: string) => p.replace(/\\/g, "/");

async function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const srv = createServer();
    srv.once("error", reject);
    srv.listen(0, "127.0.0.1", () => {
      const addr = srv.address();
      const port = typeof addr === "object" && addr ? addr.port : 0;
      srv.close(() => resolve(port));
    });
  });
}

async function listening(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const sock = createConnection({ host: "127.0.0.1", port });
    const done = (v: boolean) => {
      sock.destroy();
      resolve(v);
    };
    sock.setTimeout(500, () => done(false));
    sock.once("connect", () => done(true));
    sock.once("error", () => done(false));
  });
}

type Sandbox = { root: string; home: string; dd: string; withHomeScreenpipe: boolean };
const sandboxes: string[] = [];

function sandbox(withHomeScreenpipe: boolean): Sandbox {
  const root = mkdtempSync(join(tmpdir(), "fp-app-dev-"));
  sandboxes.push(root);
  const home = join(root, "home");
  const dd = join(root, "dd");
  mkdirSync(home, { recursive: true });
  if (withHomeScreenpipe) {
    mkdirSync(join(home, ".screenpipe", "chats"), { recursive: true });
    writeFileSync(join(home, ".screenpipe", "store.bin"), "{}");
  }
  return { root, home, dd, withHomeScreenpipe };
}

afterEach(() => {
  while (sandboxes.length) rmSync(sandboxes.pop()!, { recursive: true, force: true });
});

async function run(
  sb: Sandbox,
  args: string[],
  extra: Record<string, string> = {},
): Promise<{ code: number; out: string; err: string }> {
  const env: Record<string, string> = {};
  for (const [k, v] of Object.entries(process.env)) {
    if (v === undefined) continue;
    // Drop anything a dev shell may have exported, so the script derives
    // every value from the sandbox.
    if (/^(FP_|SCREENPIPE_|WEBVIEW2_USER_DATA_FOLDER$)/.test(k)) continue;
    env[k] = v;
  }
  Object.assign(env, {
    HOME: slash(sb.home),
    FP_DD: slash(sb.dd),
    FP_PORT: String(await freePort()),
    SCREENPIPE_FOCUS_PORT: String(await freePort()),
    // A real screenpipe may be running on this machine; that guard is not
    // what these cases test.
    FP_ALLOW_SCREENPIPE_RUNNING: "1",
    ...extra,
  });
  // Git Bash on some Windows machines intermittently fails to fork
  // ("cygheap read copy failed", "fork: retry"). dev-env.sh then fails
  // closed with exit 2; rerun those attempts instead of reporting them.
  let last = { code: -1, out: "", err: "" };
  for (let attempt = 0; attempt < 4; attempt++) {
    const proc = Bun.spawn([BASH!, SCRIPT, ...args], { env, stdout: "pipe", stderr: "pipe" });
    const [out, err, code] = await Promise.all([
      new Response(proc.stdout).text(),
      new Response(proc.stderr).text(),
      proc.exited,
    ]);
    last = { code, out, err };
    if (!/cygheap|fork: |dofork|Resource temporarily unavailable/.test(err)) break;
    rmSync(sb.dd, { recursive: true, force: true });
    if (!sb.withHomeScreenpipe) rmSync(join(sb.home, ".screenpipe"), { recursive: true, force: true });
  }
  return last;
}

describe.skipIf(!BASH)("app-dev.sh guards", () => {
  test("refuses while ~/.screenpipe exists", async () => {
    const sb = sandbox(true);
    const r = await run(sb, ["--check"]);
    expect(r.code).toBe(1);
    expect(r.err).toContain("refusing");
    expect(r.err).toContain(".screenpipe exists");
    expect(r.out).not.toContain("checks passed");
  }, 30_000);

  test("FP_ALLOW_HOME_SCREENPIPE=1 lets the checks pass with a warning", async () => {
    const sb = sandbox(true);
    const r = await run(sb, ["--check"], { FP_ALLOW_HOME_SCREENPIPE: "1" });
    expect(r.code).toBe(0);
    expect(r.out).toContain("checks passed");
    expect(r.err).toContain("WARNING");
  }, 30_000);

  test("moves a ~/.screenpipe that the session created into FP_DD", async () => {
    // With a screenpipe listening on 3030/11435 the script leaves the
    // directory alone on purpose; that path is covered by the warning text.
    if ((await listening(3030)) || (await listening(11435))) return;
    const sb = sandbox(false);
    const r = await run(sb, [], {
      FP_APP_DEV_CMD: 'mkdir -p "$HOME/.screenpipe/chats" && echo "{}" > "$HOME/.screenpipe/chats/a.json"',
    });
    expect(r.code).toBe(0);
    expect(existsSync(join(sb.home, ".screenpipe"))).toBe(false);
    const moved = readdirSync(sb.dd).filter((n) => n.startsWith("home-dot-screenpipe."));
    expect(moved.length).toBe(1);
    expect(existsSync(join(sb.dd, moved[0], "chats", "a.json"))).toBe(true);
    expect(r.err).toContain("moved it to");
  }, 30_000);

  test("with the override, leaves an existing ~/.screenpipe in place and lists changes", async () => {
    const sb = sandbox(true);
    const r = await run(sb, [], {
      FP_ALLOW_HOME_SCREENPIPE: "1",
      FP_APP_DEV_CMD: 'sleep 1; echo "{}" > "$HOME/.screenpipe/chats/dev.json"',
    });
    expect(r.code).toBe(0);
    expect(existsSync(join(sb.home, ".screenpipe", "chats", "dev.json"))).toBe(true);
    expect(r.err).toContain("changed entries");
    expect(r.err).toContain("dev.json");
  }, 30_000);

  test("isolates the WebView2 profile and passes the session's exit status through", async () => {
    const sb = sandbox(false);
    const r = await run(sb, [], {
      FP_APP_DEV_CMD: 'echo "WV2=$WEBVIEW2_USER_DATA_FOLDER"; exit 7',
    });
    expect(r.code).toBe(7);
    const line = r.out.split(/\r?\n/).find((l) => l.startsWith("WV2=")) ?? "";
    expect(slash(line).toLowerCase()).toContain(slash(join(sb.dd, "webview2")).toLowerCase());
  }, 30_000);

  test("refuses when a dev port is already taken", async () => {
    const sb = sandbox(false);
    const srv = createServer();
    const port = await new Promise<number>((resolve) => {
      srv.listen(0, "127.0.0.1", () => {
        const addr = srv.address();
        resolve(typeof addr === "object" && addr ? addr.port : 0);
      });
    });
    try {
      const r = await run(sb, ["--check"], { FP_PORT: String(port) });
      expect(r.code).toBe(1);
      expect(r.err).toContain(`port ${port} is already in use`);
    } finally {
      srv.close();
    }
  }, 30_000);
});
