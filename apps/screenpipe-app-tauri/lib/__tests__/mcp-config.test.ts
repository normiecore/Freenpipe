// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

import { describe, it, expect } from "bun:test";
import {
  buildMcpLaunch,
  claudeCodeAddCommand,
  codexMcpToml,
  hermesYaml,
  isAbsolutePath,
  isBundledLaunch,
  launchJson,
  mcpServersJson,
  mergeMcpServer,
  warpServerJson,
  withoutEnv,
  type McpLaunch,
} from "../mcp-config";

const WIN_RUNTIME = {
  bunPath: "C:\\Program Files\\Freenpipe\\bun.exe",
  entryPath: "C:\\Program Files\\Freenpipe\\mcp\\screenpipe-mcp.js",
};
const MAC_RUNTIME = {
  bunPath: "/Applications/Freenpipe.app/Contents/MacOS/bun",
  entryPath: "/Applications/Freenpipe.app/Contents/Resources/mcp/screenpipe-mcp.js",
};

const REGISTRY = /@latest|\bnpx\b|\bbunx\b|"x"/;

function launch(): McpLaunch {
  const l = buildMcpLaunch(WIN_RUNTIME, { apiKey: "sp-abc12345" });
  if (!l) throw new Error("expected a launch");
  return l;
}

describe("Claude Desktop config", () => {
  it("points at the bundled MCP entry by absolute path", () => {
    const existing = {
      globalShortcut: "Ctrl+Space",
      mcpServers: { other: { command: "/usr/bin/other", args: [] } },
    };
    const config = mergeMcpServer(existing, launch()) as {
      globalShortcut: string;
      mcpServers: Record<string, { command: string; args: string[]; env?: Record<string, string> }>;
    };
    const server = config.mcpServers.screenpipe;
    expect(server.command).toBe(WIN_RUNTIME.bunPath);
    expect(server.args[0]).toBe(WIN_RUNTIME.entryPath);
    expect(isAbsolutePath(server.command)).toBe(true);
    expect(isAbsolutePath(server.args[0])).toBe(true);
    expect(server.env).toEqual({ SCREENPIPE_LOCAL_API_KEY: "sp-abc12345" });
    // other servers and keys survive
    expect(config.mcpServers.other.command).toBe("/usr/bin/other");
    expect(config.globalShortcut).toBe("Ctrl+Space");
    expect(JSON.stringify(config)).not.toMatch(REGISTRY);
    // the input object is not mutated
    expect(Object.keys(existing.mcpServers)).toEqual(["other"]);
  });

  it("replaces an old registry-based entry", () => {
    const stale = { mcpServers: { screenpipe: { command: "npx", args: ["-y", "screenpipe-mcp"] } } };
    const config = mergeMcpServer(stale, launch());
    expect(JSON.stringify(config)).not.toMatch(REGISTRY);
    expect(isBundledLaunch((config.mcpServers as Record<string, McpLaunch>).screenpipe)).toBe(true);
  });
});

describe("buildMcpLaunch", () => {
  it("requires absolute paths for both bun and the entry", () => {
    expect(buildMcpLaunch({ bunPath: null, entryPath: WIN_RUNTIME.entryPath })).toBeNull();
    expect(buildMcpLaunch({ bunPath: "bun", entryPath: WIN_RUNTIME.entryPath })).toBeNull();
    expect(buildMcpLaunch({ bunPath: MAC_RUNTIME.bunPath, entryPath: "mcp/screenpipe-mcp.js" })).toBeNull();
    expect(buildMcpLaunch(MAC_RUNTIME)).toEqual({
      command: MAC_RUNTIME.bunPath,
      args: [MAC_RUNTIME.entryPath],
    });
  });

  it("passes a non-default local API port", () => {
    expect(buildMcpLaunch(MAC_RUNTIME, { port: 3035 })?.args).toEqual([
      MAC_RUNTIME.entryPath,
      "--port",
      "3035",
    ]);
    expect(buildMcpLaunch(MAC_RUNTIME, { port: 3030 })?.args).toEqual([MAC_RUNTIME.entryPath]);
  });

  it("recognises absolute paths on every platform", () => {
    for (const p of ["/opt/x", "C:\\x", "c:/x", "\\\\server\\share\\x"]) expect(isAbsolutePath(p)).toBe(true);
    for (const p of ["x", "./x", "C:x", "", null]) expect(isAbsolutePath(p)).toBe(false);
  });
});

describe("other tool formats", () => {
  const display = withoutEnv(launch());

  it("never contain registry commands and always use the absolute entry", () => {
    for (const text of [
      mcpServersJson(display),
      launchJson(display),
      warpServerJson(display),
      hermesYaml(display),
      codexMcpToml(display),
      claudeCodeAddCommand(display),
    ]) {
      expect(text).not.toMatch(REGISTRY);
      expect(text).toContain("screenpipe-mcp.js");
    }
  });

  it("withoutEnv drops secrets from on-screen snippets", () => {
    expect(mcpServersJson(display)).not.toContain("sp-abc12345");
    expect(claudeCodeAddCommand(display)).not.toContain("SCREENPIPE_LOCAL_API_KEY");
  });

  it("formats Codex TOML with an env table when a key is present", () => {
    const toml = codexMcpToml(launch());
    expect(toml).toContain("[mcp_servers.screenpipe]");
    expect(toml).toContain('command = "C:\\\\Program Files\\\\Freenpipe\\\\bun.exe"');
    expect(toml).toContain("[mcp_servers.screenpipe.env]");
    expect(toml).toContain('SCREENPIPE_LOCAL_API_KEY = "sp-abc12345"');
  });

  it("quotes paths with spaces in the Claude Code command", () => {
    expect(claudeCodeAddCommand(display)).toBe(
      'claude mcp add screenpipe -- "C:\\Program Files\\Freenpipe\\bun.exe" "C:\\Program Files\\Freenpipe\\mcp\\screenpipe-mcp.js"',
    );
  });

  it("parses back as the same launch", () => {
    const parsed = JSON.parse(warpServerJson(display)).screenpipe;
    expect(parsed.command).toBe(display.command);
    expect(parsed.args).toEqual(display.args);
    expect(parsed.start_on_launch).toBe(true);
    expect(hermesYaml(display)).toContain(`command: ${JSON.stringify(display.command)}`);
  });
});
