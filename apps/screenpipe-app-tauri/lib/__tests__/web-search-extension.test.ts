// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

// The `sp_web_search` Pi extension (src-tauri/assets/extensions/web-search.ts,
// identical to the engine's copy) against a mock of the engine's
// `/v1/web-search` route.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import registerWebSearch from "../../src-tauri/assets/extensions/web-search";

type Tool = {
  name: string;
  execute: (
    id: string,
    args: { query: string },
    signal: AbortSignal,
    onUpdate?: unknown
  ) => Promise<{
    content: Array<{ type: string; text: string }>;
    details: { text: string; sources: Array<{ title?: string; url?: string }>; query: string };
  }>;
};

function loadTool(): Tool {
  let tool: Tool | undefined;
  registerWebSearch({
    registerTool: (t: Tool) => {
      tool = t;
    },
  } as any);
  if (!tool) throw new Error("the extension registered no tool");
  return tool;
}

const LOCAL_KEY = "local-test-key";
const requests: Array<{ path: string; auth: string | null; body: any }> = [];
let server: ReturnType<typeof Bun.serve>;
const saved: Record<string, string | undefined> = {};

beforeAll(() => {
  server = Bun.serve({
    port: 0,
    async fetch(req) {
      const url = new URL(req.url);
      const body = await req.json().catch(() => null);
      requests.push({ path: url.pathname, auth: req.headers.get("authorization"), body });
      if (url.pathname !== "/v1/web-search") return new Response("not found", { status: 404 });
      if (body?.query === "unconfigured") {
        return Response.json(
          { error: "web_search_not_configured", message: "web search is not configured." },
          { status: 501 }
        );
      }
      if (body?.query === "provider down") {
        return Response.json(
          { error: "upstream_unreachable", message: "could not reach the search provider" },
          { status: 502 }
        );
      }
      return Response.json({
        content: "1. Result one\n   https://one.example.test/",
        sources: [
          { title: "Result one", url: "https://one.example.test/" },
          { title: "Script", url: "javascript:alert(1)" },
          ...Array.from({ length: 20 }, (_, i) => ({ url: `https://many.example.test/${i}` })),
        ],
      });
    },
  });
  for (const name of [
    "SCREENPIPE_LOCAL_API_URL",
    "SCREENPIPE_LOCAL_API_KEY",
    "SCREENPIPE_API_AUTH_KEY",
  ]) {
    saved[name] = process.env[name];
  }
  process.env.SCREENPIPE_LOCAL_API_URL = `http://127.0.0.1:${server.port}/`;
  process.env.SCREENPIPE_LOCAL_API_KEY = LOCAL_KEY;
  delete process.env.SCREENPIPE_API_AUTH_KEY;
});

afterAll(() => {
  server?.stop(true);
  for (const [name, value] of Object.entries(saved)) {
    if (value === undefined) delete process.env[name];
    else process.env[name] = value;
  }
});

describe("sp_web_search extension", () => {
  test("registers the sp_web_search tool", () => {
    expect(loadTool().name).toBe("sp_web_search");
  });

  test("calls the local route with the local key and returns text, sources and query", async () => {
    requests.length = 0;
    const result = await loadTool().execute(
      "t1",
      { query: "  rust   release notes " },
      new AbortController().signal
    );
    expect(requests).toHaveLength(1);
    expect(requests[0].path).toBe("/v1/web-search");
    expect(requests[0].auth).toBe(`Bearer ${LOCAL_KEY}`);
    expect(requests[0].body).toEqual({ query: "rust release notes" });

    expect(result.details.query).toBe("rust release notes");
    expect(result.details.text).toBe(result.content[0].text);
    expect(result.details.text).toContain("untrusted third-party text");
    expect(result.details.text).toContain("1. Result one");
    // Non-web links are dropped and the list is capped.
    expect(result.details.sources[0]).toEqual({
      title: "Result one",
      url: "https://one.example.test/",
    });
    expect(result.details.sources.some((s) => s.url?.startsWith("javascript:"))).toBe(false);
    expect(result.details.sources.length).toBe(10);
  });

  test("reports a disabled search without failing the run", async () => {
    const result = await loadTool().execute(
      "t2",
      { query: "unconfigured" },
      new AbortController().signal
    );
    expect(result.content[0].text).toBe(
      "Web search is not available: web search is not configured."
    );
    expect(result.details.sources).toEqual([]);
  });

  test("passes provider errors on", async () => {
    const result = await loadTool().execute(
      "t3",
      { query: "provider down" },
      new AbortController().signal
    );
    expect(result.content[0].text).toContain("HTTP 502");
    expect(result.content[0].text).toContain("could not reach the search provider");
  });

  test("sends nothing for an empty query or a cancelled run", async () => {
    requests.length = 0;
    const empty = await loadTool().execute("t4", { query: "   " }, new AbortController().signal);
    expect(empty.content[0].text).toContain("non-empty query");
    const controller = new AbortController();
    controller.abort();
    const cancelled = await loadTool().execute("t5", { query: "x" }, controller.signal);
    expect(cancelled.content[0].text).toBe("Cancelled");
    expect(requests).toHaveLength(0);
  });
});
