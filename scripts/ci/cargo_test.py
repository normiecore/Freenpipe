# Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
# https://github.com/normiecore/Freenpipe
"""Run `cargo test` for CI with the test quarantine applied, and summarise it.

    python scripts/ci/cargo_test.py --platform windows --suite engine [--quarantined]
        [--also-skip NAME ...] -- <cargo test arguments>

Default mode runs `cargo test <args> -- --exact --skip <name> ...` for every
entry of scripts/ci/test-quarantine.txt that matches the platform and suite,
and streams the output. A test binary that dies without a failed test (an
access violation, a signal) is run once more on its own with
--test-threads=1. If that passes, the crash counts as flaky: it is reported as
a warning and does not fail the step. Any failed test, any crash that repeats,
or more than MAX_CRASH_RETRIES crashed binaries fail the step.

--quarantined runs only the quarantined tests of that platform and suite
(`cargo test <args> --lib --bins --tests -- --exact <name> ...`, so no doc
tests), reports each one's outcome, flags any that pass, and always exits 0:
it keeps the quarantine honest without blocking a merge.

Both modes end the log with a "cargo-test summary" block (failed tests, failed
targets with cargo's rerun hint), add annotations for failures (the check-run
annotations API exposes them without a log download), and append a table to
$GITHUB_STEP_SUMMARY when it is set.
"""

import argparse
import os
import re
import shlex
import subprocess
import sys
from pathlib import Path

QUARANTINE = Path(__file__).resolve().parent / "test-quarantine.txt"
ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
RUNNING = re.compile(r"^\s*(Running|Doc-tests)\s+(.*)$")
TEST_LINE = re.compile(r"^test (\S.*?) \.\.\. (ok|FAILED|ignored.*)$")
RESULT = re.compile(r"^test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored;")
RERUN = re.compile(r"^error: test failed, to rerun pass `([^`]*)`")
MAX_ANNOTATIONS = 10
MAX_CRASH_RETRIES = 3


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


def target_label(kind, rest):
    """('Running', 'unittests src\\lib.rs (target\\...\\screenpipe_a11y-71d8.exe)') -> 'screenpipe_a11y (unittests src/lib.rs)'."""
    if kind == "Doc-tests":
        return f"doc-tests {rest.strip()}"
    m = re.match(r"^(.*?)\s*\((.*)\)\s*$", rest)
    if not m:
        return rest.strip()
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
    """Returns outcomes {test: [(target, outcome)]}, failed [(target, test)],
    failed targets [(rerun spec, target, crashed)], totals [passed, failed, ignored]."""
    current = ""
    current_failed = 0
    outcomes = {}
    failed = []
    targets = []
    totals = [0, 0, 0]
    for line in lines:
        m = RUNNING.match(line)
        if m:
            current = target_label(m.group(1), m.group(2))
            current_failed = 0
            continue
        m = TEST_LINE.match(line)
        if m:
            name, outcome = m.group(1), m.group(2)
            outcomes.setdefault(name, []).append((current, outcome))
            if outcome == "FAILED":
                failed.append((current, name))
                current_failed += 1
            continue
        m = RESULT.match(line)
        if m:
            for i in range(3):
                totals[i] += int(m.group(i + 1))
            continue
        m = RERUN.match(line)
        if m:
            # cargo prints this right after the binary ends. A failure without
            # a failed test means the process died (mid-run or at exit).
            targets.append((m.group(1), current, current_failed == 0))
    return outcomes, failed, targets, totals


