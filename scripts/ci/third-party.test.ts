// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

/**
 * Third-party dependency guard (W0-13).
 *
 * Git dependencies hosted by the upstream vendor's organisations or by its
 * contributors' personal accounts are vendored under third_party/ and used as
 * path dependencies (docs/clean-room/DECISIONS.md, decision 2). This test keeps
 * it that way:
 *
 *   - no Cargo.toml (vendored ones included) declares a git dependency on a
 *     repository owned by one of those accounts, and no Cargo.lock records one;
 *   - third_party/ stays out of the root workspace;
 *   - every path dependency into third_party/ names an existing crate;
 *   - third_party/README.md lists every vendored tree and every git source the
 *     lockfiles still fetch.
 *
 * Run from the repository root:  bun test scripts/ci/third-party.test.ts
 */

import { describe, expect, test } from "bun:test";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";

const REPO_ROOT = join(import.meta.dir, "..", "..");
const THIRD_PARTY = join(REPO_ROOT, "third_party");

/** GitHub accounts whose repositories must be vendored rather than fetched. */
export const VENDOR_OWNERS = [
  "screenpipe",
  "mediar-ai",
  "louis030195",
  "divanshu-go",
  "neo773",
  "tribhuwan-kumar",
];

/** The two lockfiles: the root workspace and the Tauri app crate's own workspace. */
const LOCKFILES = ["Cargo.lock", "apps/screenpipe-app-tauri/src-tauri/Cargo.lock"];

const SKIP_DIRS = new Set([".git", "node_modules", "target", ".next", "out", "dist", ".turbo"]);

export type GitDep = { url: string; line: number };

