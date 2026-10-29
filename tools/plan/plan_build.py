# Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
# https://github.com/normiecore/Freenpipe
#
# Validates work-package (WP) coverage of the verified delta and writes the published plan:
#   docs/clean-room/PLAN.md        human-readable plan (head + generated WP sections + tail)
#   docs/clean-room/plan_wps.json  machine-readable WP list (read by tools/plan/status.py)
#
# Usage (from anywhere; paths are resolved from this file):
#   python3 tools/plan/plan_build.py            validate and rewrite both outputs
#   python3 tools/plan/plan_build.py --check    validate and fail if the outputs are stale
#
# Source data: tools/plan/plan_data.py (WPs), tools/plan/plan_head.md (sections 1-6),
# tools/plan/plan_tail.md (sections 11-13). Edit those, never the generated files.
import argparse
import json
import os
import sys
from collections import Counter, defaultdict

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)
import plan_data as P  # noqa: E402

PLAN_MD = os.path.join(ROOT, "docs", "clean-room", "PLAN.md")
PLAN_JSON = os.path.join(ROOT, "docs", "clean-room", "plan_wps.json")
HEAD_MD = os.path.join(HERE, "plan_head.md")
TAIL_MD = os.path.join(HERE, "plan_tail.md")
BUILD_CMD = "python3 tools/plan/plan_build.py"


def rng(prefix, a, b, width=3):
    return [f"{prefix}-{i:0{width}d}" for i in range(a, b + 1)]


# Canonical list of every delta item id in docs/clean-room/specs/*.md.
canon = []
canon += rng("api", 1, 40) + ["api-M01", "api-M02", "api-M03"]
canon += rng("db", 1, 29) + ["db-v2-01"]
canon += ["vision-" + s for s in [
    "device-control", "display-layout", "capture-wedge-watchdog", "probe-timeouts", "capture-loop-refinements",
    "wgc-native-capture", "corrupt-frame-guard", "pipeline-metrics", "ocr-gate", "ocr-unify-frames", "windows-ocr-worker",
    "tree-walker-worker", "uia-retained-capture", "windows-window-resolution", "url-domain-rules", "disable-screenshots",
    "capture-max-width", "recording-detail", "thumbnail-endpoint", "preview-samples", "preview-media", "elements-formats",
    "compaction-encoder", "hybrid-payload-storage", "redaction-surfaces", "redaction-pseudonyms-a11y", "redaction-models",
    "tinfoil-decloud", "telemetry-decloud", "history-access-stub", "starred-hd", "frame-comparison-hardening",
    "capture-identity-gate", "exclusion-unknown-fail-closed", "frame-media-errors"]] + ["a11y-input-hook-watchdog"]
canon += rng("audio", 1, 33) + [f"audio-M0{i}" for i in range(1, 7)]
canon += [x for x in rng("meetings_memory", 1, 47) if x != "meetings_memory-036"]
canon += rng("agents_pipes", 1, 46) + [f"agents_pipes-m0{i}" for i in range(1, 7)]
canon += ["connections-" + s for s in [
    "secret-db-split", "credential-refresh-leases", "oauth-byo-broker", "oauth-connect-v2", "oauth-default-slot-promotion",
    "secret-redaction", "webhook-proxy", "telegram-send", "slack-user-token", "imap", "outlook-email", "hermes",
    "hubspot-oauth", "zendesk-oauth", "oauth-scope-tweaks", "gmail-sheets-composio", "mcp-oauth-tiles", "pi-connect-tools",
    "bee-pairing", "obsidian-memories", "agent-memory-guidance", "browser-snapshot-v2-act", "owned-browser-tabs",
    "google-calendar-multi", "connection-triggers", "vault-secrets-db", "whatsapp-signout", "supports-oauth-instances",
    "ui-catalog", "chat-context-per-turn", "pipe-mcp-connection-refs"]]
canon += rng("frontend", 1, 56) + ["frontend-M01"]
canon += rng("shell", 1, 39) + ["shell-M01", "shell-M02"]
canon += [f"cloud-{n:03d}" for n in [1, 2, 3, 4, 5] + list(range(9, 25)) + list(range(30, 45)) + [46]]
canon += ["cloud-V01", "cloud-V02", "cloud-M01", "cloud-M02"]
canon += rng("gap_retention-v2-engine", 1, 11)
canon += ["gap_workflow-sop-video-and-media-" + s for s in [
    "core-renderer", "narration-decloud", "app-video-jobs", "render-cli", "pi-video-sop-extension",
    "video-project-workspace", "recording-replay", "captured-moment-open", "workflow-ready-notification",
    "entitlement-and-telemetry"]]
canon += rng("gap_health-lifecycle-internals", 1, 13)

