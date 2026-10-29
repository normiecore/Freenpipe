// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

/**
 * Tests for the release manifest generator. Run from the repository root:
 *   bun test scripts/release/make-latest-json.test.ts
 *
 * `fixtures/latest.json` is the golden output for the `.sig` fixtures under
 * `fixtures/bundle/`; the app crate parses the same file with the updater
 * plugin's own deserializer (`updates::tests`), so a format drift fails on
 * both sides. Regenerate it with the command in `GOLDEN_ARGS` if the format
 * changes on purpose.
 */

import { afterEach, describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  buildManifest,
  classifyArtifact,
  explicitArtifact,
  ManifestError,
  parseArgs,
  readSignature,
  releaseAssetName,
  run,
  scanBundle,
  targetToPlatform,
  validateManifest,
} from "./make-latest-json";

const FIXTURES = join(import.meta.dir, "fixtures");
const BUNDLES = join(FIXTURES, "bundle");
const BASE_URL = "https://github.com/normiecore/Handvel/releases/download/v1.2.3";

const GOLDEN_ARGS = [
  "--version", "v1.2.3",
  "--base-url", BASE_URL,
  "--notes", "Fixture release for make-latest-json tests.",
  "--pub-date", "2026-01-01T00:00:00Z",
  "--bundle", `x86_64-pc-windows-msvc=${join(BUNDLES, "x86_64-pc-windows-msvc")}`,
  "--bundle", `aarch64-apple-darwin=${join(BUNDLES, "aarch64-apple-darwin")}`,
  "--bundle", `x86_64-unknown-linux-gnu=${join(BUNDLES, "x86_64-unknown-linux-gnu")}`,
];

/** A signature in the same encoding Tauri uses (base64 of minisign text). */
function fakeSig(file: string): string {
  const text = [
    "untrusted comment: signature from tauri secret key (test)",
    `RUQ${"A".repeat(97)}=`,
    `trusted comment: timestamp:0\tfile:${file}`,
    `${"A".repeat(86)}==`,
    "",
  ].join("\n");
  return Buffer.from(text).toString("base64");
}

const temps: string[] = [];
function tempDir(): string {
  const dir = mkdtempSync(join(tmpdir(), "make-latest-json-"));
  temps.push(dir);
  return dir;
}
afterEach(() => {
  while (temps.length) rmSync(temps.pop()!, { recursive: true, force: true });
});

/** Write `files` (relative path -> contents) under a fresh temp dir. */
function bundle(files: Record<string, string>): string {
  const root = tempDir();
  for (const [rel, contents] of Object.entries(files)) {
    const full = join(root, rel);
    mkdirSync(join(full, ".."), { recursive: true });
    writeFileSync(full, contents);
  }
  return root;
}

