// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

/**
 * Vendor-host guard.
 *
 * Scans the code that ships (Rust crates, the desktop app, packages, CI
 * workflows, scripts) for hosts and keys of the upstream vendor's services:
 * its website and API, its analytics and crash-reporting projects, and its
 * hosted privacy enclave. Comments and test code are skipped, so the guard
 * only reports places that could actually build a request.
 *
 * It also guards the product's identity (W0-12): the Tauri configs must name
 * Freenpipe, use its own bundle identifier and register only `freenpipe`
 * deep-link schemes, and the desktop UI's copy must not call the product
 * "screenpipe". Interop names keep the upstream spelling and are not
 * reported: crate and package names, `SCREENPIPE_*` variables,
 * `~/.screenpipe` paths, the `screenpipe` CLI, skill names, quoted ids.
 *
 * Modes (env `VENDOR_GUARD_MODE`):
 *   - `strict` (default since W0-12): fail when any hit remains.
 *   - `report`: print every remaining hit and pass. Only for local triage.
 *
 * The endpoint switchboard, commerce and identity checks fail in every mode.
 *
 * Run from the repository root:  bun test scripts/ci/vendor-guard.test.ts
 */

import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { extname, join, relative, sep } from "node:path";

const REPO_ROOT = join(import.meta.dir, "..", "..");

type Rule = { id: string; pattern: RegExp };

/** What counts as a vendor reference. */
export const RULES: Rule[] = [
  { id: "screenpi.pe", pattern: /screenpi\.pe\b/i },
  { id: "screenpipe.com", pattern: /screenpipe\.com\b/i },
  { id: "posthog.com", pattern: /posthog\.com\b/i },
  { id: "posthog-key", pattern: /\bphc_[A-Za-z0-9]{20,}/ },
  { id: "sentry.io", pattern: /sentry\.io\b/i },
  { id: "tinfoil", pattern: /tinfoil\.(dev|sh)\b/i },
  { id: "mediar.ai", pattern: /mediar\.ai\b/i },
  // The vendor's company name in shipped code (installer publisher, about
  // text, copy). Case-sensitive so `EvictMediaRequest` and friends pass.
  { id: "vendor-company", pattern: /\bMediar\b/ },
];

/**
 * Paths left out of every scan. A path ending in "/" covers everything below
 * it. Keep each entry narrow and give its reason.
 */
const EXCLUDED: { path: string; reason: string }[] = [
  {
    path: "packages/ai-gateway/",
    reason:
      "upstream's hosted gateway worker, kept as MIT reference source; not built or shipped (W0-06 open question 11)",
  },
  {
    path: "third_party/",
    reason: "vendored third-party sources (W0-13) legitimately name their own authors and hosts",
  },
];

function isExcluded(repoPath: string): boolean {
  return EXCLUDED.some((e) => repoPath === e.path || repoPath.startsWith(e.path));
}

/**
 * Hits that are legitimate: third-party integrations the user connects with
 * their own account (the Sentry and PostHog connectors talk to the user's
 * own projects, not the vendor's).
 */
const ALLOWED: { path: string; rules: string[]; reason: string }[] = [
  {
    path: "crates/screenpipe-connect/src/connections/sentry.rs",
    rules: ["sentry.io"],
    reason: "user-owned Sentry connector",
  },
  {
    path: "crates/screenpipe-connect/src/connections/posthog.rs",
    rules: ["posthog.com"],
    reason: "user-owned PostHog connector",
  },
  {
    path: "crates/screenpipe-redact/examples/pii_eval.rs",
    rules: ["sentry.io"],
    reason: "synthetic Sentry-DSN-shaped secret for the redaction eval; never requested",
  },
];

/**
 * Commerce and promotion surfaces: checkout and billing, referral, the hosted
 * team and enterprise consoles, sales booking links, the vendor's community
 * invite, plan and upgrade rows, and the vendor's later trial, paywall,
 * card-ask, annual-plan, announcement, remote-control and hosted-quota
 * features.
 *
 * W0-05 removed every one of these that the MIT base had. The target-only
 * ones are never ported: Freenpipe has nothing to sell, so they have no
 * counterpart. Unlike RULES, these fail in every mode. Code that has to name
 * one of them to recognise and ignore it (store compatibility, deep-link
 * filtering) goes in COMMERCE_ALLOWED with its reason.
 */
