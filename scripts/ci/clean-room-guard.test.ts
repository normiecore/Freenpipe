// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

/**
 * Clean-room guard (owner decision 8).
 *
 * 1. Evidence pointers stay out of the repository. Every tracked text file
 *    except the pre-freeze specs (docs/clean-room/legacy-specs/) and the
 *    generated plan must not name REA evidence ids, the private analysis
 *    workspace, extracted target files or the frontend chunks pulled from the
 *    shipped binary. Comments count too: a pointer in a comment shows the
 *    author read the evidence.
 * 2. Re-sourced specs (docs/clean-room/specs/) are held to more: no hex
 *    offsets, and every delta item carries a source tag (SOURCING.md).
 * 3. Work-package branches carry a filled-in provenance record. When
 *    `FP_BRANCH` is `wp/<WP-ID>`, `docs/clean-room/provenance/<WP-ID>.md` must
 *    exist, name a real work package and have no template placeholders or
 *    unticked checks left.
 *
 * Hits are reported as file, line and rule only, so the CI log never repeats
 * the pointer it caught.
 *
 * Run from the repository root:  bun test scripts/ci/clean-room-guard.test.ts
 */

import { describe, expect, test } from "bun:test";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

const REPO_ROOT = join(import.meta.dir, "..", "..");

type Rule = { id: string; pattern: RegExp };

/** What counts as an evidence pointer, everywhere. */
export const RULES: Rule[] = [
  { id: "rea-evidence-id", pattern: /\bev_[0-9a-f]{64}\b/ },
  { id: "workspace-placeholder", pattern: /<re-workspace>|<screenpipe-install>/ },
  {
    id: "workspace-path",
    pattern: /(?:^|[^A-Za-z0-9_])(?:[A-Za-z]:[\\/]sp|(?:\/mnt)?\/+c\/sp)(?=$|[\\/\s"'`.,;:)\]>*])/i,
  },
  { id: "extracted-strings", pattern: /\bapp_strings\.tsv\b/ },
  { id: "extracted-frontend-chunk", pattern: /\bchunk_[0-9a-f]{4,}-[0-9a-f]{16}\.js\b/ },
];

/** Extra rules for the re-sourced specs. */
export const SPEC_RULES: Rule[] = [{ id: "hex-offset", pattern: /\b0x[0-9a-f]{6,8}\b/i }];

const SPECS_DIR = "docs/clean-room/specs/";

/**
 * Paths left out of the scan. A path ending in "/" covers everything below
 * it. Keep each entry narrow and give its reason.
 */
const EXCLUDED: { path: string; reason: string }[] = [
  {
    path: "docs/clean-room/legacy-specs/",
    reason: "pre-freeze specs, kept unchanged as the record of that work (SOURCING.md)",
  },
  { path: "docs/clean-room/PLAN.md", reason: "generated planning snapshot from before the freeze" },
  { path: "docs/clean-room/plan_wps.json", reason: "generated planning data from before the freeze" },
  { path: "tools/plan/", reason: "plan generator and its templates from before the freeze" },
  { path: "scripts/ci/clean-room-guard.test.ts", reason: "this guard names the patterns it looks for" },
];

/**
 * Lines that name a pointer to define or forbid it. A hit is allowed only
 * when its file and rule match and the line contains `token`.
 */
const ALLOWED: { path: string; rule: string; token: string; reason: string }[] = [
  {
    path: "docs/clean-room/README.md",
    rule: "workspace-placeholder",
    token: "is the private analysis workspace",
    reason: "defines the placeholders used by the pre-freeze specs",
  },
  {
    path: "docs/clean-room/README.md",
    rule: "workspace-placeholder",
    token: "is the installed target",
    reason: "defines the placeholders used by the pre-freeze specs",
  },
  {
    path: "docs/clean-room/DECISIONS.md",
    rule: "extracted-strings",
    token: "is NEVER published",
    reason: "decision 4 names the extracted files that are never published",
  },
  {
    path: "CLAUDE.md",
    rule: "workspace-path",
    token: "never read the private analysis workspace",
    reason: "names the workspace to forbid it",
  },
  {
    path: ".claude/settings.json",
    rule: "workspace-path",
    token: "Read(",
    reason: "deny rules for the workspace",
  },
  {
    path: ".claude/settings.json",
    rule: "extracted-strings",
    token: "Read(",
    reason: "deny rule for the extracted string table",
  },
  {
    path: ".claude/settings.json",
    rule: "workspace-path",
    token: "Edit(",
    reason: "deny rules for the workspace",
  },
];

/**
 * Work-package branches opened before the freeze. They were built from the
 * pre-freeze specs under the rules then in force, and the coordinator states
 * that in their PR instead of a provenance record.
 */
const GRANDFATHERED_WPS: { id: string; reason: string }[] = [
  { id: "W4-02", reason: "PR open on 2026-10-29, before decision 8" },
  { id: "W4-05", reason: "PR open on 2026-10-29, before decision 8" },
];

export type Hit = { file: string; line: number; rule: string; text: string };

function isExcluded(path: string): boolean {
  return EXCLUDED.some((e) => path === e.path || (e.path.endsWith("/") && path.startsWith(e.path)));
}

function isAllowed(hit: Hit): boolean {
  return ALLOWED.some((a) => a.path === hit.file && a.rule === hit.rule && hit.text.includes(a.token));
}

export function scanText(file: string, source: string, rules: Rule[] = RULES): Hit[] {
  const hits: Hit[] = [];
  source.split(/\r?\n/).forEach((text, i) => {
    for (const rule of rules) {
      if (rule.pattern.test(text)) hits.push({ file, line: i + 1, rule: rule.id, text });
    }
  });
  return hits;
}

const DELTA_ITEM = /^\s*-\s+([a-z][a-z0-9_]*-[a-z0-9]*\d+[a-z]?):/;
const SOURCE_TAG = /\[(own|mit(: [^\]]+)?|interop|public: [^\]]+)\]\s*$/;
const PENDING = /\(behaviour pending\)\s*$/;

