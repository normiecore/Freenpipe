// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

/**
 * Where the MCP server looks for the `screenpipe` CLI it uses to read the
 * local API key. Only absolute paths of installed binaries: the server never
 * resolves the CLI from a package registry.
 */

import * as path from "path";

/** Absolute locations where a bundled `screenpipe` CLI may live. */
export function cliCandidates(opts: {
  platform: NodeJS.Platform;
  home: string;
  env: Record<string, string | undefined>;
  scriptPath?: string;
}): string[] {
  const { platform, home, env, scriptPath } = opts;
  const win = platform === "win32";
  const p = win ? path.win32 : path.posix;
  const exe = win ? "screenpipe.exe" : "screenpipe";
  const out: string[] = [];
  const cliEnv = env.SCREENPIPE_CLI?.trim();
  if (cliEnv && p.isAbsolute(cliEnv)) out.push(cliEnv);
  if (scriptPath && p.isAbsolute(scriptPath)) {
    // <install>/mcp/screenpipe-mcp.js -> <install>/screenpipe(.exe)
    const scriptDir = p.dirname(scriptPath);
    out.push(p.join(scriptDir, "..", exe), p.join(scriptDir, exe));
  }
  if (platform === "darwin") {
    out.push(
      "/Applications/Handvel.app/Contents/MacOS/screenpipe",
      p.join(home, "Applications", "Handvel.app", "Contents", "MacOS", "screenpipe"),
    );
  } else if (win) {
    const localAppData = env.LOCALAPPDATA || p.join(home, "AppData", "Local");
    const programFiles = env.ProgramFiles || "C:\\Program Files";
    out.push(
      p.join(localAppData, "Handvel", exe),
      p.join(programFiles, "Handvel", exe),
    );
  }
  return [...new Set(out.map((c) => p.normalize(c)))];
}