export const COMMERCE_RULES: Rule[] = [
  // Removed from the base app by W0-05.
  { id: "checkout-endpoint", pattern: /\/api\/(subscription|cloud-sync)\/checkout(?![\w-])/ },
  { id: "referral-endpoint", pattern: /\/api\/referral(?![\w-])/ },
  { id: "team-endpoint", pattern: /\/api\/team(?![\w-])/ },
  { id: "enterprise-endpoint", pattern: /\/api\/enterprise\/(policy|heartbeat|v1)(?![\w-])/ },
  { id: "booking-link", pattern: /cal\.com\/team\//i },
  { id: "community-invite", pattern: /discord\.(com\/invite|gg)\/screenpipe\b/i },
  { id: "purchase-deep-link", pattern: /purchase-successful|stripe-connect/ },
  { id: "tray-upgrade", pattern: /tray-upgrade|upgrade to business/i },
  // Target-only (v2.7.x); never ported.
  { id: "trial-paywall", pattern: /trial-activation|\/onboarding\/checkout/ },
  { id: "card-ask", pattern: /card-ask|business-trial|FORCE_BUSINESS_TRIAL/i },
  { id: "upgrade-offer", pattern: /\/api\/subscription\/(offer|change-plan)|\/api\/billing(?![\w-])|\/api\/team\/billing/ },
  { id: "annual-plan-banner", pattern: /annual-plan-banner|plan-expiration-notice/ },
  { id: "announcements", pattern: /screenpipe-announcement|announcements-dismissed/ },
  { id: "remote-control-policy", pattern: /remoteControlPolicy|desktop_remote_control_applied/ },
  {
    id: "hosted-quota",
    pattern:
      /free_pipe_limit_reached|free_chat_limit_exceeded|hosted_ai_allowance_exceeded|trial_cost_limit_exceeded|activitiesQuotaPause|background_pipe_advisory/,
  },
];

/**
 * Legitimate COMMERCE_RULES hits. A path ending in "/" covers everything
 * below it.
 */
const COMMERCE_ALLOWED: { path: string; rules: string[]; reason: string }[] = [
  {
    path: "apps/screenpipe-app-tauri/src-tauri/src/store.rs",
    rules: ["trial-paywall"],
    reason: "reads trial onboarding steps that screenpipe stored and treats them as complete",
  },
  {
    path: "apps/screenpipe-app-tauri/lib/account-deep-link.ts",
    rules: ["purchase-deep-link"],
    reason: "recognises vendor purchase deep links so they can be dropped",
  },
  {
    path: "packages/ai-gateway/",
    rules: ["upgrade-offer"],
    reason: "the vendor's hosted gateway source; not built or shipped (W0-06)",
  },
];

/**
 * Product name in the desktop UI's copy (W0-12). Only the app's pages and
 * components are scanned; their strings and JSX text are what people read.
 */
const BRAND_COPY_ROOTS = ["apps/screenpipe-app-tauri/app/", "apps/screenpipe-app-tauri/components/"];
const BRAND_COPY_EXTS = new Set([".ts", ".tsx"]);

/** Words after the name that make it an interop name, not the product. */
const INTEROP_FOLLOWERS = /^ (pipe|skill|cloud)\b/;

/**
 * Legitimate brand-copy hits: `contains` must appear on the line. Keep these
 * to interop text with a reason.
 */
const BRAND_ALLOWED: { path: string; contains: string; reason: string }[] = [
  {
    path: "apps/screenpipe-app-tauri/components/rewind/ai-presets-selector.tsx",
    contains: "because screenpipe paths often contain",
    reason:
      "default preset prompt; must stay identical to the copies in store.rs and use-settings.tsx so stored presets keep matching it",
  },
];

/**
 * Columns where a line calls the product "screenpipe": a standalone word
 * that is not part of an identifier, path, URL, file name or quoted id, not
 * a backticked command name and not followed by an interop word.
 */
export function brandCopyHits(line: string): number[] {
  if (/\bconsole\.(log|info|warn|error|debug)\(/.test(line)) return [];
  const found: number[] = [];
  const re = /screenpipe/gi;
  let m: RegExpExecArray | null;
  while ((m = re.exec(line)) !== null) {
    const start = m.index;
    const end = start + m[0].length;
    const prev = line[start - 1] ?? "";
    const next = line[end] ?? "";
    const after = line[end + 1] ?? "";
    if (/[-_./@~:$=\\A-Za-z0-9]/.test(prev)) continue; // ids, paths, ?name=screenpipe
    if (/[-_/:&\\A-Za-z0-9]/.test(next)) continue;
    if (next === "." && /[A-Za-z0-9]/.test(after)) continue; // screenpipe.exe, .md
    if (/["'`]/.test(prev) && prev === next) continue; // "screenpipe" id, `screenpipe`
    if (INTEROP_FOLLOWERS.test(line.slice(end))) continue;
    found.push(start);
  }
  return found;
}

function scanBrandCopy(): { hits: Hit[]; files: number } {
  const hits: Hit[] = [];
  let files = 0;
  for (const root of BRAND_COPY_ROOTS) {
    for (const full of walk(join(REPO_ROOT, root))) {
      const ext = extname(full).toLowerCase();
      if (!BRAND_COPY_EXTS.has(ext)) continue;
      const file = toRepoPath(full);
      if (isExcluded(file)) continue;
      files++;
      let source: string;
      try {
        source = readFileSync(full, "utf8");
      } catch {
        continue;
      }
      stripComments(source, ext).forEach((text, i) => {
        if (brandCopyHits(text).length === 0) return;
        if (BRAND_ALLOWED.some((a) => a.path === file && text.includes(a.contains))) return;
        hits.push({ file, line: i + 1, rule: "product-name", text: text.trim().slice(0, 160) });
      });
    }
  }
  return { hits, files };
}

/** Bundle identity every Tauri config must carry. */
const TAURI_DIR = "apps/screenpipe-app-tauri/src-tauri";

export function identityProblems(file: string, conf: any): string[] {
  const problems: string[] = [];
  if ("productName" in conf && !/^Freenpipe( |$)/.test(String(conf.productName))) {
    problems.push(`${file}: productName "${conf.productName}" is not Freenpipe`);
  }
  if ("identifier" in conf && !/^com\.normiecore\.freenpipe(\.[a-z]+)?$/.test(String(conf.identifier))) {
    problems.push(`${file}: identifier "${conf.identifier}" is not com.normiecore.freenpipe[.channel]`);
  }
  const publisher = conf.bundle?.publisher;
  if (publisher !== undefined && /mediar|screenpipe/i.test(String(publisher))) {
    problems.push(`${file}: publisher "${publisher}" names the upstream vendor`);
  }
  const deepLink = conf.plugins?.["deep-link"];
  const desktop = deepLink?.desktop?.schemes ?? deepLink?.desktop?.scheme;
  const schemes: string[] = desktop === undefined ? [] : [desktop].flat();
  for (const scheme of schemes) {
    if (!/^freenpipe(-[a-z]+)?$/.test(String(scheme))) {
      problems.push(`${file}: registers deep-link scheme "${scheme}" (only freenpipe schemes may be claimed)`);
    }
  }
  for (const entry of deepLink?.mobile ?? []) {
    const host = String(entry?.host ?? "");
    if (RULES.some((r) => r.pattern.test(host))) {
      problems.push(`${file}: mobile deep-link host "${host}" is a vendor host`);
    }
  }
  return problems;
}

/** Files that make up the endpoint switchboard; these must stay clean. */
const SWITCHBOARD_FILES = [
  "crates/screenpipe-core/src/endpoints.rs",
  "apps/screenpipe-app-tauri/lib/web-url.ts",
  "apps/screenpipe-app-tauri/src-tauri/capabilities/main.json",
];

const SCAN_ROOTS = ["crates", "apps", "packages", ".github/workflows", "scripts"];

const SKIP_DIRS = new Set([
  "node_modules",
  "target",
  ".next",
  "out",
  "dist",
  "build",
  "gen",
  ".turbo",
  "coverage",
  "__tests__",
  "tests",
  "test",
  "e2e",
  ".e2e",
  "fixtures",
  "docs",
]);

const SLASH_COMMENT = new Set([".rs", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".swift"]);
const HASH_COMMENT = new Set([".toml", ".yml", ".yaml", ".sh", ".py", ".ps1"]);
const PLAIN = new Set([".json"]);
const SKIP_FILES = new Set(["package-lock.json", "bun.lock", "Cargo.lock", "tsconfig.tsbuildinfo"]);

export type Hit = { file: string; line: number; rule: string; text: string };

function isTestFile(name: string): boolean {
  return /\.(test|spec)\.[cm]?[jt]sx?$/.test(name) || /_test\.rs$/.test(name);
}

function* walk(dir: string): Generator<string> {
  let entries: string[];
  try {
    entries = readdirSync(dir);
  } catch {
    return;
  }
  for (const name of entries) {
    const full = join(dir, name);
    let st;
    try {
      st = statSync(full);
    } catch {
      continue;
    }
    if (st.isDirectory()) {
      if (!SKIP_DIRS.has(name)) yield* walk(full);
    } else if (st.isFile() && !SKIP_FILES.has(name) && !isTestFile(name)) {
      yield full;
    }
  }
}

/**
 * Blank out comments line by line so line numbers stay stable. Strings that
 * contain comment markers (`"https://..."`) survive because a `//` only
 * starts a comment at the line start or after whitespace.
 */
export function stripComments(source: string, ext: string): string[] {
  const lines = source.split(/\r?\n/);
  if (PLAIN.has(ext)) return lines;
  if (HASH_COMMENT.has(ext)) return lines.map((l) => l.replace(/(^|\s)#.*$/, "$1"));

  const out: string[] = [];
  let inBlock = false;
  for (const line of lines) {
    let rest = line;
    let kept = "";
    while (rest.length > 0) {
      if (inBlock) {
        const end = rest.indexOf("*/");
        if (end < 0) {
          rest = "";
        } else {
          rest = rest.slice(end + 2);
          inBlock = false;
        }
        continue;
      }
      // `{/* … */}` (JSX) and `(/* … */` also open comments.
      const start = rest.search(/(^|[\s{(,;])\/\*/);
      const lineComment = rest.search(/(^|\s)\/\//);
      if (lineComment >= 0 && (start < 0 || lineComment < start)) {
        kept += rest.slice(0, lineComment);
        rest = "";
      } else if (start >= 0) {
        kept += rest.slice(0, start);
        rest = rest.slice(rest.indexOf("/*", start) + 2);
        inBlock = true;
      } else {
        kept += rest;
        rest = "";
      }
    }
    out.push(kept);
  }
  return out;
}

/**
 * Blank the bodies of `#[cfg(test)] mod … { … }` blocks (other attributes
 * may sit between the cfg and the `mod`). Brace counting is naive, which is
 * fine on comment-stripped lines of ordinary test modules.
 */
export function blankRustTestModules(lines: string[]): string[] {
  const out = [...lines];
  let i = 0;
  while (i < out.length) {
    if (out[i].trim() !== "#[cfg(test)]") {
      i++;
      continue;
    }
    let j = i + 1;
    while (j < out.length && /^\s*#\[/.test(out[j])) j++;
    if (j >= out.length || !/^\s*(pub(\([^)]*\))?\s+)?mod\s+\w+/.test(out[j])) {
      i++;
      continue;
    }
    let depth = 0;
    let opened = false;
    let k = i;
    for (; k < out.length; k++) {
      const line = out[k];
      if (k >= j) {
        for (const ch of line) {
          if (ch === "{") {
            depth++;
            opened = true;
          } else if (ch === "}") {
            depth--;
          }
        }
      }
      out[k] = "";
      if (k >= j && (line.trimEnd().endsWith(";") && !opened)) break; // `mod tests;`
      if (opened && depth <= 0) break;
    }
    i = k + 1;
  }
  return out;
}

export function scanSource(file: string, source: string, rules: Rule[] = RULES): Hit[] {
  const ext = extname(file).toLowerCase();
  if (!SLASH_COMMENT.has(ext) && !HASH_COMMENT.has(ext) && !PLAIN.has(ext)) return [];
  let lines = stripComments(source, ext);
  if (ext === ".rs") lines = blankRustTestModules(lines);
  const hits: Hit[] = [];
  lines.forEach((text, i) => {
    for (const rule of rules) {
      if (rule.pattern.test(text)) {
        hits.push({ file, line: i + 1, rule: rule.id, text: text.trim().slice(0, 160) });
      }
    }
  });
  return hits;
}

function isAllowed(hit: Hit): boolean {
  return ALLOWED.some((a) => hit.file === a.path && a.rules.includes(hit.rule));
}

function isCommerceAllowed(hit: Hit): boolean {
  return COMMERCE_ALLOWED.some(
    (a) =>
      (a.path.endsWith("/") ? hit.file.startsWith(a.path) : hit.file === a.path) &&
      a.rules.includes(hit.rule),
  );
}

function toRepoPath(full: string): string {
  return relative(REPO_ROOT, full).split(sep).join("/");
}

export function scanRepo(
  rules: Rule[] = RULES,
  allow: (hit: Hit) => boolean = isAllowed,
): { hits: Hit[]; allowed: Hit[]; files: number } {
  const hits: Hit[] = [];
  const allowed: Hit[] = [];
  let files = 0;
  for (const root of SCAN_ROOTS) {
    for (const full of walk(join(REPO_ROOT, root))) {
      const repoPath = toRepoPath(full);
      if (isExcluded(repoPath)) continue;
      const ext = extname(full).toLowerCase();
      if (!SLASH_COMMENT.has(ext) && !HASH_COMMENT.has(ext) && !PLAIN.has(ext)) continue;
      files++;
      let source: string;
      try {
        source = readFileSync(full, "utf8");
      } catch {
        continue;
      }
      for (const hit of scanSource(repoPath, source, rules)) {
        (allow(hit) ? allowed : hits).push(hit);
      }
    }
  }
  return { hits, allowed, files };
}

function printReport(hits: Hit[], allowed: Hit[], files: number): void {
  const byFile = new Map<string, Hit[]>();
  for (const h of hits) byFile.set(h.file, [...(byFile.get(h.file) ?? []), h]);
  const byRule = new Map<string, number>();
  for (const h of hits) byRule.set(h.rule, (byRule.get(h.rule) ?? 0) + 1);

  const lines: string[] = [];
  lines.push(`vendor-guard: scanned ${files} files`);
  lines.push(
    `vendor-guard: ${hits.length} remaining hit(s) in ${byFile.size} file(s); ${allowed.length} allowlisted`,
  );
  for (const [rule, n] of [...byRule].sort((a, b) => b[1] - a[1])) {
    lines.push(`  ${rule.padEnd(16)} ${n}`);
  }
  for (const [file, fileHits] of [...byFile].sort((a, b) => b[1].length - a[1].length)) {
    lines.push(`${file} (${fileHits.length})`);
    for (const h of fileHits) lines.push(`  ${h.line}: [${h.rule}] ${h.text}`);
  }
  console.log(lines.join("\n"));
}

const MODE = (process.env.VENDOR_GUARD_MODE ?? "strict").toLowerCase();

describe("vendor-host guard", () => {
  test("comment stripping keeps URLs inside strings", () => {
    const lines = stripComments(
      [
        'let a = "https://example.test/x"; // see screenpi.pe',
        "/* screenpipe.com",
        "   still comment */ let b = 1;",
        "// sentry.io",
      ].join("\n"),
      ".rs",
    );
    expect(lines[0]).toContain("https://example.test/x");
    expect(lines.join("\n")).not.toMatch(/screenpi\.pe|screenpipe\.com|sentry\.io/);
    expect(lines[2]).toContain("let b = 1;");
  });

  test("rust test modules are skipped, code after them is not", () => {
    const src = [
      'const A: &str = "https://screenpi.pe/a";',
      "#[cfg(test)]",
      "#[allow(dead_code)]",
      "mod tests {",
      "    fn f() { let _ = \"x@screenpi.pe\"; }",
      "}",
      'const B: &str = "https://screenpipe.com/b";',
    ].join("\n");
    const hits = scanSource("x.rs", src);
    expect(hits.map((h) => [h.line, h.rule])).toEqual([
      [1, "screenpi.pe"],
      [7, "screenpipe.com"],
    ]);
  });

  test("rules catch request-building code", () => {
    const hits = scanSource("x.ts", 'fetch("https://api.screenpipe.com/v1/models")\n');
    expect(hits.map((h) => h.rule)).toEqual(["screenpipe.com"]);
  });

  test("endpoint switchboard files contain no vendor hosts", () => {
    const offenders: Hit[] = [];
    for (const rel of SWITCHBOARD_FILES) {
      const source = readFileSync(join(REPO_ROOT, rel), "utf8");
      offenders.push(...scanSource(rel, source));
    }
    expect(offenders).toEqual([]);
  });

  test("commerce rules catch removed and never-ported surfaces", () => {
    const sample = [
      'fetch("https://example.test/api/subscription/checkout")',
      'fetch(`${web}/api/cloud-sync/checkout`)',
      'fetch(base + "/api/referral/invite")',
      'const API = base + "/api/team";',
      'openUrl("https://cal.com/team/acme/chat")',
      'openUrl("https://discord.com/invite/screenpipe")',
      'let _ = app.emit("tray-upgrade", ());',
      'step: "trial-activation-v1-paywall"',
      'localStorage.getItem("screenpipe-announcements-dismissed-v1")',
      'if (settings.remoteControlPolicy) {}',
      'if (code === "free_pipe_limit_reached") {}',
    ].join("\n");
    const hit = scanSource("x.ts", sample, COMMERCE_RULES).map((h) => h.line);
    expect(hit).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);

    const innocent = [
      "git checkout main",
      'fetch("/api/teams-meeting/status")',
      'const plan = "upgrade the parser";',
      'route("/api/billing-address-autofill")',
      'const help = "https://discord.com/channels/@me/123";',
    ].join("\n");
    expect(scanSource("x.ts", innocent, COMMERCE_RULES)).toEqual([]);
  });

  test("no commerce, team or promotion surfaces remain (every mode)", () => {
    const { hits, files } = scanRepo(COMMERCE_RULES, isCommerceAllowed);
    if (hits.length > 0) {
      console.log(
        ["vendor-guard: commerce surfaces found (allowlist only with a reason):"]
          .concat(hits.map((h) => `  ${h.file}:${h.line} [${h.rule}] ${h.text}`))
          .join("\n"),
      );
    }
    expect(files).toBeGreaterThan(0);
    expect(hits).toEqual([]);
  });

  test("brand-copy matcher separates the product name from interop names", () => {
    const flagged = [
      '"screenpipe will launch when your computer starts"',
      "Lets Screenpipe index what's on your screen",
      "        same machine as screenpipe.",
      "<strong>screenpipe</strong> on in System Settings",
      "## what is screenpipe?",
      '"failed to restart screenpipe"',
    ];
    for (const line of flagged) expect(brandCopyHits(line).length, line).toBeGreaterThan(0);
    const fine = [
      'import { x } from "@/lib/screenpipe-skills";',
      'if (citation.kind === "screenpipe") {',
      "install: screenpipe pipe install ~/.screenpipe/pipes/my-pipe",
      "in your shell it is the \\`screenpipe\\` command ($SCREENPIPE_CLI)",
      'return "screenpipe cloud";',
      "first read the screenpipe skill file",
      'const p = "%LOCALAPPDATA%\\\\screenpipe\\\\screenpipe.exe";',
      "lmstudio://add_mcp?name=screenpipe&config=x",
      'localStorage.getItem("screenpipe:pipes-welcome-dismissed")',
      'console.error("failed to open screenpipe link", e);',
      "Freenpipe is recording",
    ];
    for (const line of fine) expect(brandCopyHits(line), line).toEqual([]);
  });

  test("desktop UI copy names Freenpipe, not screenpipe (every mode)", () => {
    const { hits, files } = scanBrandCopy();
    if (hits.length > 0) {
      console.log(
        ["vendor-guard: UI copy still calls the product screenpipe:"]
          .concat(hits.map((h) => `  ${h.file}:${h.line} ${h.text}`))
          .join("\n"),
      );
    }
    expect(files).toBeGreaterThan(0);
    expect(hits).toEqual([]);
  });

  test("identity checks reject upstream identity", () => {
    const upstream = {
      productName: "screenpipe",
      identifier: "screenpi.pe",
      bundle: { publisher: "Mediar, Inc." },
      plugins: {
        "deep-link": {
          desktop: { schemes: ["screenpipe"] },
          mobile: [{ host: "screenpi.pe" }],
        },
      },
    };
    expect(identityProblems("x.json", upstream)).toHaveLength(5);
    const ours = {
      productName: "Freenpipe Beta",
      identifier: "com.normiecore.freenpipe.beta",
      bundle: { publisher: "normiecore" },
      plugins: { "deep-link": { desktop: { schemes: ["freenpipe-beta"] } } },
    };
    expect(identityProblems("x.json", ours)).toEqual([]);
  });

  test("Tauri configs carry the Freenpipe identity (every mode)", () => {
    const dir = join(REPO_ROOT, TAURI_DIR);
    const files = readdirSync(dir).filter((f) => /^tauri(\..+)?\.(conf\.)?json$/.test(f));
    expect(files).toContain("tauri.conf.json");
    const problems = files.flatMap((f) =>
      identityProblems(`${TAURI_DIR}/${f}`, JSON.parse(readFileSync(join(dir, f), "utf8"))),
    );
    expect(problems).toEqual([]);
    // The release config must actually set the name and identifier.
    const prod = JSON.parse(readFileSync(join(dir, "tauri.prod.conf.json"), "utf8"));
    expect(prod.productName).toBe("Freenpipe");
    expect(prod.identifier).toBe("com.normiecore.freenpipe");
  });

  test(`repository scan (${MODE} mode)`, () => {
    const { hits, allowed, files } = scanRepo();
    printReport(hits, allowed, files);
    expect(files).toBeGreaterThan(0);
    if (MODE === "strict") {
      expect(hits).toEqual([]);
    }
  });
});
