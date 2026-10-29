// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

/**
 * Release notes for the changelog dialog.
 *
 * Source: the GitHub Releases of the Handvel repository, read without
 * authentication and only when the user opens the dialog. Results are kept
 * for an hour in localStorage so reopening the dialog does not hit GitHub
 * again; a stale copy is still shown if GitHub cannot be reached. When there
 * is neither, the dialog falls back to the CHANGELOG.md bundled with the app.
 */

import { FREENPIPE_REPO_URL } from "@/lib/web-url";

export interface ChangelogEntry {
  /** Version without a leading "v", e.g. "2.7.104". */
  version: string;
  /** Release title as written on GitHub (falls back to the tag). */
  title: string;
  /** Publication date as YYYY-MM-DD, or "" when GitHub gave none. */
  date: string;
  /** Release notes in Markdown. */
  body: string;
  /** Link to the release page. */
  url: string;
  prerelease: boolean;
}

const REPO_SLUG = FREENPIPE_REPO_URL.replace(/^https:\/\/github\.com\//, "");

export const RELEASES_API_URL = `https://api.github.com/repos/${REPO_SLUG}/releases?per_page=50`;
export const CHANGELOG_CACHE_KEY = "freenpipe:changelog-cache";
export const CHANGELOG_CACHE_TTL_MS = 60 * 60 * 1000;

type GithubRelease = {
  tag_name?: unknown;
  name?: unknown;
  body?: unknown;
  html_url?: unknown;
  published_at?: unknown;
  created_at?: unknown;
  draft?: unknown;
  prerelease?: unknown;
};

const str = (v: unknown): string => (typeof v === "string" ? v : "");

/** Turn the GitHub "list releases" payload into dialog entries. */
export function parseGithubReleases(payload: unknown): ChangelogEntry[] {
  if (!Array.isArray(payload)) return [];
  const entries: ChangelogEntry[] = [];
  for (const raw of payload as GithubRelease[]) {
    if (!raw || typeof raw !== "object" || raw.draft === true) continue;
    const tag = str(raw.tag_name).trim();
    if (!tag) continue;
    const when = str(raw.published_at) || str(raw.created_at);
    entries.push({
      version: tag.replace(/^v(?=\d)/i, ""),
      title: str(raw.name).trim() || tag,
      date: /^\d{4}-\d{2}-\d{2}/.test(when) ? when.slice(0, 10) : "",
      body: str(raw.body).replace(/\r\n/g, "\n").trim(),
      url: str(raw.html_url) || `${FREENPIPE_REPO_URL}/releases/tag/${encodeURIComponent(tag)}`,
      prerelease: raw.prerelease === true,
    });
  }
  return entries;
}

type CacheRecord = { fetchedAt: number; entries: ChangelogEntry[] };

/** Minimal storage surface, so tests can pass a fake. */
export type ChangelogStorage = Pick<Storage, "getItem" | "setItem">;

export function readChangelogCache(storage: ChangelogStorage | null): CacheRecord | null {
  if (!storage) return null;
  try {
    const raw = storage.getItem(CHANGELOG_CACHE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<CacheRecord>;
    if (typeof parsed.fetchedAt !== "number" || !Array.isArray(parsed.entries)) return null;
    return { fetchedAt: parsed.fetchedAt, entries: parsed.entries as ChangelogEntry[] };
  } catch {
    return null;
  }
}

function writeChangelogCache(storage: ChangelogStorage | null, record: CacheRecord): void {
  if (!storage) return;
  try {
    storage.setItem(CHANGELOG_CACHE_KEY, JSON.stringify(record));
  } catch {
    // Storage full or unavailable: the dialog still works, just uncached.
  }
}

export interface LoadChangelogOptions {
  fetchImpl?: typeof fetch;
  storage?: ChangelogStorage | null;
  now?: number;
}

/**
 * Release entries for the dialog, or null when none could be obtained (the
 * caller then shows the bundled CHANGELOG.md).
 */
export async function loadChangelog(opts: LoadChangelogOptions = {}): Promise<ChangelogEntry[] | null> {
  const fetchImpl = opts.fetchImpl ?? fetch;
  const storage =
    opts.storage !== undefined ? opts.storage : typeof localStorage !== "undefined" ? localStorage : null;
  const now = opts.now ?? Date.now();

  const cached = readChangelogCache(storage);
  if (cached && cached.entries.length > 0 && now - cached.fetchedAt < CHANGELOG_CACHE_TTL_MS) {
    return cached.entries;
  }

  try {
    const resp = await fetchImpl(RELEASES_API_URL, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (resp.ok) {
      const entries = parseGithubReleases(await resp.json());
      if (entries.length > 0) {
        writeChangelogCache(storage, { fetchedAt: now, entries });
        return entries;
      }
    }
  } catch {
    // Offline or blocked: fall through to whatever is cached.
  }

  return cached && cached.entries.length > 0 ? cached.entries : null;
}
