#!/usr/bin/env bun
// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

/**
 * Build a Tauri updater manifest (`latest.json`, static format) from the
 * `.sig` files that `tauri build` writes next to each updater artifact when
 * TAURI_SIGNING_PRIVATE_KEY is set.
 *
 *   bun scripts/release/make-latest-json.ts \
 *     --version 1.2.3 \
 *     --base-url https://github.com/normiecore/Freenpipe/releases/download/v1.2.3 \
 *     --bundle x86_64-pc-windows-msvc=path/to/bundle \
 *     [--bundle <rust target>=<bundle dir> ...] \
 *     [--artifact <platform key>=<artifact file> ...] \
 *     [--notes "text" | --notes-file notes.md] [--pub-date 2026-01-01T00:00:00Z] \
 *     [--stage dist/] [--out latest.json]
 *
 * `--bundle` scans a Tauri bundle directory (recursively) for `*.sig` files
 * and maps each artifact to its platform keys: `<os>-<arch>-<installer>`
 * always, plus the plain `<os>-<arch>` for the preferred installer of that
 * OS (NSIS on Windows, the .app tarball on macOS, AppImage on Linux). The
 * updater looks up the installer-specific key first and falls back to the
 * plain one.
 *
 * Download URLs are `<base-url>/<asset name>`, where the asset name is the
 * artifact's file name with every character outside `[A-Za-z0-9._-]`
 * replaced by `.`, so it survives a GitHub release upload unchanged.
 * `--stage <dir>` copies each artifact and its `.sig` into `<dir>` under
 * exactly those names (and fails if an artifact is missing); upload that
 * directory. Without `--stage` only the `.sig` files are read.
 *
 * Writes the manifest to `--out` (or stdout). Exits non-zero on any problem,
 * including when no signature was found: an unsigned build has no manifest.
 */

import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";

export type Os = "windows" | "darwin" | "linux";
export type Installer = "nsis" | "msi" | "app" | "appimage" | "deb" | "rpm";

export type PlatformEntry = { signature: string; url: string };
export type Manifest = {
  version: string;
  notes?: string;
  pub_date?: string;
  platforms: Record<string, PlatformEntry>;
};

/** Installer kinds per OS, most preferred first (that one gets the plain key). */
const INSTALLER_PREFERENCE: Record<Os, Installer[]> = {
  windows: ["nsis", "msi"],
  darwin: ["app"],
  linux: ["appimage", "deb", "rpm"],
};

const INSTALLER_OS: Record<Installer, Os> = {
  nsis: "windows",
  msi: "windows",
  app: "darwin",
  appimage: "linux",
  deb: "linux",
  rpm: "linux",
};

const ARCHES = ["x86_64", "aarch64", "i686", "armv7"] as const;

const SEMVER =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;

const RFC3339 = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$/;

const PLATFORM_KEY = new RegExp(
  `^(windows|darwin|linux)-(${ARCHES.join("|")})(-(nsis|msi|app|appimage|deb|rpm))?$`,
);

export class ManifestError extends Error {}

/** Map a Rust target triple to the updater's `<os>-<arch>`. */
export function targetToPlatform(triple: string): { os: Os; arch: string } {
  const parts = triple.trim().split("-");
  const rawArch = parts[0] ?? "";
  const arch = rawArch === "armv7" || rawArch.startsWith("armv7") ? "armv7" : rawArch;
  if (!(ARCHES as readonly string[]).includes(arch)) {
    throw new ManifestError(`unsupported architecture in target "${triple}"`);
  }
  let os: Os;
  if (triple.includes("windows")) os = "windows";
  else if (triple.includes("apple-darwin")) os = "darwin";
  else if (triple.includes("linux")) os = "linux";
  else throw new ManifestError(`unsupported OS in target "${triple}"`);
  return { os, arch };
}