# Items the verifiers dropped (already_in_base / unsupported / duplicate) -> where the residue lands.
DROPPED = {
    "api-024": ("already_in_base", "cloud-016 in W0-06 (BYO gateway or 503)"),
    "api-036": ("unsupported", "no work: /openapi.* stays (base-present); Gmail retention handled with api-031 in W2-20"),
    "api-040": ("already_in_base", "cloud-020 in W0-06 and cloud-021 in W0-08"),
    "db-028": ("duplicate", "paths owned by their WPs; secrets.sqlite counted in DB size by db-026 (W4-10)"),
    "vision-ocr-unify-frames": ("duplicate", "db-005 and db-006 in W1-01"),
    "vision-hybrid-payload-storage": ("duplicate", "db-021 (W5-07) and db-022 (W6-06); vision must read/write through them"),
    "vision-tinfoil-decloud": ("duplicate", "cloud-020 in W0-06"),
    "vision-telemetry-decloud": ("duplicate", "cloud-010 in W0-02; wedge restart stays a local log in W3-11"),
    "vision-history-access-stub": ("duplicate", "api-035 in W1-04"),
    "audio-015": ("duplicate", "session snapshot in meetings_memory-030 (W4-12); per-pid input resolution in audio-016 (W6-05)"),
    "audio-025": ("duplicate", "api-039 in W3-06"),
    "audio-029": ("duplicate", "cloud-033 in W1-02 (remote-control flags ignored, shipped defaults kept)"),
    "audio-030": ("duplicate", "cloud-017 in W0-06"),
    "audio-031": ("duplicate", "cloud-017 in W0-06"),
    "audio-032": ("duplicate", "cloud-024 in W0-09"),
    "audio-033": ("duplicate", "cloud-010 in W0-02"),
    "meetings_memory-043": ("already_in_base", "no work; screenpipe-team-memory excluded from default members by W0-05"),
    "agents_pipes-034": ("already_in_base", "cloud-016 (W0-06) and agents_pipes-033 (W3-25)"),
    "connections-secret-db-split": ("duplicate", "db-026 in W4-10 (construction-site list, pool/quarantine notes merged)"),
    "connections-credential-refresh-leases": ("duplicate", "db-026 in W4-10 (ChatGPT refresh caller merged)"),
    "connections-oauth-byo-broker": ("duplicate", "cloud-018 in W0-07 (secret key name fixed as oauth_client:<id>)"),
    "connections-gmail-sheets-composio": ("duplicate", "cloud-038 in W2-20"),
    "connections-pi-connect-tools": ("duplicate", "agents_pipes-043 (W3-26) and agents_pipes-014 (W3-23)"),
    "connections-obsidian-memories": ("duplicate", "meetings_memory-035 in W3-22"),
    "connections-agent-memory-guidance": ("duplicate", "meetings_memory-035 in W3-22 (Codex switch + restore-on-failed-cleanup merged)"),
    "connections-owned-browser-tabs": ("duplicate", "frontend-013 in W6-18"),
    "connections-connection-triggers": ("duplicate", "agents_pipes-008 in W6-03 (file is .connection-triggers.json)"),
    "connections-vault-secrets-db": ("duplicate", "meetings_memory-044 in W4-10"),
    "frontend-054": ("unsupported", "command removals recorded by frontend-056 (W8-03); screenpipe-ui-theme and console_logs keys stay"),
    "shell-015": ("duplicate", "db-023 in W7-01 (activity event, error file, pendingStorageMigration merged)"),
    "shell-016": ("duplicate", "cloud-012 and cloud-041 in W0-03"),
    "shell-027": ("duplicate", "frontend-045 in W5-21"),
    "shell-030": ("duplicate", "audio-020 in W2-19"),
    "shell-031": ("duplicate", "cloud-010 and cloud-011 in W0-02"),
    "shell-032": ("duplicate", "cloud-034 in W1-02"),
    "shell-033": ("duplicate", "cloud-036 in W0-10"),
    "shell-034": ("duplicate", "cloud-014 and cloud-030 in W0-04"),
    "shell-035": ("duplicate", "cloud-001/002 (W0-01) and cloud-043 (W1-04)"),
}


def validate():
    errs = []
    ids = [w["id"] for w in P.WPS]
    if len(ids) != len(set(ids)):
        errs.append(f"duplicate WP ids: {[k for k, v in Counter(ids).items() if v > 1]}")
    idset = set(ids)
    wave_of = {x["id"]: x["wave"] for x in P.WPS}
    where = defaultdict(list)
    for w in P.WPS:
        for it in w["items"]:
            if it.startswith("("):
                continue
            where[it].append(w["id"])
        for d in w["depends_on"]:
            if d not in idset:
                errs.append(f"{w['id']} depends on unknown {d}")
            elif wave_of[d] > w["wave"]:
                errs.append(f"{w['id']} (wave {w['wave']}) depends on later {d} (wave {wave_of[d]})")
    cs = set(canon)
    if len(canon) != len(cs):
        errs.append("canonical list has duplicates")
    for it, wps in where.items():
        if it not in cs:
            errs.append(f"unknown item {it} in {wps}")
        if len(wps) > 1:
            errs.append(f"item {it} in several WPs {wps}")
        if it in DROPPED:
            errs.append(f"dropped item {it} assigned to {wps}")
    for it in canon:
        if it not in where and it not in DROPPED:
            errs.append(f"live item {it} not assigned")
    for d in DROPPED:
        if d not in cs:
            errs.append(f"dropped id {d} not canonical")
    live = [c for c in canon if c not in DROPPED]
    return errs, live, where


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def render_json():
    doc = {
        "generated_by": BUILD_CMD,
        "source": "tools/plan/plan_data.py",
        "wave_names": {str(k): v for k, v in sorted(P.WAVE_NAMES.items())},
        "wps": P.WPS,
    }
    return json.dumps(doc, indent=1, ensure_ascii=True) + "\n"