/** The owner segment of a GitHub URL (lower-cased), or null for other hosts. */
export function githubOwner(url: string): string | null {
  const m = /github\.com[/:]([^/\s"]+)\//i.exec(url);
  return m ? m[1].toLowerCase() : null;
}

/** Repository identity used to match lockfile sources against the README. */
export function normalizeRepoUrl(url: string): string {
  return url
    .replace(/^git\+/, "")
    .replace(/[?#].*$/, "")
    .replace(/\.git$/, "")
    .replace(/\/+$/, "")
    .toLowerCase();
}

/**
 * Every `git = "..."` key in a manifest, inline-table or dotted-table form.
 * Comments are dropped first so a commented-out dependency does not count.
 */
export function gitDepsInManifest(text: string): GitDep[] {
  const deps: GitDep[] = [];
  text.split(/\r?\n/).forEach((raw, i) => {
    const line = raw.replace(/(^|\s)#.*$/, "");
    for (const m of line.matchAll(/(?:^|[\s{,])git\s*=\s*"([^"]+)"/g)) {
      deps.push({ url: m[1], line: i + 1 });
    }
  });
  return deps;
}

/** Every `path = "..."` value in a manifest (with its line). */
export function pathDepsInManifest(text: string): { path: string; line: number }[] {
  const out: { path: string; line: number }[] = [];
  text.split(/\r?\n/).forEach((raw, i) => {
    const line = raw.replace(/(^|\s)#.*$/, "");
    for (const m of line.matchAll(/(?:^|[\s{,])path\s*=\s*"([^"]+)"/g)) {
      out.push({ path: m[1], line: i + 1 });
    }
  });
  return out;
}

/** The git sources a Cargo.lock records (one entry per package). */
export function gitSourcesInLock(text: string): string[] {
  return [...text.matchAll(/^source = "(git\+[^"]+)"/gm)].map((m) => m[1]);
}

function* manifests(dir: string): Generator<string> {
  let entries: string[];
  try {
    entries = readdirSync(dir);
  } catch {
    return;
  }
  for (const name of entries) {
    if (SKIP_DIRS.has(name)) continue;
    const full = join(dir, name);
    let st;
    try {
      st = statSync(full);
    } catch {
      continue;
    }
    if (st.isDirectory()) yield* manifests(full);
    else if (name === "Cargo.toml") yield full;
  }
}

function repoPath(full: string): string {
  return relative(REPO_ROOT, full).split(sep).join("/");
}

function isVendorOwned(url: string): boolean {
  const owner = githubOwner(url);
  return owner !== null && VENDOR_OWNERS.includes(owner);
}

function vendoredTrees(): string[] {
  return readdirSync(THIRD_PARTY)
    .filter((n) => n !== "patches" && statSync(join(THIRD_PARTY, n)).isDirectory())
    .sort();
}

describe("third-party dependency guard: parsers", () => {
  test("githubOwner reads the account from https and ssh forms", () => {
    expect(githubOwner("https://github.com/Screenpipe/whisper-rs.git")).toBe("screenpipe");
    expect(githubOwner("git+https://github.com/yury/cidre.git?rev=1#1")).toBe("yury");
    expect(githubOwner("git@github.com:neo773/hf-hub")).toBe("neo773");
    expect(githubOwner("https://gitlab.com/screenpipe/x")).toBeNull();
  });

  test("normalizeRepoUrl ignores the git+ prefix, query, fragment and .git suffix", () => {
    expect(
      normalizeRepoUrl("git+https://github.com/yury/cidre.git?rev=8481f3f#8481f3f0dc7dd39b"),
    ).toBe("https://github.com/yury/cidre");
    expect(normalizeRepoUrl("https://github.com/eiz/accessibility/")).toBe(
      "https://github.com/eiz/accessibility",
    );
  });

  test("gitDepsInManifest finds inline and dotted-table git keys, not comments", () => {
    const toml = [
      'a = { git = "https://github.com/x/a", rev = "1" }',
      "[dependencies.b]",
      'git = "https://github.com/x/b"',
      '# c = { git = "https://github.com/x/c" }',
      'd = { version = "1", features = ["git"] }',
      'e = { path = "../e" } # was git = "https://github.com/x/e"',
    ].join("\n");
    expect(gitDepsInManifest(toml)).toEqual([
      { url: "https://github.com/x/a", line: 1 },
      { url: "https://github.com/x/b", line: 3 },
    ]);
  });

  test("pathDepsInManifest and gitSourcesInLock", () => {
    expect(pathDepsInManifest('x = { path = "../../third_party/x", optional = true }')).toEqual([
      { path: "../../third_party/x", line: 1 },
    ]);
    const lock = [
      "[[package]]",
      'name = "a"',
      'source = "registry+https://github.com/rust-lang/crates.io-index"',
      "[[package]]",
      'name = "b"',
      'source = "git+https://github.com/x/b?rev=1#1"',
    ].join("\n");
    expect(gitSourcesInLock(lock)).toEqual(["git+https://github.com/x/b?rev=1#1"]);
  });
});

describe("third-party dependency guard: repository", () => {
  test("no Cargo.toml declares a git dependency on a vendor-owned repository", () => {
    const offenders: string[] = [];
    for (const file of manifests(REPO_ROOT)) {
      for (const dep of gitDepsInManifest(readFileSync(file, "utf8"))) {
        if (isVendorOwned(dep.url)) offenders.push(`${repoPath(file)}:${dep.line}: ${dep.url}`);
      }
    }
    expect(offenders).toEqual([]);
  });

  test("no Cargo.lock records a git source from a vendor-owned repository", () => {
    const offenders: string[] = [];
    for (const lock of LOCKFILES) {
      for (const src of gitSourcesInLock(readFileSync(join(REPO_ROOT, lock), "utf8"))) {
        if (isVendorOwned(src)) offenders.push(`${lock}: ${src}`);
      }
    }
    expect(offenders).toEqual([]);
  });

  test("third_party/ is excluded from the root workspace", () => {
    const root = readFileSync(join(REPO_ROOT, "Cargo.toml"), "utf8");
    const exclude = /^exclude\s*=\s*\[([\s\S]*?)\]/m.exec(root);
    expect(exclude).not.toBeNull();
    expect(exclude![1]).toMatch(/"third_party"/);
  });

  test("every path dependency into third_party/ names an existing crate", () => {
    const broken: string[] = [];
    let seen = 0;
    for (const file of manifests(REPO_ROOT)) {
      const rel = repoPath(file);
      if (rel.startsWith("third_party/")) continue;
      for (const dep of pathDepsInManifest(readFileSync(file, "utf8"))) {
        if (!dep.path.includes("third_party/")) continue;
        seen++;
        const target = resolve(dirname(file), dep.path);
        if (!existsSync(join(target, "Cargo.toml"))) broken.push(`${rel}:${dep.line}: ${dep.path}`);
      }
    }
    expect(broken).toEqual([]);
    expect(seen).toBeGreaterThan(0);
  });

  test("third_party/README.md lists every vendored tree", () => {
    const readme = readFileSync(join(THIRD_PARTY, "README.md"), "utf8");
    const missing = vendoredTrees().filter((name) => !readme.includes(`| \`${name}\` |`));
    expect(missing).toEqual([]);
  });

  test("third_party/README.md lists every git source the lockfiles still fetch", () => {
    const readme = readFileSync(join(THIRD_PARTY, "README.md"), "utf8").toLowerCase();
    const missing = new Set<string>();
    for (const lock of LOCKFILES) {
      for (const src of gitSourcesInLock(readFileSync(join(REPO_ROOT, lock), "utf8"))) {
        const repo = normalizeRepoUrl(src);
        const rev = /#([0-9a-f]{40})$/.exec(src)?.[1];
        if (!readme.includes(repo) || (rev && !readme.includes(rev))) {
          missing.add(`${lock}: ${src}`);
        }
      }
    }
    expect([...missing]).toEqual([]);
  });
});
