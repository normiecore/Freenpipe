// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

"use client";

import { useEffect, useState } from "react";
import { commands } from "@/lib/utils/tauri";
import {
  buildMcpLaunch,
  MCP_NOT_BUNDLED_MESSAGE,
  PLACEHOLDER_MCP_LAUNCH,
  shouldEmbedApiKey,
  withoutEnv,
  type McpApiKeyMode,
  type McpLaunch,
  type McpRuntimePaths,
} from "@/lib/mcp-config";

/**
 * Launch for the MCP server bundled with this install, or `null` when the
 * build has none. A non-default local API port is always passed. `apiKey`
 * (default `"none"`) decides whether the local API key goes into the launch
 * env, see {@link McpApiKeyMode}; a key belongs only in configs written to
 * the user's own files, never in text shown on screen.
 */
export async function resolveMcpLaunch(
  opts: { apiKey?: McpApiKeyMode } = {},
): Promise<McpLaunch | null> {
  let runtime: McpRuntimePaths | null = null;
  try {
    const res = await commands.mcpRuntimeCheck();
    if (res.status === "ok" && res.data.available) runtime = res.data;
  } catch {
    /* command unavailable: treat as not bundled */
  }
  if (!runtime) return null;

  let apiKey: string | null = null;
  let port: number | null = null;
  try {
    const cfg = (await commands.getLocalApiConfig()) as {
      key?: string | null;
      port?: number | null;
    } | null;
    if (shouldEmbedApiKey(opts.apiKey ?? "none", runtime.cliPath)) apiKey = cfg?.key ?? null;
    port = typeof cfg?.port === "number" ? cfg.port : null;
  } catch {
    /* defaults: no key in the config, default port */
  }

  return buildMcpLaunch(
    { bunPath: runtime.bunPath, entryPath: runtime.entryPath },
    { apiKey, port },
  );
}

/** Like {@link resolveMcpLaunch} but throws a user-readable error when absent. */
export async function requireMcpLaunch(
  opts: { apiKey?: McpApiKeyMode } = {},
): Promise<McpLaunch> {
  const launch = await resolveMcpLaunch(opts);
  if (!launch) throw new Error(MCP_NOT_BUNDLED_MESSAGE);
  return launch;
}

/**
 * Launch to show in copy-paste instructions: the bundled server without
 * secrets, or a placeholder (with `bundled: false`) when there is none.
 */
export function useMcpDisplayLaunch(): { launch: McpLaunch; bundled: boolean } {
  const [state, setState] = useState<{ launch: McpLaunch; bundled: boolean }>({
    launch: PLACEHOLDER_MCP_LAUNCH,
    bundled: false,
  });
  useEffect(() => {
    let cancelled = false;
    resolveMcpLaunch().then((launch) => {
      if (!cancelled && launch) setState({ launch: withoutEnv(launch), bundled: true });
    });
    return () => {
      cancelled = true;
    };
  }, []);
  return state;
}
