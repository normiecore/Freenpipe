# Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
# https://github.com/normiecore/Freenpipe
"""Run `cargo test` for CI with the test quarantine applied, and summarise it.

    python scripts/ci/cargo_test.py --platform windows --suite engine [--quarantined]
        [--also-skip NAME ...] -- <cargo test arguments>

Default mode runs `cargo test <args> -- --exact --skip <name> ...` for every
entry of scripts/ci/test-quarantine.txt that matches the platform and suite,
streams the output, and exits with cargo's exit code.

--quarantined runs only the quarantined tests of that platform and suite
(`cargo test <args> --lib --bins --tests -- --exact <name> ...`, so no doc
tests), reports each one's outcome, flags any that now pass, and always exits
0: it keeps the quarantine honest without blocking a merge.

Both modes end the log with a "cargo-test summary" block (failed tests, failed
targets with cargo's rerun hint), add ::error annotations for failures (the
check-run annotations API exposes them without a log download), and append a
table to $GITHUB_STEP_SUMMARY when it is set.
"""

import argparse
import os
import re
import subprocess
import sys
from pathlib import Path

QUARANTINE = Path(__file__).resolve().parent / "test-quarantine.txt"
ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
RUNNING = re.compile(r"^\s*(?:Running|Doc-tests)\s+(.*)$")
TEST_LINE = re.compile(r"^test (\S.*?) \.\.\. (ok|FAILED|ignored.*)$")
RESULT = re.compile(r"^test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored;")
RERUN = re.compile(r"^error: test failed, to rerun pass `([^`]*)`")
MAX_ANNOTATIONS = 10


def load_quarantine(platform, suite):
    """Entries are `<platform> <suite> <exact test name> <reason...>`."""
    entries = []
    for raw in QUARANTINE.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split(None, 3)
        if len(parts) < 3:
            sys.exit(f"cargo_test: malformed quarantine line: {raw!r}")
        plat, sui, name = parts[0], parts[1], parts[2]
        reason = parts[3] if len(parts) > 3 else ""
        if plat in (platform, "all") and sui == suite:
            entries.append((name, reason))
    return entries


def target_label(running):
    """'unittests src\\lib.rs (target\\...\\deps\\screenpipe_a11y-71d8.exe)' -> 'screenpipe_a11y (unittests src/lib.rs)'."""
    m = re.match(r"^(.*?)\s*\((.*)\)\s*$", running)
    if not m:
        return running.strip()
    what, exe = m.group(1).replace("\\", "/"), m.group(2).replace("\\", "/")
    stem = exe.rsplit("/", 1)[-1]
    stem = re.sub(r"\.exe$", "", stem)
    stem = re.sub(r"-[0-9a-f]{16}$", "", stem)
    return f"{stem} ({what})"


def run(cmd):
    print("+ " + " ".join(cmd), flush=True)
    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    out = sys.stdout.buffer
    lines = []
    for chunk in iter(proc.stdout.readline, b""):
        out.write(chunk)
        out.flush()
        lines.append(ANSI.sub("", chunk.decode("utf-8", "replace")).rstrip("\r\n"))
    return proc.wait(), lines


def parse(lines):
    current = ""
    outcomes = {}  # test name -> list of (target, outcome)
    failed = []  # (target, test)
    rerun = []
    totals = [0, 0, 0]
    for line in lines:
        m = RUNNING.match(line)
        if m:
            current = target_label(m.group(1)) if line.lstrip().startswith("Running") else f"doc-tests {m.group(1).strip()}"
            continue
        m = TEST_LINE.match(line)
        if m:
            name, outcome = m.group(1), m.group(2)
            outcomes.setdefault(name, []).append((current, outcome))
            if outcome == "FAILED":
                failed.append((current, name))
            continue
        m = RESULT.match(line)
        if m:
            for i in range(3):
                totals[i] += int(m.group(i + 1))
            continue
        m = RERUN.match(line)
        if m:
            rerun.append(m.group(1))
    return outcomes, failed, rerun, totals


def esc(s):
    return s.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")


