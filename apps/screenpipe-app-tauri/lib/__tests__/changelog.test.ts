// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

import { describe, expect, it, vi } from "vitest";
import {
  CHANGELOG_CACHE_KEY,
  CHANGELOG_CACHE_TTL_MS,
  RELEASES_API_URL,
  loadChangelog,
  parseGithubReleases,
  type ChangelogStorage,
} from "../changelog";

const RELEASES = [
  {
    tag_name: "v0.2.0",
    name: "Handvel 0.2.0",
    body: "## Fixes\r\n- one\r\n",
    html_url: "https://github.com/normiecore/Handvel/releases/tag/v0.2.0",
    published_at: "2026-10-01T12:00:00Z",
    draft: false,
    prerelease: false,
  },
  { tag_name: "v0.3.0-rc1", name: "", body: null, published_at: null, draft: false, prerelease: true },
  { tag_name: "v9.9.9", name: "unpublished", draft: true },
  { name: "no tag" },
  "garbage",
];

function memoryStorage(initial: Record<string, string> = {}): ChangelogStorage & { data: Record<string, string> } {
  const data = { ...initial };
  return {
    data,
    getItem: (k: string) => (k in data ? data[k] : null),
    setItem: (k: string, v: string) => {
      data[k] = v;
    },
  };
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
}

describe("parseGithubReleases", () => {
  it("maps published releases and skips drafts and junk", () => {
    const entries = parseGithubReleases(RELEASES);
    expect(entries.map((e) => e.version)).toEqual(["0.2.0", "0.3.0-rc1"]);
    expect(entries[0]).toMatchObject({
      title: "Handvel 0.2.0",
      date: "2026-10-01",
      body: "## Fixes\n- one",
      prerelease: false,
    });
    expect(entries[1].title).toBe("v0.3.0-rc1");
    expect(entries[1].date).toBe("");
    expect(entries[1].url).toBe("https://github.com/normiecore/Handvel/releases/tag/v0.3.0-rc1");
    expect(entries[1].prerelease).toBe(true);
  });

  it("returns nothing for a non-array payload", () => {
    expect(parseGithubReleases({ message: "rate limited" })).toEqual([]);
    expect(parseGithubReleases(null)).toEqual([]);
  });
});

describe("loadChangelog", () => {
  it("reads the Handvel GitHub releases and caches them", async () => {
    expect(RELEASES_API_URL).toBe("https://api.github.com/repos/normiecore/Handvel/releases?per_page=50");
    const fetchImpl = vi.fn(async () => jsonResponse(RELEASES));
    const storage = memoryStorage();

    const first = await loadChangelog({ fetchImpl: fetchImpl as unknown as typeof fetch, storage, now: 1_000 });
    expect(first?.map((e) => e.version)).toEqual(["0.2.0", "0.3.0-rc1"]);
    expect(fetchImpl).toHaveBeenCalledTimes(1);
    expect((fetchImpl.mock.calls[0] as unknown[])[0]).toBe(RELEASES_API_URL);
    expect(storage.data[CHANGELOG_CACHE_KEY]).toBeTruthy();

    // Within the hour: served from the cache.
    const second = await loadChangelog({
      fetchImpl: fetchImpl as unknown as typeof fetch,
      storage,
      now: 1_000 + CHANGELOG_CACHE_TTL_MS - 1,
    });
    expect(second).toEqual(first);
    expect(fetchImpl).toHaveBeenCalledTimes(1);
  });

  it("refreshes after an hour and keeps the stale copy when GitHub fails", async () => {
    const storage = memoryStorage({
      [CHANGELOG_CACHE_KEY]: JSON.stringify({ fetchedAt: 0, entries: parseGithubReleases(RELEASES) }),
    });
    const failing = vi.fn(async () => {
      throw new Error("offline");
    });
    const out = await loadChangelog({
      fetchImpl: failing as unknown as typeof fetch,
      storage,
      now: CHANGELOG_CACHE_TTL_MS + 1,
    });
    expect(failing).toHaveBeenCalledTimes(1);
    expect(out?.[0].version).toBe("0.2.0");
  });

  it("returns null with no cache and no reachable GitHub, so the bundled file is used", async () => {
    const notFound = vi.fn(async () => jsonResponse({ message: "Not Found" }, 404));
    const out = await loadChangelog({ fetchImpl: notFound as unknown as typeof fetch, storage: memoryStorage(), now: 5 });
    expect(out).toBeNull();
  });

  it("works without storage", async () => {
    const fetchImpl = vi.fn(async () => jsonResponse(RELEASES));
    const out = await loadChangelog({ fetchImpl: fetchImpl as unknown as typeof fetch, storage: null, now: 5 });
    expect(out).toHaveLength(2);
  });
});