/** Installer kind of an updater artifact, by file name; null if unknown. */
export function classifyArtifact(fileName: string): Installer | null {
  const name = fileName.toLowerCase();
  if (name.endsWith(".exe") || name.endsWith(".nsis.zip")) return "nsis";
  if (name.endsWith(".msi") || name.endsWith(".msi.zip")) return "msi";
  if (name.endsWith(".app.tar.gz")) return "app";
  if (name.endsWith(".appimage") || name.endsWith(".appimage.tar.gz")) return "appimage";
  if (name.endsWith(".deb")) return "deb";
  if (name.endsWith(".rpm")) return "rpm";
  return null;
}

/** Release asset name that GitHub will not rewrite on upload. */
export function releaseAssetName(fileName: string): string {
  return fileName.replace(/[^A-Za-z0-9._-]/g, ".");
}

/**
 * A Tauri `.sig` file holds a base64-encoded minisign signature. Return the
 * trimmed text, or throw when it does not look like one.
 */
export function readSignature(text: string, source: string): string {
  const signature = text.trim();
  if (!signature) throw new ManifestError(`${source}: empty signature`);
  if (!/^[A-Za-z0-9+/]+={0,2}$/.test(signature)) {
    throw new ManifestError(`${source}: signature is not base64`);
  }
  const decoded = Buffer.from(signature, "base64").toString("utf8");
  if (!decoded.startsWith("untrusted comment:") || !decoded.includes("\ntrusted comment:")) {
    throw new ManifestError(`${source}: signature is not a minisign signature`);
  }
  return signature;
}

function findSigFiles(dir: string): string[] {
  const found: string[] = [];
  for (const name of readdirSync(dir)) {
    const full = join(dir, name);
    const st = statSync(full);
    if (st.isDirectory()) found.push(...findSigFiles(full));
    else if (st.isFile() && name.endsWith(".sig")) found.push(full);
  }
  return found.sort();
}

export type ArtifactInput = {
  /** Path of the artifact; its signature is `<path>.sig`. */
  path: string;
  os: Os;
  arch: string;
  installer: Installer;
};

/** Collect the signed artifacts of one Tauri bundle directory. */
export function scanBundle(triple: string, dir: string, warn: (msg: string) => void = () => {}): ArtifactInput[] {
  if (!existsSync(dir) || !statSync(dir).isDirectory()) {
    throw new ManifestError(`bundle directory not found: ${dir}`);
  }
  const { os, arch } = targetToPlatform(triple);
  const artifacts: ArtifactInput[] = [];
  for (const sig of findSigFiles(dir)) {
    const path = sig.slice(0, -".sig".length);
    const installer = classifyArtifact(basename(path));
    if (!installer) {
      warn(`skipping ${sig}: not an updater artifact`);
      continue;
    }
    if (INSTALLER_OS[installer] !== os) {
      throw new ManifestError(`${sig}: a ${installer} artifact does not belong to ${triple}`);
    }
    artifacts.push({ path, os, arch, installer });
  }
  return artifacts;
}

/** Parse `--artifact <platform key>=<file>`. */
export function explicitArtifact(key: string, path: string): ArtifactInput {
  const match = PLATFORM_KEY.exec(key);
  if (!match) throw new ManifestError(`invalid platform key "${key}"`);
  const os = match[1] as Os;
  const installer = (match[4] as Installer | undefined) ?? classifyArtifact(basename(path));
  if (!installer) throw new ManifestError(`cannot tell the installer kind of ${path}`);
  if (INSTALLER_OS[installer] !== os) {
    throw new ManifestError(`${path}: a ${installer} artifact does not belong to ${key}`);
  }
  return { path, os, arch: match[2], installer };
}

function joinUrl(base: string, name: string): string {
  return `${base.replace(/\/+$/, "")}/${encodeURIComponent(name)}`;
}