/** Delta items in a re-sourced spec that have no source tag. */
export function untaggedItems(file: string, source: string): Hit[] {
  const hits: Hit[] = [];
  source.split(/\r?\n/).forEach((text, i) => {
    if (DELTA_ITEM.test(text) && !SOURCE_TAG.test(text) && !PENDING.test(text)) {
      hits.push({ file, line: i + 1, rule: "untagged-item", text });
    }
  });
  return hits;
}

function trackedFiles(): string[] {
  const out = execFileSync("git", ["ls-files", "-z"], { cwd: REPO_ROOT, maxBuffer: 64 * 1024 * 1024 });
  return out.toString("utf8").split("\0").filter(Boolean);
}

function readText(path: string): string | null {
  const full = join(REPO_ROOT, path);
  if (!existsSync(full)) return null; // deleted in the working tree
  const buf = readFileSync(full);
  if (buf.subarray(0, 8192).includes(0)) return null; // binary
  return buf.toString("utf8");
}

export function scanRepo(files: string[] = trackedFiles()): { hits: Hit[]; allowed: Hit[]; files: number } {
  const all: Hit[] = [];
  let scanned = 0;
  for (const file of files) {
    if (isExcluded(file)) continue;
    const text = readText(file);
    if (text === null) continue;
    scanned++;
    all.push(...scanText(file, text));
    if (file.startsWith(SPECS_DIR) && file.endsWith(".md") && !file.endsWith("/README.md")) {
      all.push(...scanText(file, text, SPEC_RULES), ...untaggedItems(file, text));
    }
  }
  return { hits: all.filter((h) => !isAllowed(h)), allowed: all.filter(isAllowed), files: scanned };
}

/** `wp/W3-02` -> `W3-02`; another `wp/` name -> "invalid"; anything else -> null. */
export function wpIdFromBranch(branch: string | undefined): string | null {
  if (!branch?.startsWith("wp/")) return null;
  const m = /^wp\/((?:W[0-9]|WR)-\d{2})$/.exec(branch);
  return m ? m[1] : "invalid";
}

