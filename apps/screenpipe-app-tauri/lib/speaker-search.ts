// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

/**
 * Helpers for `GET /speakers/search`.
 *
 * The engine answers one page per call: at most `limit` names (100 when the
 * parameter is missing), ordered case-insensitively and skipped by `offset`.
 * `include_samples=false` asks for the light form (`{id, name, metadata:"{}"}`),
 * which autocomplete boxes use. A caller that needs every named speaker, such
 * as the speakers settings page, pages with {@link fetchAllSpeakers}.
 */

/** Names requested per page when listing every speaker. */
export const SPEAKER_SEARCH_PAGE_SIZE = 100;

/**
 * Most pages {@link fetchAllSpeakers} requests before it stops (10 000 names
 * at the default page size), so a misbehaving server cannot keep a page
 * looping.
 */
export const SPEAKER_SEARCH_MAX_PAGES = 100;

export interface SpeakerSearchParams {
  /** Name prefix; empty or missing matches every named speaker. */
  name?: string;
  limit?: number;
  offset?: number;
  /** `false` selects the light result without voice samples. */
  includeSamples?: boolean;
}

/** Path and query string for one `/speakers/search` call. */
export function speakerSearchPath(params: SpeakerSearchParams = {}): string {
  const query = new URLSearchParams();
  query.set("name", params.name ?? "");
  if (params.limit !== undefined) query.set("limit", String(params.limit));
  if (params.offset !== undefined) query.set("offset", String(params.offset));
  if (params.includeSamples !== undefined) {
    query.set("include_samples", String(params.includeSamples));
  }
  return `/speakers/search?${query.toString()}`;
}

/** The part of `fetch`/`localFetch` the pager needs. */
export type SpeakerSearchFetch = (
  path: string,
  init?: RequestInit
) => Promise<Pick<Response, "ok" | "status" | "json">>;

export interface FetchAllSpeakersOptions {
  name?: string;
  includeSamples?: boolean;
  pageSize?: number;
  maxPages?: number;
  signal?: AbortSignal;
}

/**
 * Every speaker matching `name`, gathered page by page.
 *
 * Paging stops at the first short page, at a page that adds no new speaker
 * (an engine that ignores `offset` returns the same list again), or after
 * `maxPages` pages. Speakers are de-duplicated by id, in case a rename moves a
 * name across a page boundary between two calls. A failed page throws, so the
 * caller can keep the list it already shows instead of a partial one.
 */
export async function fetchAllSpeakers<T extends { id: number }>(
  fetchPath: SpeakerSearchFetch,
  options: FetchAllSpeakersOptions = {}
): Promise<T[]> {
  const pageSize = Math.max(1, Math.floor(options.pageSize ?? SPEAKER_SEARCH_PAGE_SIZE));
  const maxPages = Math.max(1, Math.floor(options.maxPages ?? SPEAKER_SEARCH_MAX_PAGES));
  const seen = new Set<number>();
  const speakers: T[] = [];

  for (let page = 0; page < maxPages; page++) {
    const path = speakerSearchPath({
      name: options.name,
      limit: pageSize,
      offset: page * pageSize,
      includeSamples: options.includeSamples,
    });
    const response = await fetchPath(path, options.signal ? { signal: options.signal } : undefined);
    if (!response.ok) {
      throw new Error(`speaker search failed with status ${response.status}`);
    }
    const body: unknown = await response.json();
    const rows = Array.isArray(body) ? (body as T[]) : [];

    let added = 0;
    for (const row of rows) {
      if (row && typeof row.id === "number" && !seen.has(row.id)) {
        seen.add(row.id);
        speakers.push(row);
        added++;
      }
    }
    if (rows.length < pageSize || added === 0) break;
  }
  return speakers;
}