export type BuildOptions = {
  version: string;
  baseUrl: string;
  notes?: string;
  pubDate?: string;
  artifacts: ArtifactInput[];
  /** Copy artifacts and signatures here under their release asset names. */
  stageDir?: string;
  /** Read a `.sig` file (injectable for tests). */
  readSig?: (path: string) => string;
};

export function buildManifest(options: BuildOptions): Manifest {
  const version = options.version.trim().replace(/^v/, "");
  if (!SEMVER.test(version)) throw new ManifestError(`version "${options.version}" is not semver`);
  let base: URL;
  try {
    base = new URL(options.baseUrl);
  } catch {
    throw new ManifestError(`base url "${options.baseUrl}" is not a URL`);
  }
  if (base.protocol !== "https:") throw new ManifestError("base url must use https");
  const pubDate = options.pubDate ?? new Date().toISOString().replace(/\.\d{3}Z$/, "Z");
  if (!RFC3339.test(pubDate) || Number.isNaN(Date.parse(pubDate))) {
    throw new ManifestError(`pub date "${pubDate}" is not RFC 3339`);
  }
  if (options.artifacts.length === 0) {
    throw new ManifestError("no signed updater artifacts found (was TAURI_SIGNING_PRIVATE_KEY set?)");
  }

  const readSig = options.readSig ?? ((path: string) => readFileSync(path, "utf8"));
  const platforms: Record<string, PlatformEntry> = {};
  const assets = new Set<string>();
  const put = (key: string, entry: PlatformEntry) => {
    if (platforms[key]) throw new ManifestError(`two artifacts claim platform "${key}"`);
    platforms[key] = entry;
  };

  // Stable order: by OS, arch, then installer preference.
  const ordered = [...options.artifacts].sort((a, b) => {
    const rank = (x: ArtifactInput) => INSTALLER_PREFERENCE[x.os].indexOf(x.installer);
    return a.os.localeCompare(b.os) || a.arch.localeCompare(b.arch) || rank(a) - rank(b);
  });

  for (const artifact of ordered) {
    const sigPath = `${artifact.path}.sig`;
    const signature = readSignature(readSig(sigPath), sigPath);
    const asset = releaseAssetName(basename(artifact.path));
    if (assets.has(asset)) throw new ManifestError(`two artifacts share the asset name "${asset}"`);
    assets.add(asset);

    if (options.stageDir) {
      if (!existsSync(artifact.path)) throw new ManifestError(`artifact missing next to its signature: ${artifact.path}`);
      mkdirSync(options.stageDir, { recursive: true });
      copyFileSync(artifact.path, join(options.stageDir, asset));
      copyFileSync(sigPath, join(options.stageDir, `${asset}.sig`));
    }

    const entry = { signature, url: joinUrl(base.toString(), asset) };
    const platform = `${artifact.os}-${artifact.arch}`;
    put(`${platform}-${artifact.installer}`, entry);
    if (!platforms[platform]) put(platform, entry);
  }

  const notes = options.notes?.trim() ? options.notes : undefined;
  return {
    version,
    ...(notes !== undefined ? { notes } : {}),
    pub_date: pubDate,
    platforms,
  };
}

