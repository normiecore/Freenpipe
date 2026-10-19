// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe
"use client";

import { useCallback, useEffect, useState } from "react";
import { commands } from "@/lib/utils/tauri";
import { useSettings } from "@/lib/hooks/use-settings";

/**
 * Base URL of the user's AI gateway as the engine sees it (the `aiGatewayUrl`
 * setting, else `SCREENPIPE_AI_GATEWAY_URL`), or `null` when none is
 * configured. Asking also makes the engine pick up a just-saved setting.
 */
export async function fetchAiGatewayUrl(): Promise<string | null> {
  try {
    const res = await commands.getScreenpipeAiGatewayUrl();
    return res.status === "ok" ? res.data : null;
  } catch {
    return null;
  }
}

/**
 * Whether the engine has a gateway key (the saved `aiGatewayApiKey`, or
 * `SCREENPIPE_AI_GATEWAY_KEY` in its environment). The key is never returned.
 */
export async function fetchAiGatewayKeyConfigured(): Promise<boolean> {
  try {
    return await commands.getAiGatewayKeyConfigured();
  } catch {
    return false;
  }
}

export interface AiGatewayState {
  /** Configured gateway base, or `null`. */
  url: string | null;
  /** Shorthand for `url !== null`. Gateway-only features hide when false. */
  configured: boolean;
  /** A gateway key is available to the engine (setting or environment). */
  keyConfigured: boolean;
  /** True until the first answer arrives. */
  loading: boolean;
  /** Ask again, e.g. right after saving the gateway settings. */
  refresh: () => Promise<void>;
}

/** Track whether an AI gateway is configured. Re-checks when the setting changes. */
export function useAiGateway(): AiGatewayState {
  const { settings } = useSettings();
  const [url, setUrl] = useState<string | null>(null);
  const [keyConfigured, setKeyConfigured] = useState(false);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    const [nextUrl, nextKey] = await Promise.all([
      fetchAiGatewayUrl(),
      fetchAiGatewayKeyConfigured(),
    ]);
    setUrl(nextUrl);
    setKeyConfigured(nextKey);
    setLoading(false);
  }, []);

  const savedUrl = (settings as { aiGatewayUrl?: string }).aiGatewayUrl ?? "";
  const hasSavedKey = Boolean(
    (settings as { aiGatewayApiKey?: string }).aiGatewayApiKey?.trim(),
  );
  useEffect(() => {
    refresh();
  }, [refresh, savedUrl, hasSavedKey]);

  return { url, configured: url !== null, keyConfigured, loading, refresh };
}
