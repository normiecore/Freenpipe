// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

/**
 * Licence guard (W1-05, owner decision 7 in docs/clean-room/DECISIONS.md).
 *
 * Every build the project ships has to be usable commercially. This test asks
 * cargo which packages each shipped build links and fails when one of them
 * would forbid that. "Shipped builds" are:
 *
 *   - screenpipe-engine and the Tauri app crate with their default features;
 *   - every feature set the release workflows pass: the app's `args` and
 *     `tauri-args` in .github/workflows/release-app.yml, and the engine's
 *     `cargo build -p screenpipe-engine` lines in release-cli.yml. They are
 *     read from the workflow files, so a change there is checked here too.
 *
 * For each of them `cargo tree -e normal,build` lists the linked packages
 * (dev-dependencies do not ship) with their `license` field, and the test
 * fails when a package
 *
 *   1. is one of the opt-in-only crates: opf-rs (CC BY-NC 4.0) and tinfoil
 *      (AGPL-3.0) may only enter a build through the `redact-opf-text` and
 *      `tinfoil-enclave` features (third_party/README.md);
 *   2. has a licence expression that cannot be met with the accepted licences
 *      below. GPL (but not LGPL), AGPL, SSPL, BUSL, the Elastic licence and
 *      CC BY-NC are refused by name; any identifier nobody has reviewed yet
 *      fails as well. Permissive licences, MPL-2.0 and LGPL (weak copyleft)
 *      pass;
 *   3. declares no licence at all, unless it is Handvel's own crate (covered
 *      by LICENSE.md) or listed in DECLARED_ELSEWHERE with a reason.
 *
 * The test needs no network: it runs cargo with `--offline --locked`, so the
 * package cache must hold both lockfiles' packages (CI's gates job fetches
 * them first). Locally, if cargo reports a missing package, run once:
 *
 *   cargo fetch --locked
 *   cargo fetch --locked --manifest-path apps/screenpipe-app-tauri/src-tauri/Cargo.toml
 *
 * Run from the repository root:  bun test scripts/ci/licence-guard.test.ts
 */