/** Problems with a provenance record's text. */
export function recordProblems(text: string): string[] {
  const problems: string[] = [];
  if (/<[A-Za-z][^>\n]*>/.test(text.replace(/`[^`\n]*`/g, ""))) problems.push("template placeholders left");
  if (/^\s*- \[ \]/m.test(text)) problems.push("unticked checks");
  if (!/^## Spec items implemented/m.test(text)) problems.push("missing 'Spec items implemented' section");
  return problems;
}

function knownWpIds(): Set<string> {
  const plan = JSON.parse(readFileSync(join(REPO_ROOT, "docs/clean-room/plan_wps.json"), "utf8"));
  return new Set(plan.wps.map((w: { id: string }) => w.id));
}

describe("clean-room guard", () => {
  test("rules catch evidence pointers and leave ordinary code alone", () => {
    const bad = [
      `// see ev_${"a1".repeat(32)}`,
      "source <re-workspace>/tools/devenv.sh",
      'let p = "C:\\sp\\chunk.js";',
      "never read `C:\\sp`.",
      "Read(//c/sp/**)",
      "ls /mnt/c/sp/x",
      "grep foo app_strings.tsv",
      "chunk_1002-fedb2c94a40a9597.js",
    ];
    for (const line of bad) expect(scanText("x", line).length).toBeGreaterThan(0);
    const good = ["let prev_state = 0x5a42a38;", "fn ev_parse() {}", "C:\\spool\\x", "let sp = 1;", "chunk_size = 4", "/c/spark/x"];
    for (const line of good) expect(scanText("x", line)).toEqual([]);
  });

  test("re-sourced specs need source tags and no hex offsets", () => {
    const spec = [
      "- api-031: `GET /meetings/{id}` returns `meeting_id`. [interop]",
      "- audio-012b: echo cancellation per device. [public: https://example.org, read 2026-11-02]",
      "- db-008: retention runs every 6 hours. [own]",
      "- vision-004: (behaviour pending)",
      "- shell-010: something unsourced.",
    ].join("\n");
    expect(untaggedItems("s", spec).map((h) => h.line)).toEqual([5]);
    expect(scanText("s", "found at 0x5a42a38", SPEC_RULES).length).toBe(1);
  });

  test("branch names map to work-package ids", () => {
    expect(wpIdFromBranch("wp/W3-02")).toBe("W3-02");
    expect(wpIdFromBranch("wp/WR-01")).toBe("WR-01");
    expect(wpIdFromBranch("wp/w3-02")).toBe("invalid");
    expect(wpIdFromBranch("wp/W3-02-retry")).toBe("invalid");
    expect(wpIdFromBranch("main")).toBeNull();
    expect(wpIdFromBranch("fix/main-abc1234")).toBeNull();
  });

  test("provenance records must be filled in", () => {
    const template = readFileSync(join(REPO_ROOT, "docs/clean-room/provenance/TEMPLATE.md"), "utf8");
    expect(recordProblems(template).length).toBeGreaterThan(0);
    expect(recordProblems("# W3-02 provenance\n\n## Spec items implemented\n\n- [x] checked\n")).toEqual([]);
  });

  test("no evidence pointers outside the pre-freeze specs", () => {
    const { hits, allowed, files } = scanRepo();
    console.log(`clean-room-guard: ${files} files scanned, ${hits.length} hits, ${allowed.length} allowed`);
    for (const h of hits) console.log(`  ${h.file}:${h.line} [${h.rule}]`);
    expect(hits.length).toBe(0);
  });

  test("a work-package branch carries its provenance record", () => {
    const branch = process.env.FP_BRANCH;
    const wp = wpIdFromBranch(branch);
    if (wp === null) return;
    expect(wp, `branch ${branch} is not wp/<WP-ID>`).not.toBe("invalid");
    expect(knownWpIds().has(wp), `${wp} is not in plan_wps.json`).toBe(true);
    if (GRANDFATHERED_WPS.some((g) => g.id === wp)) return;
    const record = join(REPO_ROOT, `docs/clean-room/provenance/${wp}.md`);
    expect(existsSync(record), `missing docs/clean-room/provenance/${wp}.md`).toBe(true);
    expect(recordProblems(readFileSync(record, "utf8"))).toEqual([]);
  });
});