/** Problems with a static Tauri manifest; empty when it is valid. */
export function validateManifest(value: unknown): string[] {
  const problems: string[] = [];
  if (value === null || typeof value !== "object" || Array.isArray(value)) return ["manifest is not an object"];
  const m = value as Record<string, unknown>;
  if (typeof m.version !== "string" || !SEMVER.test(m.version.replace(/^v/, ""))) {
    problems.push("version must be a semver string");
  }
  if (m.notes !== undefined && typeof m.notes !== "string") problems.push("notes must be a string");
  if (m.pub_date !== undefined && (typeof m.pub_date !== "string" || !RFC3339.test(m.pub_date))) {
    problems.push("pub_date must be RFC 3339");
  }
  const platforms = m.platforms;
  if (platforms === null || typeof platforms !== "object" || Array.isArray(platforms)) {
    problems.push("platforms must be an object");
    return problems;
  }
  const entries = Object.entries(platforms as Record<string, unknown>);
  if (entries.length === 0) problems.push("platforms is empty");
  for (const [key, raw] of entries) {
    if (!PLATFORM_KEY.test(key)) problems.push(`platform key "${key}" is not <os>-<arch>[-<installer>]`);
    const entry = raw as Record<string, unknown> | null;
    if (!entry || typeof entry !== "object") {
      problems.push(`${key}: entry is not an object`);
      continue;
    }
    if (typeof entry.url !== "string" || !/^https:\/\/[^\s]+$/.test(entry.url)) {
      problems.push(`${key}: url must be an absolute https URL`);
    }
    try {
      readSignature(typeof entry.signature === "string" ? entry.signature : "", key);
    } catch (e) {
      problems.push((e as Error).message);
    }
  }
  return problems;
}

type CliArgs = {
  version?: string;
  baseUrl?: string;
  notes?: string;
  notesFile?: string;
  pubDate?: string;
  bundles: [string, string][];
  artifacts: [string, string][];
  stage?: string;
  out?: string;
};

function splitPair(flag: string, value: string): [string, string] {
  const at = value.indexOf("=");
  if (at <= 0 || at === value.length - 1) throw new ManifestError(`${flag} expects <key>=<path>, got "${value}"`);
  return [value.slice(0, at), value.slice(at + 1)];
}

export function parseArgs(argv: string[]): CliArgs {
  const args: CliArgs = { bundles: [], artifacts: [] };
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i];
    const value = argv[i + 1];
    const need = () => {
      if (value === undefined || value.startsWith("--")) throw new ManifestError(`${flag} needs a value`);
      i++;
      return value;
    };
    switch (flag) {
      case "--version": args.version = need(); break;
      case "--base-url": args.baseUrl = need(); break;
      case "--notes": args.notes = need(); break;
      case "--notes-file": args.notesFile = need(); break;
      case "--pub-date": args.pubDate = need(); break;
      case "--bundle": args.bundles.push(splitPair(flag, need())); break;
      case "--artifact": args.artifacts.push(splitPair(flag, need())); break;
      case "--stage": args.stage = need(); break;
      case "--out": args.out = need(); break;
      default: throw new ManifestError(`unknown argument "${flag}"`);
    }
  }
  return args;
}

export function run(argv: string[]): Manifest {
  const args = parseArgs(argv);
  if (!args.version) throw new ManifestError("--version is required");
  if (!args.baseUrl) throw new ManifestError("--base-url is required");
  if (args.notes !== undefined && args.notesFile !== undefined) {
    throw new ManifestError("use --notes or --notes-file, not both");
  }
  const notes = args.notesFile !== undefined ? readFileSync(args.notesFile, "utf8").trim() : args.notes;
  const warn = (msg: string) => console.error(`make-latest-json: ${msg}`);
  const artifacts = [
    ...args.bundles.flatMap(([triple, dir]) => scanBundle(triple, dir, warn)),
    ...args.artifacts.map(([key, path]) => explicitArtifact(key, path)),
  ];
  const manifest = buildManifest({
    version: args.version,
    baseUrl: args.baseUrl,
    notes,
    pubDate: args.pubDate,
    artifacts,
    stageDir: args.stage,
  });
  const problems = validateManifest(manifest);
  if (problems.length > 0) throw new ManifestError(`generated manifest is invalid:\n  ${problems.join("\n  ")}`);
  const json = `${JSON.stringify(manifest, null, 2)}\n`;
  if (args.out) writeFileSync(args.out, json);
  else process.stdout.write(json);
  return manifest;
}

if (import.meta.main) {
  try {
    run(process.argv.slice(2));
  } catch (e) {
    console.error(`make-latest-json: ${(e as Error).message}`);
    process.exit(1);
  }
}