describe("make-latest-json", () => {
  test("fixture .sig files produce the golden, valid latest.json", () => {
    const out = join(tempDir(), "latest.json");
    const manifest = run([...GOLDEN_ARGS, "--out", out]);

    expect(validateManifest(manifest)).toEqual([]);
    expect(readFileSync(out, "utf8")).toBe(readFileSync(join(FIXTURES, "latest.json"), "utf8"));

    expect(manifest.version).toBe("1.2.3");
    expect(Object.keys(manifest.platforms).sort()).toEqual([
      "darwin-aarch64",
      "darwin-aarch64-app",
      "linux-x86_64",
      "linux-x86_64-appimage",
      "windows-x86_64",
      "windows-x86_64-nsis",
    ]);
    const win = manifest.platforms["windows-x86_64"];
    expect(win).toEqual(manifest.platforms["windows-x86_64-nsis"]);
    expect(win.url).toBe(`${BASE_URL}/Handvel_1.2.3_x64-setup.exe`);
    expect(win.signature).toBe(
      readFileSync(join(BUNDLES, "x86_64-pc-windows-msvc/nsis/Handvel_1.2.3_x64-setup.exe.sig"), "utf8").trim(),
    );
  });

  test("maps rust targets to updater platforms", () => {
    expect(targetToPlatform("x86_64-pc-windows-msvc")).toEqual({ os: "windows", arch: "x86_64" });
    expect(targetToPlatform("aarch64-pc-windows-msvc")).toEqual({ os: "windows", arch: "aarch64" });
    expect(targetToPlatform("x86_64-apple-darwin")).toEqual({ os: "darwin", arch: "x86_64" });
    expect(targetToPlatform("aarch64-unknown-linux-gnu")).toEqual({ os: "linux", arch: "aarch64" });
    expect(() => targetToPlatform("wasm32-unknown-unknown")).toThrow(ManifestError);
    expect(() => targetToPlatform("x86_64-unknown-freebsd")).toThrow(ManifestError);
  });

  test("classifies updater artifacts by name", () => {
    expect(classifyArtifact("Handvel_1.0.0_x64-setup.exe")).toBe("nsis");
    expect(classifyArtifact("Handvel_1.0.0_x64-setup.nsis.zip")).toBe("nsis");
    expect(classifyArtifact("Handvel_1.0.0_x64_en-US.msi")).toBe("msi");
    expect(classifyArtifact("Handvel.app.tar.gz")).toBe("app");
    expect(classifyArtifact("Handvel_1.0.0_amd64.AppImage")).toBe("appimage");
    expect(classifyArtifact("handvel_1.0.0_amd64.deb")).toBe("deb");
    expect(classifyArtifact("Handvel.dmg")).toBeNull();
  });

  test("asset names survive a GitHub upload unchanged", () => {
    expect(releaseAssetName("Handvel beta_1.0.0_x64-setup.exe")).toBe("Handvel.beta_1.0.0_x64-setup.exe");
    expect(releaseAssetName("Handvel_1.0.0+build.7_x64-setup.exe")).toBe("Handvel_1.0.0.build.7_x64-setup.exe");
    expect(releaseAssetName("plain-name_1.0.tar.gz")).toBe("plain-name_1.0.tar.gz");
  });

  test("--stage copies artifacts and signatures under their asset names", () => {
    const dir = bundle({
      "nsis/Handvel beta_2.0.0_x64-setup.exe": "installer bytes",
      "nsis/Handvel beta_2.0.0_x64-setup.exe.sig": fakeSig("Handvel beta_2.0.0_x64-setup.exe"),
    });
    const stage = join(tempDir(), "stage");
    const manifest = buildManifest({
      version: "2.0.0",
      baseUrl: "https://github.com/o/r/releases/download/v2.0.0/",
      pubDate: "2026-02-03T04:05:06Z",
      artifacts: scanBundle("x86_64-pc-windows-msvc", dir),
      stageDir: stage,
    });
    const asset = "Handvel.beta_2.0.0_x64-setup.exe";
    expect(manifest.platforms["windows-x86_64"].url).toBe(`https://github.com/o/r/releases/download/v2.0.0/${asset}`);
    expect(readFileSync(join(stage, asset), "utf8")).toBe("installer bytes");
    expect(existsSync(join(stage, `${asset}.sig`))).toBe(true);
    expect(manifest.notes).toBeUndefined();
  });

  test("--stage refuses a signature without its artifact", () => {
    const dir = bundle({ "nsis/App_1.0.0_x64-setup.exe.sig": fakeSig("App_1.0.0_x64-setup.exe") });
    expect(() =>
      buildManifest({
        version: "1.0.0",
        baseUrl: BASE_URL,
        artifacts: scanBundle("x86_64-pc-windows-msvc", dir),
        stageDir: join(tempDir(), "stage"),
      }),
    ).toThrow(/artifact missing/);
  });

  test("skips unknown signed files and rejects artifacts for the wrong OS", () => {
    const warnings: string[] = [];
    const dir = bundle({
      "nsis/App_1.0.0_x64-setup.exe.sig": fakeSig("App_1.0.0_x64-setup.exe"),
      "share/readme.txt.sig": fakeSig("readme.txt"),
    });
    expect(scanBundle("x86_64-pc-windows-msvc", dir, (m) => warnings.push(m))).toHaveLength(1);
    expect(warnings.join()).toContain("readme.txt.sig");

    const mac = bundle({ "macos/App.app.tar.gz.sig": fakeSig("App.app.tar.gz") });
    expect(() => scanBundle("x86_64-pc-windows-msvc", mac)).toThrow(/does not belong/);
    expect(() => scanBundle("x86_64-pc-windows-msvc", join(mac, "missing"))).toThrow(/not found/);
  });

  test("explicit artifacts and preferred installers", () => {
    const dir = bundle({
      "App_1.0.0_x64_en-US.msi.sig": fakeSig("App_1.0.0_x64_en-US.msi"),
      "App_1.0.0_x64-setup.exe.sig": fakeSig("App_1.0.0_x64-setup.exe"),
    });
    const manifest = buildManifest({
      version: "1.0.0",
      baseUrl: BASE_URL,
      pubDate: "2026-01-01T00:00:00Z",
      artifacts: [
        explicitArtifact("windows-x86_64-msi", join(dir, "App_1.0.0_x64_en-US.msi")),
        explicitArtifact("windows-x86_64", join(dir, "App_1.0.0_x64-setup.exe")),
      ],
    });
    // NSIS is preferred on Windows, whatever the input order.
    expect(manifest.platforms["windows-x86_64"].url).toEndWith("App_1.0.0_x64-setup.exe");
    expect(manifest.platforms["windows-x86_64-msi"].url).toEndWith("App_1.0.0_x64_en-US.msi");
    expect(() => explicitArtifact("windows-x86_64-app", "x.app.tar.gz")).toThrow(/does not belong/);
    expect(() => explicitArtifact("beos-x86_64", "x.exe")).toThrow(/invalid platform key/);
  });

  test("rejects bad inputs", () => {
    const sig = bundle({ "a/App_1.0.0_x64-setup.exe.sig": fakeSig("App_1.0.0_x64-setup.exe") });
    const artifacts = scanBundle("x86_64-pc-windows-msvc", sig);
    const base = { baseUrl: BASE_URL, artifacts };
    expect(() => buildManifest({ ...base, version: "1.2" })).toThrow(/semver/);
    expect(() => buildManifest({ ...base, version: "1.0.0", baseUrl: "http://example.test/x" })).toThrow(/https/);
    expect(() => buildManifest({ ...base, version: "1.0.0", pubDate: "yesterday" })).toThrow(/RFC 3339/);
    expect(() => buildManifest({ ...base, version: "1.0.0", artifacts: [] })).toThrow(/no signed updater artifacts/);
    expect(() =>
      buildManifest({ ...base, version: "1.0.0", artifacts: [...artifacts, ...artifacts] }),
    ).toThrow(/claim platform|share the asset name/);
    expect(() => buildManifest({ ...base, version: "1.0.0", readSig: () => "not base64!" })).toThrow(/base64/);
    expect(() =>
      buildManifest({ ...base, version: "1.0.0", readSig: () => Buffer.from("hello").toString("base64") }),
    ).toThrow(/minisign/);
  });

  test("signature check accepts Tauri .sig text with surrounding whitespace", () => {
    expect(readSignature(`\n${fakeSig("x")}\n`, "x.sig")).toBe(fakeSig("x"));
    expect(() => readSignature("   ", "x.sig")).toThrow(/empty/);
  });

  test("validateManifest reports structural problems", () => {
    expect(validateManifest(null)).toEqual(["manifest is not an object"]);
    const problems = validateManifest({
      version: "one",
      pub_date: "2026-01-01",
      platforms: { "windows-x86_64": { url: "ftp://x", signature: "" }, "plan9-x86_64": {} },
    });
    expect(problems.join("\n")).toContain("semver");
    expect(problems.join("\n")).toContain("RFC 3339");
    expect(problems.join("\n")).toContain("https URL");
    expect(problems.join("\n")).toContain("empty signature");
    expect(problems.join("\n")).toContain('"plan9-x86_64"');
    expect(validateManifest({ version: "1.0.0", platforms: {} })).toEqual(["platforms is empty"]);
  });

  test("argument parsing", () => {
    expect(parseArgs(["--version", "1.0.0", "--bundle", "x86_64-pc-windows-msvc=C:/b=c"]).bundles).toEqual([
      ["x86_64-pc-windows-msvc", "C:/b=c"],
    ]);
    expect(() => parseArgs(["--version"])).toThrow(/needs a value/);
    expect(() => parseArgs(["--bundle", "nokey"])).toThrow(/<key>=<path>/);
    expect(() => parseArgs(["--frobnicate", "1"])).toThrow(/unknown argument/);
    expect(() => run(["--base-url", BASE_URL])).toThrow(/--version is required/);
    expect(() => run(["--version", "1.0.0"])).toThrow(/--base-url is required/);
  });
});