import { afterAll, describe, expect, test } from "bun:test";
import {
  copyFileSync,
  existsSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  symlinkSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { isAbsolute, join, relative, sep } from "node:path";

const REPO_ROOT = join(import.meta.dir, "..", "..");
const APP_MANIFEST = join("apps", "screenpipe-app-tauri", "src-tauri", "Cargo.toml");
const ENGINE_PACKAGE = "screenpipe-engine";

/** Generous: the first cargo call may wait for the package-cache lock. */
const CARGO_TIMEOUT_MS = 300_000;

// ---------------------------------------------------------------------------
// Policy
// ---------------------------------------------------------------------------

/**
 * Crates that may only be linked through an opt-in feature. A shipped feature
 * set that reaches one of them fails whatever its licence field says (opf-rs
 * declares none).
 */
export const OPT_IN_ONLY: Record<string, string> = {
  opf: "opf-rs is CC BY-NC 4.0 (no commercial use); only the opt-in `redact-opf-text` / `opf-text` features may link it",
  tinfoil: "the Tinfoil SDK is AGPL-3.0; only the opt-in `tinfoil-enclave` feature may link it",
};

/**
 * SPDX identifiers reviewed as fine for a commercially used binary. Compared
 * case-insensitively after `+`, `-only` and `-or-later` are dropped.
 */
export const ACCEPTED_LICENCES: string[] = [
  // Permissive.
  "MIT",
  "MIT-0",
  "Apache-2.0",
  "BSD-1-Clause",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "0BSD",
  "ISC",
  "Zlib",
  "BSL-1.0",
  "NCSA",
  "X11",
  "bzip2-1.0.6",
  "OpenSSL",
  // Public-domain dedications.
  "Unlicense",
  "CC0-1.0",
  // Data and attribution licences (Unicode tables, CA root lists, CC BY
  // content with attribution, as decision 7 allows).
  "Unicode-3.0",
  "Unicode-DFS-2016",
  "CDLA-Permissive-2.0",
  "CC-BY-3.0",
  "CC-BY-4.0",
  // Weak copyleft: file-level (MPL) or library-level (LGPL). Allowed by
  // decision 7; the release licence audit covers their notice duties.
  "MPL-2.0",
  "LGPL-2.0",
  "LGPL-2.1",
  "LGPL-3.0",
];

/** Licence families refused outright, with the reason shown in failures. */
export const REFUSED_LICENCES: { pattern: RegExp; why: string }[] = [
  { pattern: /^AGPL-/i, why: "strong network copyleft (AGPL)" },
  { pattern: /^GPL-/i, why: "strong copyleft (GPL)" },
  { pattern: /^SSPL/i, why: "source-available, not open source (SSPL)" },
  { pattern: /^BUSL/i, why: "source-available, restricts production use (BUSL)" },
  { pattern: /^Elastic-/i, why: "source-available (Elastic License)" },
  { pattern: /^CC-BY-NC/i, why: "non-commercial only (CC BY-NC)" },
];

/**
 * Packages that declare no `license` in their Cargo.toml although their
 * licence is known. Keep this short; each entry says where the licence is
 * declared. opf-rs must never be added here.
 */
export const DECLARED_ELSEWHERE: Record<string, { licence: string; where: string }> = {
  // macOS app only (git dependency). Upstream ships LICENSE-APACHE and
  // LICENSE-MIT but no `license` field; third_party/README.md lists it under
  // "Git dependencies that stay".
  "tauri-nspanel": { licence: "Apache-2.0 OR MIT", where: "third_party/README.md" },
};

// ---------------------------------------------------------------------------
// Licence expressions
// ---------------------------------------------------------------------------

export type LicenceExpr =
  | { kind: "id"; id: string; exception?: string }
  | { kind: "and" | "or"; parts: LicenceExpr[] };

/** Split an SPDX expression; the old `/` separator means OR. */
export function tokenizeLicence(expr: string): string[] {
  return expr
    .replace(/\//g, " OR ")
    .replace(/\(/g, " ( ")
    .replace(/\)/g, " ) ")
    .split(/\s+/)
    .filter(Boolean);
}

/** Parse an SPDX licence expression (AND binds tighter than OR). */
export function parseLicence(expr: string): LicenceExpr {
  const tokens = tokenizeLicence(expr);
  let pos = 0;
  const fail = (what: string): never => {
    throw new Error(`${what} in licence expression "${expr}"`);
  };
  const isWord = (t: string | undefined, word: string) => t !== undefined && t.toUpperCase() === word;

  const parseAtom = (): LicenceExpr => {
    const t = tokens[pos++];
    if (t === undefined) return fail("unexpected end");
    if (t === "(") {
      const inner = parseOr();
      if (tokens[pos++] !== ")") fail("missing )");
      return inner;
    }
    if (t === ")" || isWord(t, "AND") || isWord(t, "OR") || isWord(t, "WITH")) {
      return fail(`unexpected "${t}"`);
    }
    if (isWord(tokens[pos], "WITH")) {
      pos++;
      const exception = tokens[pos++];
      if (exception === undefined || exception === "(" || exception === ")") fail("missing exception");
      return { kind: "id", id: t, exception };
    }
    return { kind: "id", id: t };
  };
  const parseAnd = (): LicenceExpr => {
    const parts = [parseAtom()];
    while (isWord(tokens[pos], "AND")) {
      pos++;
      parts.push(parseAtom());
    }
    return parts.length === 1 ? parts[0] : { kind: "and", parts };
  };
  const parseOr = (): LicenceExpr => {
    const parts = [parseAnd()];
    while (isWord(tokens[pos], "OR")) {
      pos++;
      parts.push(parseAnd());
    }
    return parts.length === 1 ? parts[0] : { kind: "or", parts };
  };

  if (tokens.length === 0) fail("empty");
  const result = parseOr();
  if (pos !== tokens.length) fail(`unexpected "${tokens[pos]}"`);
  return result;
}

/** `GPL-2.0+`, `LGPL-3.0-or-later` and `MIT` → the base identifier. */
export function baseLicenceId(id: string): string {
  return id.replace(/\+$/, "").replace(/-(only|or-later)$/i, "");
}

const ACCEPTED_LOWER = new Set(ACCEPTED_LICENCES.map((l) => l.toLowerCase()));

/** null when the identifier is accepted, else why it is not. */
export function judgeLicenceId(id: string): string | null {
  const base = baseLicenceId(id);
  const refused = REFUSED_LICENCES.find((r) => r.pattern.test(base));
  if (refused) return `${id}: ${refused.why}`;
  if (ACCEPTED_LOWER.has(base.toLowerCase())) return null;
  return `${id}: not on the reviewed licence list (scripts/ci/licence-guard.test.ts)`;
}

export type Verdict = { ok: boolean; problems: string[] };

/** Whether the expression can be satisfied with accepted licences only. */
export function judgeLicence(expr: string): Verdict {
  if (expr.trim() === "") return { ok: false, problems: ["no licence declared"] };
  let tree: LicenceExpr;
  try {
    tree = parseLicence(expr);
  } catch (e) {
    return { ok: false, problems: [(e as Error).message] };
  }
  const walk = (node: LicenceExpr): Verdict => {
    if (node.kind === "id") {
      const problem = judgeLicenceId(node.id);
      return problem ? { ok: false, problems: [problem] } : { ok: true, problems: [] };
    }
    const results = node.parts.map(walk);
    const ok = node.kind === "or" ? results.some((r) => r.ok) : results.every((r) => r.ok);
    return { ok, problems: ok ? [] : [...new Set(results.flatMap((r) => r.problems))] };
  };
  return walk(tree);
}

/** MPL or LGPL somewhere in an accepted expression (reported, not failed). */
export function isWeakCopyleft(expr: string): boolean {
  return /\b(MPL|LGPL)-/i.test(expr) && !/\bOR\b|\//i.test(expr);
}

// ---------------------------------------------------------------------------
// cargo tree output
// ---------------------------------------------------------------------------

export type CratePkg = {
  name: string;
  version: string;
  /** Path or git source as cargo prints it; null for crates.io. */
  source: string | null;
  licence: string;
};

/**
 * One line of `cargo tree --prefix none --format "{p}\t{l}"`, for example
 * `opf v0.1.0 (/repo/third_party/opf-rs)\t` or `tinfoil v0.1.0 (https://…)\tAGPL-3.0 (*)`.
 */
export function parseTreeLine(line: string): CratePkg | null {
  const trimmed = line.replace(/\r$/, "").replace(/ \(\*\)$/, "");
  if (trimmed.trim() === "") return null;
  const tab = trimmed.indexOf("\t");
  const pkg = tab === -1 ? trimmed : trimmed.slice(0, tab);
  const licence = tab === -1 ? "" : trimmed.slice(tab + 1).trim();
  const m = /^(\S+) v(\S+)(.*)$/.exec(pkg.trim());
  if (!m) return null;
  let source: string | null = null;
  for (const group of m[3].matchAll(/\(([^()]*)\)/g)) {
    if (group[1] !== "proc-macro") {
      source = group[1];
      break;
    }
  }
  return { name: m[1], version: m[2], source, licence };
}

/** The unique packages of a `cargo tree` listing. */
export function parseTree(output: string): CratePkg[] {
  const seen = new Map<string, CratePkg>();
  for (const line of output.split("\n")) {
    const pkg = parseTreeLine(line);
    if (pkg) seen.set(`${pkg.name}@${pkg.version}@${pkg.source ?? ""}`, pkg);
  }
  return [...seen.values()];
}

/** A path source inside `root` but outside third_party/: Handvel's own crate. */
export function isOwnCrate(pkg: CratePkg, root: string): boolean {
  if (!pkg.source || !(isAbsolute(pkg.source) || /^[A-Za-z]:[\\/]/.test(pkg.source))) return false;
  const rel = relative(root, pkg.source);
  if (rel === "" || rel.startsWith("..") || isAbsolute(rel)) return false;
  return rel.split(/[\\/]/)[0] !== "third_party";
}

export type Violation = { pkg: string; problem: string };

/** Everything in `pkgs` that a shipped build may not contain. */
export function licenceViolations(pkgs: CratePkg[], root: string): Violation[] {
  const out: Violation[] = [];
  for (const pkg of pkgs) {
    const label = `${pkg.name} ${pkg.version}${pkg.source ? ` (${pkg.source})` : ""}`;
    if (OPT_IN_ONLY[pkg.name]) {
      out.push({ pkg: label, problem: OPT_IN_ONLY[pkg.name] });
      continue;
    }
    let licence = pkg.licence;
    if (licence === "") {
      if (isOwnCrate(pkg, root)) continue; // LICENSE.md covers the repository
      const declared = DECLARED_ELSEWHERE[pkg.name];
      if (declared) licence = declared.licence;
    }
    const verdict = judgeLicence(licence);
    if (!verdict.ok) out.push({ pkg: label, problem: verdict.problems.join("; ") });
  }
  return out.sort((a, b) => a.pkg.localeCompare(b.pkg));
}

// ---------------------------------------------------------------------------
// Feature sets
// ---------------------------------------------------------------------------

export type CargoArgs = {
  target: string | null;
  features: string[];
  noDefault: boolean;
  allFeatures: boolean;
};

/** The flags of a cargo / tauri command line that change what gets linked. */
export function parseCargoArgs(line: string): CargoArgs {
  const tokens = line.replace(/["']/g, " ").split(/\s+/).filter(Boolean);
  const out: CargoArgs = { target: null, features: [], noDefault: false, allFeatures: false };
  const addFeatures = (list: string | undefined) => {
    for (const f of (list ?? "").split(/[,\s]+/)) if (f) out.features.push(f);
  };
  for (let i = 0; i < tokens.length; i++) {
    const t = tokens[i];
    if (t === "--features" || t === "-F") addFeatures(tokens[++i]);
    else if (t.startsWith("--features=")) addFeatures(t.slice("--features=".length));
    else if (t === "--target") out.target = tokens[++i] ?? null;
    else if (t.startsWith("--target=")) out.target = t.slice("--target=".length);
    else if (t === "--no-default-features") out.noDefault = true;
    else if (t === "--all-features") out.allFeatures = true;
  }
  // A templated target (`${{ matrix.target }}`) is resolved for every platform.
  if (out.target !== null && !/^[A-Za-z0-9_.-]+$/.test(out.target)) out.target = null;
  out.features = [...new Set(out.features)].sort();
  return out;
}

export type ShippedBuild = CargoArgs & {
  crate: "engine" | "app";
  /** Where the feature set comes from, for test names. */
  origin: string;
};

/**
 * The app feature sets in release-app.yml (`args:` and `tauri-args:` matrix
 * values) and the engine feature sets in release-cli.yml.
 */
export function releaseBuilds(releaseAppYml: string, releaseCliYml: string): ShippedBuild[] {
  const builds: ShippedBuild[] = [];
  for (const m of releaseAppYml.matchAll(/^\s*(tauri-args|args):\s*"([^"]*)"/gm)) {
    const args = parseCargoArgs(m[2]);
    // The workflow's "Compute Tauri build args" step appends
    // apple-intelligence on aarch64-apple-darwin when the runner's Xcode
    // supports it, so that set is checked with it.
    if (m[1] === "tauri-args" && args.target === "aarch64-apple-darwin") {
      args.features = [...new Set([...args.features, "apple-intelligence"])].sort();
    }
    builds.push({ ...args, crate: "app", origin: `release-app.yml ${m[1]}` });
  }
  for (const line of releaseCliYml.split("\n")) {
    const code = line.replace(/(^|\s)#.*$/, "");
    if (!/\bcargo\s+build\b/.test(code) || !/(-p|--package)\s+screenpipe-engine\b/.test(code)) continue;
    builds.push({ ...parseCargoArgs(code), crate: "engine", origin: "release-cli.yml" });
  }
  return dedupeBuilds(builds);
}

function buildKey(b: ShippedBuild): string {
  return [b.crate, b.target ?? "all", b.noDefault, b.allFeatures, b.features.join(",")].join("|");
}

function dedupeBuilds(builds: ShippedBuild[]): ShippedBuild[] {
  const seen = new Map<string, ShippedBuild>();
  for (const b of builds) if (!seen.has(buildKey(b))) seen.set(buildKey(b), b);
  return [...seen.values()];
}

/** Defaults of both crates, on every platform, plus the release sets. */
export function shippedBuilds(): ShippedBuild[] {
  const read = (p: string) => readFileSync(join(REPO_ROOT, p), "utf8");
  const defaults: ShippedBuild[] = (["engine", "app"] as const).map((crate) => ({
    crate,
    origin: "default features",
    target: null,
    features: [],
    noDefault: false,
    allFeatures: false,
  }));
  return dedupeBuilds([
    ...defaults,
    ...releaseBuilds(
      read(".github/workflows/release-app.yml"),
      read(".github/workflows/release-cli.yml"),
    ),
  ]);
}

export function describeBuild(b: ShippedBuild): string {
  const features = b.allFeatures ? "--all-features" : b.features.length ? b.features.join(",") : "(none)";
  const defaults = b.noDefault ? "no defaults" : "defaults";
  return `${b.crate} @ ${b.target ?? "all targets"}: ${defaults} + ${features} [${b.origin}]`;
}

// ---------------------------------------------------------------------------
// Running cargo
// ---------------------------------------------------------------------------

/** `cargo tree` for one build, from the workspace rooted at `root`. */
export function cargoTree(b: ShippedBuild, root: string = REPO_ROOT): CratePkg[] {
  const args = [
    "tree",
    "--offline",
    "--locked",
    "-e",
    "normal,build",
    "--prefix",
    "none",
    "--format",
    "{p}\t{l}",
    "--target",
    b.target ?? "all",
  ];
  if (b.crate === "engine") args.push("-p", ENGINE_PACKAGE);
  else args.push("--manifest-path", join(root, APP_MANIFEST));
  if (b.noDefault) args.push("--no-default-features");
  if (b.allFeatures) args.push("--all-features");
  else if (b.features.length) args.push("--features", b.features.join(","));

  const proc = Bun.spawnSync(["cargo", ...args], {
    cwd: root,
    env: { ...process.env, CARGO_TERM_COLOR: "never" },
    stdout: "pipe",
    stderr: "pipe",
  });
  const stdout = proc.stdout.toString();
  if (proc.exitCode !== 0) {
    const stderr = proc.stderr.toString();
    const hint = /offline|download|not found in the cache|failed to load source/i.test(stderr)
      ? "\nThe package cache is incomplete: run `cargo fetch --locked` for the root and the app manifest (see the header of this file)."
      : "";
    throw new Error(`cargo ${args.join(" ")} failed (exit ${proc.exitCode}):\n${stderr}${hint}`);
  }
  const pkgs = parseTree(stdout);
  if (pkgs.length === 0) throw new Error(`cargo tree printed no packages for ${describeBuild(b)}`);
  return pkgs;
}

/**
 * A copy of the root workspace with one manifest of screenpipe-engine edited,
 * built from symlinks so nothing is copied but the manifests. The returned
 * cleanup removes exactly what was created; it never follows a symlink.
 */
export function overlayWorkspace(editEngineManifest: (text: string) => string): {
  root: string;
  cleanup: () => void;
} {
  const root = mkdtempSync(join(tmpdir(), "handvel-licence-guard-"));
  const links: string[] = [];
  const link = (target: string, path: string) => {
    symlinkSync(target, path, lstatSync(target).isDirectory() ? "dir" : "file");
    links.push(path);
  };
  for (const f of ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml"]) {
    if (existsSync(join(REPO_ROOT, f))) copyFileSync(join(REPO_ROOT, f), join(root, f));
  }
  for (const d of [".cargo", "third_party"]) link(join(REPO_ROOT, d), join(root, d));
  mkdirSync(join(root, "crates"));
  for (const entry of readdirSync(join(REPO_ROOT, "crates"))) {
    if (entry !== ENGINE_PACKAGE) link(join(REPO_ROOT, "crates", entry), join(root, "crates", entry));
  }
  const engineSrc = join(REPO_ROOT, "crates", ENGINE_PACKAGE);
  const engineDst = join(root, "crates", ENGINE_PACKAGE);
  mkdirSync(engineDst);
  for (const entry of readdirSync(engineSrc)) {
    if (entry !== "Cargo.toml") link(join(engineSrc, entry), join(engineDst, entry));
  }
  writeFileSync(
    join(engineDst, "Cargo.toml"),
    editEngineManifest(readFileSync(join(engineSrc, "Cargo.toml"), "utf8")),
  );
  const cleanup = () => {
    // Symlinks first, one by one, so the recursive removal below only ever
    // sees the directories and copied files created here.
    for (const p of links) {
      try {
        if (lstatSync(p).isSymbolicLink()) unlinkSync(p);
      } catch {
        // already gone
      }
    }
    for (const p of links) {
      if (existsSync(p)) throw new Error(`refusing to remove ${root}: ${p} is still present`);
    }
    rmSync(root, { recursive: true, force: true });
  };
  return { root, cleanup };
}

/** Apply `edit` to the engine manifest and fail loudly when it no longer matches. */
function mustReplace(text: string, pattern: RegExp, replacement: (m: string) => string): string {
  if (!pattern.test(text)) throw new Error(`the engine manifest no longer matches ${pattern}; update the negative test`);
  return text.replace(pattern, replacement);
}

const SHIPPED = shippedBuilds();

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe("licence guard: policy", () => {
  test("accepted, weak-copyleft and dual-licensed expressions pass", () => {
    for (const expr of [
      "MIT",
      "MIT OR Apache-2.0",
      "MIT/Apache-2.0",
      "Apache-2.0 / MIT",
      "Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT",
      "(MIT OR Apache-2.0) AND Unicode-3.0",
      "ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause",
      "MPL-2.0",
      "LGPL-3.0",
      "LGPL-2.1-or-later",
      "MIT OR LGPL-3.0-or-later",
      "MIT OR GPL-3.0",
      "CC0-1.0",
      "CDLA-Permissive-2.0",
      "BSL-1.0",
      "Unlicense OR MIT",
    ]) {
      expect({ expr, ...judgeLicence(expr) }).toEqual({ expr, ok: true, problems: [] });
    }
  });

  test("copyleft, source-available, non-commercial, unknown and missing licences fail", () => {
    for (const expr of [
      "GPL-3.0",
      "GPL-2.0+",
      "GPL-3.0-only",
      "AGPL-3.0",
      "AGPL-3.0-or-later",
      "SSPL-1.0",
      "BUSL-1.1",
      "Elastic-2.0",
      "CC-BY-NC-4.0",
      "CC-BY-NC-SA-4.0",
      "MIT AND GPL-2.0",
      "LicenseRef-Proprietary",
      "",
      "MIT OR",
      "(MIT",
    ]) {
      expect({ expr, ok: judgeLicence(expr).ok }).toEqual({ expr, ok: false });
    }
    expect(judgeLicence("AGPL-3.0").problems[0]).toContain("AGPL");
    expect(judgeLicence("CC-BY-NC-4.0").problems[0]).toContain("non-commercial");
    expect(judgeLicence("").problems).toEqual(["no licence declared"]);
  });

  test("LGPL is accepted but GPL is not, whatever the suffix", () => {
    expect(judgeLicenceId("LGPL-3.0-only")).toBeNull();
    expect(judgeLicenceId("LGPL-2.1+")).toBeNull();
    expect(judgeLicenceId("GPL-2.0-or-later")).toContain("strong copyleft");
  });

  test("the allow-list is short, explained, never covers an opt-in crate and holds accepted licences", () => {
    expect(Object.keys(DECLARED_ELSEWHERE).length).toBeLessThanOrEqual(5);
    for (const [name, entry] of Object.entries(DECLARED_ELSEWHERE)) {
      expect(OPT_IN_ONLY[name]).toBeUndefined();
      expect(entry.where.length).toBeGreaterThan(0);
      expect({ name, ...judgeLicence(entry.licence) }).toEqual({ name, ok: true, problems: [] });
    }
  });
});

describe("licence guard: parsers", () => {
  test("cargo tree lines: crates.io, path, git, proc-macro, dedupe marker, empty licence", () => {
    expect(parseTreeLine("serde v1.0.228\tMIT OR Apache-2.0")).toEqual({
      name: "serde",
      version: "1.0.228",
      source: null,
      licence: "MIT OR Apache-2.0",
    });
    expect(parseTreeLine("opf v0.1.0 (/repo/third_party/opf-rs)\t")).toEqual({
      name: "opf",
      version: "0.1.0",
      source: "/repo/third_party/opf-rs",
      licence: "",
    });
    expect(
      parseTreeLine("tinfoil v0.1.0 (https://github.com/tinfoilsh/tinfoil-rs?rev=82143d7#82143d76)\tAGPL-3.0 (*)"),
    ).toEqual({
      name: "tinfoil",
      version: "0.1.0",
      source: "https://github.com/tinfoilsh/tinfoil-rs?rev=82143d7#82143d76",
      licence: "AGPL-3.0",
    });
    expect(parseTreeLine("yoke-derive v0.8.2 (proc-macro)\tUnicode-3.0")?.source).toBeNull();
    expect(parseTreeLine("")).toBeNull();
    expect(parseTree("a v1.0.0\tMIT\na v1.0.0\tMIT (*)\nb v2.0.0\tMIT\n")).toHaveLength(2);
  });

  test("own crates are path packages outside third_party/", () => {
    const pkg = (source: string): CratePkg => ({ name: "x", version: "1.0.0", source, licence: "" });
    expect(isOwnCrate(pkg("/repo/crates/screenpipe-secrets"), "/repo")).toBe(true);
    expect(isOwnCrate(pkg("/repo/apps/screenpipe-app-tauri/src-tauri"), "/repo")).toBe(true);
    expect(isOwnCrate(pkg("/repo/third_party/opf-rs"), "/repo")).toBe(false);
    expect(isOwnCrate(pkg("/elsewhere/crate"), "/repo")).toBe(false);
    expect(isOwnCrate(pkg("https://github.com/o/r#abc"), "/repo")).toBe(false);
  });

  test("a missing licence fails unless the crate is ours or declared elsewhere; opt-in crates always fail", () => {
    const root = `${sep}repo`;
    const pkgs: CratePkg[] = [
      { name: "screenpipe-secrets", version: "0.1.0", source: join(root, "crates", "screenpipe-secrets"), licence: "" },
      { name: "tauri-nspanel", version: "2.0.1", source: "https://github.com/ahkohd/tauri-nspanel#18ffb9a2", licence: "" },
      { name: "mystery", version: "0.1.0", source: null, licence: "" },
      { name: "opf", version: "0.1.0", source: join(root, "third_party", "opf-rs"), licence: "" },
      { name: "tinfoil", version: "0.1.0", source: "https://github.com/tinfoilsh/tinfoil-rs#82143d76", licence: "AGPL-3.0" },
      { name: "fine", version: "1.0.0", source: null, licence: "MIT" },
    ];
    const names = licenceViolations(pkgs, root).map((v) => v.pkg.split(" ")[0]);
    expect(names).toEqual(["mystery", "opf", "tinfoil"]);
  });

  test("cargo arguments: target, feature lists, templated targets", () => {
    expect(parseCargoArgs("--target x86_64-pc-windows-msvc --features official-build,directml")).toEqual({
      target: "x86_64-pc-windows-msvc",
      features: ["directml", "official-build"],
      noDefault: false,
      allFeatures: false,
    });
    const templated = parseCargoArgs(
      "cargo build --release -p screenpipe-engine --features=metal --target ${{ matrix.target }} --no-default-features",
    );
    expect(templated.target).toBeNull();
    expect(templated.features).toEqual(["metal"]);
    expect(templated.noDefault).toBe(true);
    expect(parseCargoArgs("--all-features").allFeatures).toBe(true);
  });

  test("release feature sets are read from both workflows, Windows included", () => {
    const app = `
        include:
          - platform: windows-2022
            args: "--target x86_64-pc-windows-msvc"
            tauri-args: "--target x86_64-pc-windows-msvc --features official-build,directml,redact-onnx-directml"
          - platform: macos
            tauri-args: "--target aarch64-apple-darwin --features metal,official-build"
`;
    const cli = `
      run: |
        cargo build --release -p screenpipe-engine --bin screenpipe --features directml,redact-onnx-directml --target x86_64-pc-windows-msvc
        # cargo build -p screenpipe-engine --features opf-text   (a comment, ignored)
        cargo build --release -p screenpipe-engine --bin screenpipe --features redact-onnx-cpu --target \${{ matrix.target }}
`;
    const builds = releaseBuilds(app, cli).map(describeBuild);
    expect(builds).toEqual([
      "app @ x86_64-pc-windows-msvc: defaults + (none) [release-app.yml args]",
      "app @ x86_64-pc-windows-msvc: defaults + directml,official-build,redact-onnx-directml [release-app.yml tauri-args]",
      "app @ aarch64-apple-darwin: defaults + apple-intelligence,metal,official-build [release-app.yml tauri-args]",
      "engine @ x86_64-pc-windows-msvc: defaults + directml,redact-onnx-directml [release-cli.yml]",
      "engine @ all targets: defaults + redact-onnx-cpu [release-cli.yml]",
    ]);
  });

  test("the real workflows yield Windows release builds of both crates", () => {
    const windows = SHIPPED.filter((b) => b.target === "x86_64-pc-windows-msvc");
    expect(windows.some((b) => b.crate === "app" && b.features.length > 0)).toBe(true);
    expect(windows.some((b) => b.crate === "engine" && b.features.length > 0)).toBe(true);
    expect(SHIPPED.filter((b) => b.origin === "default features")).toHaveLength(2);
  });
});

describe("licence guard: shipped builds", () => {
  // Weak-copyleft packages are allowed but listed once at the end, for the
  // release licence audit (notices, LGPL relinking terms).
  const weakCopyleft = new Map<string, string>();
  afterAll(() => {
    if (weakCopyleft.size === 0) return;
    const list = [...weakCopyleft].sort(([a], [b]) => a.localeCompare(b)).map(([n, l]) => `${n} (${l})`);
    console.log(`licence-guard: weak-copyleft packages in shipped builds: ${list.join(", ")}`);
  });

  for (const build of SHIPPED) {
    test(
      describeBuild(build),
      () => {
        const pkgs = cargoTree(build);
        const weak = pkgs.filter((p) => isWeakCopyleft(p.licence));
        for (const p of weak) weakCopyleft.set(p.name, p.licence);
        console.log(
          `licence-guard: ${describeBuild(build)}: ${pkgs.length} packages, ${weak.length} weak copyleft`,
        );
        expect(pkgs.some((p) => p.name === (build.crate === "engine" ? ENGINE_PACKAGE : "screenpipe-app"))).toBe(true);
        expect(licenceViolations(pkgs, REPO_ROOT)).toEqual([]);
      },
      CARGO_TIMEOUT_MS,
    );
  }
});

describe("licence guard: negative checks", () => {
  test(
    "the opt-in features still reach opf-rs and tinfoil, and the gate reports both",
    () => {
      for (const crate of ["engine", "app"] as const) {
        const pkgs = cargoTree({
          crate,
          origin: "opt-in features",
          target: null,
          features: ["redact-opf-text", "tinfoil-enclave"],
          noDefault: false,
          allFeatures: false,
        });
        const flagged = licenceViolations(pkgs, REPO_ROOT).map((v) => v.pkg.split(" ")[0]);
        expect({ crate, opf: flagged.includes("opf"), tinfoil: flagged.includes("tinfoil") }).toEqual({
          crate,
          opf: true,
          tinfoil: true,
        });
      }
    },
    CARGO_TIMEOUT_MS,
  );

  const reEnabled: { name: string; edit: (text: string) => string; expect: string }[] = [
    {
      // Exactly the pre-W1-05 manifest line.
      name: "re-enabling opf-text on the engine's screenpipe-redact dependency fails the gate",
      edit: (text) =>
        mustReplace(text, /^screenpipe-redact = \{ path = "\.\.\/screenpipe-redact" \}$/m, () =>
          'screenpipe-redact = { path = "../screenpipe-redact", features = ["opf-text"] }',
        ),
      expect: "opf",
    },
    {
      name: "adding redact-opf-text to the engine's default features fails the gate",
      edit: (text) => mustReplace(text, /^default = \[(.*)\]$/m, (m) => m.replace(/\]$/, ', "redact-opf-text"]')),
      expect: "opf",
    },
    {
      name: "adding tinfoil-enclave to the engine's default features fails the gate",
      edit: (text) => mustReplace(text, /^default = \[(.*)\]$/m, (m) => m.replace(/\]$/, ', "tinfoil-enclave"]')),
      expect: "tinfoil",
    },
  ];
  for (const scenario of reEnabled) {
    test(
      scenario.name,
      () => {
        const overlay = overlayWorkspace(scenario.edit);
        try {
          const pkgs = cargoTree(
            {
              crate: "engine",
              origin: "overlay",
              target: null,
              features: [],
              noDefault: false,
              allFeatures: false,
            },
            overlay.root,
          );
          const violations = licenceViolations(pkgs, overlay.root);
          expect(violations.map((v) => v.pkg.split(" ")[0])).toContain(scenario.expect);
          // Nothing else in the default graph is affected by the edit.
          expect(violations.every((v) => Object.keys(OPT_IN_ONLY).includes(v.pkg.split(" ")[0]))).toBe(true);
        } finally {
          overlay.cleanup();
        }
      },
      CARGO_TIMEOUT_MS,
    );
  }
});
