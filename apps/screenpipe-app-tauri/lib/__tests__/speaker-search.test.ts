// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

import { describe, expect, test } from "bun:test";
import {
  SPEAKER_SEARCH_PAGE_SIZE,
  fetchAllSpeakers,
  speakerSearchPath,
  type SpeakerSearchFetch,
} from "../speaker-search";

interface Row {
  id: number;
  name: string;
}

const people = (count: number): Row[] =>
  Array.from({ length: count }, (_, i) => ({ id: i + 1, name: `person ${i + 1}` }));

/** A fake engine that pages like `/speakers/search` (default limit 100). */
function pagingServer(rows: Row[]) {
  const calls: string[] = [];
  const fetchPath: SpeakerSearchFetch = async (path) => {
    calls.push(path);
    const query = new URL(path, "http://localhost:3035").searchParams;
    const limit = Number(query.get("limit") ?? 100);
    const offset = Number(query.get("offset") ?? 0);
    const page = rows.slice(offset, offset + limit);
    return { ok: true, status: 200, json: async () => page };
  };
  return { calls, fetchPath };
}

describe("speakerSearchPath", () => {
  test("always sends a name and only the parameters given", () => {
    expect(speakerSearchPath()).toBe("/speakers/search?name=");
    expect(speakerSearchPath({ name: "Al Bo", limit: 50, includeSamples: false })).toBe(
      "/speakers/search?name=Al+Bo&limit=50&include_samples=false"
    );
    expect(speakerSearchPath({ name: "a&b", limit: 100, offset: 200, includeSamples: true })).toBe(
      "/speakers/search?name=a%26b&limit=100&offset=200&include_samples=true"
    );
  });
});

describe("fetchAllSpeakers", () => {
  test("pages past the 100-name default until a short page", async () => {
    const server = pagingServer(people(250));
    const all = await fetchAllSpeakers<Row>(server.fetchPath);
    expect(all).toHaveLength(250);
    expect(all[249]).toEqual({ id: 250, name: "person 250" });
    expect(server.calls).toEqual([
      "/speakers/search?name=&limit=100&offset=0",
      "/speakers/search?name=&limit=100&offset=100",
      "/speakers/search?name=&limit=100&offset=200",
    ]);
  });

  test("an exact multiple of the page size ends on an empty page", async () => {
    const server = pagingServer(people(2 * SPEAKER_SEARCH_PAGE_SIZE));
    const all = await fetchAllSpeakers<Row>(server.fetchPath);
    expect(all).toHaveLength(200);
    expect(server.calls).toHaveLength(3);
  });

  test("passes the name and sample flag on every page", async () => {
    const server = pagingServer(people(3));
    await fetchAllSpeakers<Row>(server.fetchPath, {
      name: "jo",
      includeSamples: false,
      pageSize: 2,
    });
    expect(server.calls).toEqual([
      "/speakers/search?name=jo&limit=2&offset=0&include_samples=false",
      "/speakers/search?name=jo&limit=2&offset=2&include_samples=false",
    ]);
  });

  test("stops when an engine ignores offset and repeats the list", async () => {
    const everyone = people(150);
    const calls: string[] = [];
    const fetchPath: SpeakerSearchFetch = async (path) => {
      calls.push(path);
      return { ok: true, status: 200, json: async () => everyone };
    };
    const all = await fetchAllSpeakers<Row>(fetchPath);
    expect(all).toHaveLength(150);
    expect(calls).toHaveLength(2);
  });

  test("drops a speaker repeated across pages", async () => {
    const pages: Row[][] = [
      [
        { id: 1, name: "a" },
        { id: 2, name: "b" },
      ],
      [
        { id: 2, name: "b" },
        { id: 3, name: "c" },
      ],
      [],
    ];
    let call = 0;
    const fetchPath: SpeakerSearchFetch = async () => {
      const page = pages[call++] ?? [];
      return { ok: true, status: 200, json: async () => page };
    };
    const all = await fetchAllSpeakers<Row>(fetchPath, { pageSize: 2 });
    expect(all.map((row) => row.id)).toEqual([1, 2, 3]);
  });

  test("never requests more than maxPages pages", async () => {
    const server = pagingServer(people(1000));
    const all = await fetchAllSpeakers<Row>(server.fetchPath, { pageSize: 10, maxPages: 3 });
    expect(all).toHaveLength(30);
    expect(server.calls).toHaveLength(3);
  });

  test("a failed page throws instead of returning a partial list", async () => {
    let call = 0;
    const fetchPath: SpeakerSearchFetch = async () => {
      call++;
      if (call === 2) return { ok: false, status: 500, json: async () => ({ error: "db" }) };
      return { ok: true, status: 200, json: async () => people(2) };
    };
    await expect(fetchAllSpeakers<Row>(fetchPath, { pageSize: 2 })).rejects.toThrow("500");
  });

  test("a non-array body counts as an empty page", async () => {
    const fetchPath: SpeakerSearchFetch = async () => ({
      ok: true,
      status: 200,
      json: async () => ({ unexpected: true }),
    });
    expect(await fetchAllSpeakers<Row>(fetchPath)).toEqual([]);
  });
});