def main():
    argv = sys.argv[1:]
    cargo_args = []
    if "--" in argv:
        i = argv.index("--")
        argv, cargo_args = argv[:i], argv[i + 1:]
    ap = argparse.ArgumentParser()
    ap.add_argument("--platform", required=True, choices=["windows", "linux", "macos"])
    ap.add_argument("--suite", required=True)
    ap.add_argument("--quarantined", action="store_true")
    ap.add_argument("--also-skip", action="append", default=[])
    opts = ap.parse_args(argv)

    entries = load_quarantine(opts.platform, opts.suite)
    names = [n for n, _ in entries]
    label = f"{opts.platform}/{opts.suite}"
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")

    if opts.quarantined:
        if not names:
            print(f"cargo_test: nothing quarantined for {label}")
            return 0
        # Without an explicit target selection, leave out doc tests: they are
        # never quarantined and rustdoc would compile them again.
        explicit = {"--lib", "--bin", "--bins", "--test", "--tests", "--doc"} & set(cargo_args)
        targets = [] if explicit else ["--lib", "--bins", "--tests"]
        cmd = ["cargo", "test", *cargo_args, *targets, "--", "--exact", *names]
    else:
        skips = names + opts.also_skip
        cmd = ["cargo", "test", *cargo_args, "--"]
        if skips:
            cmd.append("--exact")
            for n in skips:
                cmd += ["--skip", n]

    code, lines = run(cmd)
    outcomes, failed, rerun, totals = parse(lines)

    print(f"\n==== cargo-test summary ({label}{', quarantined only' if opts.quarantined else ''}) ====")
    print(f"exit {code}; passed {totals[0]}, failed {totals[1]}, ignored {totals[2]}; "
          f"{len(names)} quarantined test(s) {'run' if opts.quarantined else 'skipped'}")
    for target, name in failed:
        print(f"FAILED  {name}    [{target}]")
    for spec in rerun:
        print(f"TARGET FAILED  cargo test ... {spec}")

    md = []
    if opts.quarantined:
        md.append(f"### Quarantined tests ({label}, informational)\n")
        md.append("| Test | Outcome | Why quarantined |\n|---|---|---|\n")
        for name, reason in entries:
            seen = outcomes.get(name, [])
            if not seen:
                result = "no result (binary crashed or test not found)"
            elif all(o == "ok" for _, o in seen):
                result = "**passes now**: remove it from the quarantine"
                print(f"::warning title=quarantined test passes::{esc(name)} passes on {label}; "
                      f"remove it from scripts/ci/test-quarantine.txt")
            else:
                result = ", ".join(sorted({o for _, o in seen}))
            print(f"QUARANTINED  {name}: {result}")
            md.append(f"| `{name}` | {result} | {reason.replace('|', '/')} |\n")
        code = 0
    else:
        # GitHub keeps 10 error annotations per step: failed tests first, then
        # one line naming every failed target (a crashed binary has no FAILED
        # test line, only a target).
        for target, name in failed[:MAX_ANNOTATIONS - 1]:
            print(f"::error title=test failed ({label})::{esc(name)} in {esc(target)}")
        if rerun:
            print(f"::error title=failed test targets ({label})::{esc('; '.join(rerun))}")
        if code != 0 and not failed and not rerun:
            print(f"::error title=cargo test failed ({label})::cargo exited {code} before running tests (build error?)")
        md.append(f"### cargo test ({label})\n\n")
        md.append(f"exit {code}: {totals[0]} passed, {totals[1]} failed, {totals[2]} ignored, "
                  f"{len(names)} quarantined test(s) skipped (scripts/ci/test-quarantine.txt).\n\n")
        if failed:
            md.append("| Failed test | Target |\n|---|---|\n")
            md.extend(f"| `{n}` | {t} |\n" for t, n in failed)
            md.append("\n")
        if rerun:
            md.append("Failed targets (cargo rerun hints):\n\n")
            md.extend(f"- `{s}`\n" for s in rerun)
            md.append("\n")

    if summary_path:
        with open(summary_path, "a", encoding="utf-8") as fh:
            fh.write("".join(md) + "\n")
    return code


if __name__ == "__main__":
    sys.exit(main())
