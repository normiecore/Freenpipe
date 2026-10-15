// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

/**
 * MCP launch configs for external AI tools (Claude Desktop, Cursor, Codex,
 * Claude Code, Hermes, OpenClaw, Warp, Msty, AnythingLLM).
 *
 * The MCP server ships inside the app as a single JavaScript file
 * (`mcp/screenpipe-mcp.js`) started with the app's own bun binary, so every
 * config here launches it by absolute path. Nothing is resolved from a
 * package registry, which also means the server always matches the
 * installed app version.
 *
 * Pure functions only; `lib/mcp-runtime.ts` supplies the real paths.
 */

/** A process launch as MCP clients describe it. */
export type McpLaunch = {
  command: string;
  args: string[];
  env?: Record<string, string>;
};

/** Paths reported by the `mcp_runtime_check` command. */
export type McpRuntimePaths = {
  bunPath: string | null;
  entryPath: string | null;
};

/** Key under which tools list the server (kept for existing configs). */
export const MCP_SERVER_NAME = "screenpipe";

/** Port the MCP server assumes when `--port` is not given. */
export const DEFAULT_LOCAL_API_PORT = 3030;

/**
 * Stand-in shown in copy-paste instructions when this build carries no MCP
 * server (development builds that skipped staging, or platforms whose
 * installer does not bundle it yet).
 */
export const PLACEHOLDER_MCP_LAUNCH: McpLaunch = {
  command: "<Freenpipe install folder>/bun",
  args: ["<Freenpipe install folder>/mcp/screenpipe-mcp.js"],
};

export const MCP_NOT_BUNDLED_MESSAGE =
  "This build of Freenpipe does not include the MCP server, so it cannot be registered automatically. " +
  "Build it from the repository (cd packages/screenpipe-mcp && bun install && bun run build) " +
  "and point your tool at dist/cli.js with bun or node.";

/** Absolute on POSIX (`/x`), Windows drive (`C:\x`, `C:/x`) or UNC (`\\host\x`). */
export function isAbsolutePath(p: string | null | undefined): p is string {
  if (!p) return false;
  return p.startsWith("/") || /^[A-Za-z]:[\\/]/.test(p) || p.startsWith("\\\\");
}

/**
 * Launch for the bundled server, or `null` when either path is missing or
 * relative (a relative path would depend on the tool's working directory).
 */
export function buildMcpLaunch(
  runtime: McpRuntimePaths,
  opts: { apiKey?: string | null; port?: number | null } = {},
): McpLaunch | null {
  const { bunPath, entryPath } = runtime;
  if (!isAbsolutePath(bunPath) || !isAbsolutePath(entryPath)) return null;
  const args = [entryPath];
  if (opts.port && opts.port !== DEFAULT_LOCAL_API_PORT) {
    args.push("--port", String(opts.port));
  }
  const launch: McpLaunch = { command: bunPath, args };
  if (opts.apiKey) launch.env = { SCREENPIPE_LOCAL_API_KEY: opts.apiKey };
  return launch;
}

/** Copy of a launch without its env block (for text shown on screen). */
export function withoutEnv(launch: McpLaunch): McpLaunch {
  return { command: launch.command, args: [...launch.args] };
}

function serverEntry(launch: McpLaunch): Record<string, unknown> {
  const entry: Record<string, unknown> = {
    command: launch.command,
    args: [...launch.args],
  };
  if (launch.env && Object.keys(launch.env).length > 0) entry.env = { ...launch.env };
  return entry;
}

/**
 * Add (or replace) the server in a Claude Desktop / Cursor style config,
 * keeping every other server and top-level key.
 */
export function mergeMcpServer(
  config: Record<string, unknown>,
  launch: McpLaunch,
): Record<string, unknown> {
  const servers =
    config.mcpServers && typeof config.mcpServers === "object" && !Array.isArray(config.mcpServers)
      ? { ...(config.mcpServers as Record<string, unknown>) }
      : {};
  servers[MCP_SERVER_NAME] = serverEntry(launch);
  return { ...config, mcpServers: servers };
}

/** `{ "mcpServers": { "screenpipe": … } }` as pretty JSON. */
export function mcpServersJson(launch: McpLaunch): string {
  return JSON.stringify(mergeMcpServer({}, launch), null, 2);
}

/** Bare `{ command, args }` block (Msty's STDIO tool form). */
export function launchJson(launch: McpLaunch): string {
  return JSON.stringify(serverEntry(launch), null, 2);
}

/** Warp's per-server block. */
export function warpServerJson(launch: McpLaunch): string {
  return JSON.stringify(
    { [MCP_SERVER_NAME]: { ...serverEntry(launch), start_on_launch: true } },
    null,
    2,
  );
}

function tomlString(value: string): string {
  // JSON escapes are a subset of TOML basic-string escapes.
  return JSON.stringify(value);
}

function tomlKey(value: string): string {
  return /^[A-Za-z0-9_-]+$/.test(value) ? value : tomlString(value);
}

/** `[mcp_servers.screenpipe]` table for Codex's `config.toml`. */
export function codexMcpToml(launch: McpLaunch): string {
  const lines = [
    `[mcp_servers.${MCP_SERVER_NAME}]`,
    `command = ${tomlString(launch.command)}`,
    `args = [${launch.args.map(tomlString).join(", ")}]`,
    "enabled = true",
  ];
  const env = Object.entries(launch.env ?? {});
  if (env.length > 0) {
    lines.push("", `[mcp_servers.${MCP_SERVER_NAME}.env]`);
    for (const [k, v] of env) lines.push(`${tomlKey(k)} = ${tomlString(v)}`);
  }
  return lines.join("\n");
}

function yamlString(value: string): string {
  return JSON.stringify(value);
}

/** Hermes `config.yaml` fragment. */
export function hermesYaml(launch: McpLaunch): string {
  const lines = [
    "mcp_servers:",
    `  ${MCP_SERVER_NAME}:`,
    `    command: ${yamlString(launch.command)}`,
    "    args:",
    ...launch.args.map((a) => `      - ${yamlString(a)}`),
  ];
  if (launch.env && Object.keys(launch.env).length > 0) {
    lines.push("    env:");
    for (const [k, v] of Object.entries(launch.env)) lines.push(`      ${k}: ${yamlString(v)}`);
  }
  return `${lines.join("\n")}\n`;
}

/**
 * Quote for a terminal command line. Backslashes are left alone so Windows
 * paths stay readable (they are literal inside double quotes in bash, cmd
 * and PowerShell alike).
 */
function shellQuote(value: string): string {
  return /^[A-Za-z0-9_\-./:\\=]+$/.test(value) ? value : `"${value.replace(/(["$`])/g, "\\$1")}"`;
}

/** One-line `claude mcp add` command for Claude Code. */
export function claudeCodeAddCommand(launch: McpLaunch): string {
  const env = Object.entries(launch.env ?? {}).flatMap(([k, v]) => ["-e", shellQuote(`${k}=${v}`)]);
  return ["claude", "mcp", "add", MCP_SERVER_NAME, ...env, "--", shellQuote(launch.command), ...launch.args.map(shellQuote)].join(" ");
}

/** True when a launch was produced by these helpers for the bundled server. */
export function isBundledLaunch(launch: McpLaunch): boolean {
  return (
    isAbsolutePath(launch.command) &&
    launch.args.length > 0 &&
    isAbsolutePath(launch.args[0]) &&
    /[\\/]mcp[\\/]screenpipe-mcp\.js$/.test(launch.args[0])
  );
}