def target_flags(spec):
    """cargo's rerun hint ('-p screenpipe-engine --test tags_test') minus the
    package. The retry keeps the original package selection, so feature
    resolution, and with it every compiled binary, stays the same; with
    --workspace cargo would ignore -p anyway. A `--lib` retry therefore runs
    the unit tests of every selected crate."""
    out, skip = [], False
    for a in shlex.split(spec):
        if skip:
            skip = False
        elif a in ("-p", "--package"):
            skip = True
        elif not (a.startswith("--package=") or (a.startswith("-p") and len(a) > 2)):
            out.append(a)
    return out


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

    libtest = []
    if opts.quarantined:
        if not names:
            print(f"cargo_test: nothing quarantined for {label}")
            return 0
        # Without an explicit target selection, leave out doc tests: they are
        # never quarantined and rustdoc would compile them again.
        explicit = {"--lib", "--bin", "--bins", "--test", "--tests", "--doc"} & set(cargo_args)
        targets_sel = [] if explicit else ["--lib", "--bins", "--tests"]
        cmd = ["cargo", "test", *cargo_args, *targets_sel, "--", "--exact", *names]
    else:
        skips = names + opts.also_skip
        if skips:
            libtest.append("--exact")
            for n in skips:
                libtest += ["--skip", n]
        cmd = ["cargo", "test", *cargo_args, "--", *libtest]

    code, lines = run(cmd)
    outcomes, failed, targets, totals = parse(lines)

    # ---- retry crashed binaries once, alone and single-threaded --------------
    flaky, crashed_again = [], []
    crashes = [t for t in targets if t[2]]
    if not opts.quarantined and crashes:
        if len(crashes) > MAX_CRASH_RETRIES:
            print(f"cargo_test: {len(crashes)} test binaries crashed; not retrying more than {MAX_CRASH_RETRIES}")
            crashed_again = crashes
        else:
            for spec, target, _ in crashes:
                flags = target_flags(spec)
                if not flags:
                    crashed_again.append((spec, target, True))
                    continue
                print(f"\ncargo_test: {target} crashed without a failed test; retrying it alone with --test-threads=1", flush=True)
                rc, rlines = run(["cargo", "test", *cargo_args, *flags, "--", *libtest, "--test-threads=1"])
                r_outcomes, r_failed, _, r_totals = parse(rlines)
                for k, v in r_outcomes.items():
                    outcomes.setdefault(k, []).extend(v)
                for i in range(3):
                    totals[i] += r_totals[i]
                if rc == 0:
                    flaky.append((spec, target))
                else:
                    crashed_again.append((spec, target, True))
                    failed.extend(r_failed)
        hard = [t for t in targets if not t[2]]
        if code != 0 and not hard and not crashed_again and not failed:
            code = 0

    failed_specs = [t[0] for t in targets if not t[2]] + [t[0] for t in crashed_again]

    print(f"\n==== cargo-test summary ({label}{', quarantined only' if opts.quarantined else ''}) ====")
    print(f"exit {code}; passed {totals[0]}, failed {totals[1]}, ignored {totals[2]}; "
          f"{len(names)} quarantined test(s) {'run' if opts.quarantined else 'skipped'}")
    for target, name in failed:
        print(f"FAILED  {name}    [{target}]")
    for spec in failed_specs:
        print(f"TARGET FAILED  cargo test ... {spec}")
    for spec, target in flaky:
        print(f"FLAKY CRASH  {target}: crashed, then passed alone with --test-threads=1 ({spec})")

    md = []
    if opts.quarantined:
        md.append(f"### Quarantined tests ({label}, informational)\n\n")
        md.append("| Test | Outcome | Why quarantined |\n|---|---|---|\n")
        for name, reason in entries:
            seen = outcomes.get(name, [])
            if not seen:
                result = "no result (binary crashed or test not found)"
            elif all(o == "ok" for _, o in seen):
                result = "**passed**: if it keeps passing, remove it from the quarantine"
                print(f"::warning title=quarantined test passed::{esc(name)} passed on {label}; "
                      f"if it keeps passing, remove it from scripts/ci/test-quarantine.txt")
            else:
                result = ", ".join(sorted({o for _, o in seen}))
            print(f"QUARANTINED  {name}: {result}")
            md.append(f"| `{name}` | {result} | {reason.replace('|', '/')} |\n")
        code = 0
    else:
        # GitHub keeps 10 error annotations per step: failed tests first, then
        # one line naming every failed target (a crashed binary has no failed
        # test line, only a target).
        for target, name in failed[:MAX_ANNOTATIONS - 1]:
            print(f"::error title=test failed ({label})::{esc(name)} in {esc(target)}")
        if failed_specs:
            print(f"::error title=failed test targets ({label})::{esc('; '.join(failed_specs))}")
        for spec, target in flaky:
            print(f"::warning title=flaky test crash ({label})::{esc(target)} crashed, then passed alone "
                  f"with --test-threads=1 ({esc(spec)})")
        if code != 0 and not failed and not failed_specs:
            print(f"::error title=cargo test failed ({label})::cargo exited {code} before running tests (build error?)")
        md.append(f"### cargo test ({label})\n\n")
        md.append(f"exit {code}: {totals[0]} passed, {totals[1]} failed, {totals[2]} ignored, "
                  f"{len(names)} quarantined test(s) skipped (scripts/ci/test-quarantine.txt).\n\n")
        if failed:
            md.append("| Failed test | Target |\n|---|---|\n")
            md.extend(f"| `{n}` | {t} |\n" for t, n in failed)
            md.append("\n")
        if failed_specs:
            md.append("Failed targets (cargo rerun hints):\n\n")
            md.extend(f"- `{s}`\n" for s in failed_specs)
            md.append("\n")
        if flaky:
            md.append("Flaky crashes (passed on a single-threaded retry):\n\n")
            md.extend(f"- {t} (`{s}`)\n" for s, t in flaky)
            md.append("\n")

    if summary_path:
        with open(summary_path, "a", encoding="utf-8") as fh:
            fh.write("".join(md) + "\n")
    return code


if __name__ == "__main__":
    sys.exit(main())