def render_plan(live, where):
    waves = sorted(set(w["wave"] for w in P.WPS))
    by_wave = {k: [w for w in P.WPS if w["wave"] == k] for k in waves}
    size_count = Counter(w["size"] for w in P.WPS)

    out = [read(HEAD_MD).rstrip(), ""]
    out.append("## 7. Wave overview")
    out.append("")
    out.append("| Wave | Theme | WPs | Sizes |")
    out.append("|---|---|---|---|")
    for k in waves:
        sc = Counter(w["size"] for w in by_wave[k])
        out.append(f"| {k} | {P.WAVE_NAMES[k]} | {len(by_wave[k])} | "
                   + ", ".join(f"{n} {s}" for s, n in sorted(sc.items())) + " |")
    out.append(f"| total | | {len(P.WPS)} | " + ", ".join(f"{n} {s}" for s, n in sorted(size_count.items())) + " |")
    out.append("")
    out.append(f"Coverage: {len(live)} live delta items, each in exactly one WP (checked by `{BUILD_CMD}`); "
               f"{len(DROPPED)} verifier-dropped items are mapped in section 9.")
    out.append("")
    out.append("## 8. Work packages")
    out.append("")
    for k in waves:
        out.append(f"### Wave {k}: {P.WAVE_NAMES[k]}")
        out.append("")
        out.append("| WP | Size | Title | Depends on | Items |")
        out.append("|---|---|---|---|---|")
        for w in by_wave[k]:
            items = ", ".join(w["items"])
            deps = ", ".join(w["depends_on"]) or "-"
            out.append(f"| {w['id']} | {w['size']} | {w['title']} | {deps} | {items} |")
        out.append("")
        for w in by_wave[k]:
            out.append(f"#### {w['id']} {w['title']} ({w['size']})")
            out.append("")
            out.append(f"- **Items:** {', '.join(w['items'])}")
            out.append(f"- **Depends on:** {', '.join(w['depends_on']) or 'none'}")
            out.append(f"- **Specs:** {'; '.join(w['spec_refs'])}")
            out.append("")
            out.append(w["description"])
            out.append("")
            out.append("Owner paths:")
            for p in w["owner_paths"]:
                out.append(f"- `{p}`")
            out.append("")
            out.append("Acceptance:")
            for a in w["acceptance"]:
                out.append(f"- [ ] {a}")
            out.append("")
    out.append("## 9. Verifier-dropped items and where their residue lands")
    out.append("")
    out.append("| Item | Verdict | Handled by |")
    out.append("|---|---|---|")
    for k2, (v, h) in DROPPED.items():
        out.append(f"| {k2} | {v} | {h} |")
    out.append("")
    out.append("## 10. Item to WP index")
    out.append("")
    out.append("| Item | WP |")
    out.append("|---|---|")
    for it in canon:
        if it in where:
            out.append(f"| {it} | {where[it][0]} |")
    out.append("")
    out.append(read(TAIL_MD).strip())
    out.append("")
    return "\n".join(out)


def main():
    ap = argparse.ArgumentParser(description="Validate the WP plan and write docs/clean-room/PLAN.md + plan_wps.json")
    ap.add_argument("--check", action="store_true", help="do not write; exit 1 if either output is out of date")
    a = ap.parse_args()

    errs, live, where = validate()
    if errs:
        print("\n".join(errs))
        return 1
    size_count = Counter(w["size"] for w in P.WPS)
    print(f"OK: {len(P.WPS)} WPs, {len(live)} live items, {len(DROPPED)} dropped; sizes {dict(size_count)}")

    outputs = {PLAN_MD: render_plan(live, where), PLAN_JSON: render_json()}
    rel = {p: os.path.relpath(p, ROOT).replace(os.sep, "/") for p in outputs}
    if a.check:
        stale = [rel[p] for p, text in outputs.items() if not os.path.exists(p) or read(p) != text]
        if stale:
            print("out of date (run " + BUILD_CMD + "): " + ", ".join(stale))
            return 1
        print("up to date: " + ", ".join(rel.values()))
        return 0
    for p, text in outputs.items():
        # newline="" keeps LF line endings on every OS.
        with open(p, "w", encoding="utf-8", newline="") as f:
            f.write(text)
    print("wrote " + " and ".join(rel.values()))
    return 0


if __name__ == "__main__":
    sys.exit(main())
