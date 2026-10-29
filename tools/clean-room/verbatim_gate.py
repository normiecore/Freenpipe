# Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
# https://github.com/normiecore/Freenpipe
#
# Clean-room verbatim gate. Flags any run of N plain words in the given files that also occurs
# in the reverse-engineering corpus of the shipped screenpipe v2.7.104 binary. The corpus is
# NOT shipped: tools/clean-room/corpus-shingles-8.bin holds only keyed one-way hashes of its
# N-word prose runs. Identifiers (snake_case, paths, routes, dotted names) and SQL lines are
# not prose and never match.
#
# Usage: python -I tools/clean-room/verbatim_gate.py [--index FILE] [--json OUT] PATH [PATH ...]
# Exit code 1 when any line matches (use it in CI), 0 otherwise.
import sys, os, re, json, struct, hashlib, argparse
from array import array
from bisect import bisect_left

sys.stdout.reconfigure(encoding='utf-8', errors='replace')

SALT = b'freenpipe-clean-room-v1'
TOKEN = re.compile(r"[A-Za-z0-9_'./:@#$%&=+\-]+")
PROSE = re.compile(r"^[a-z']+$")
SQL_LINE = re.compile(r"^\s*(CREATE|ALTER|DROP|INSERT|SELECT|UPDATE|DELETE|PRAGMA|WITH)\s+[A-Z_(*]")
EXTS = ('.md', '.txt', '.json', '.rs', '.ts', '.tsx', '.js', '.jsx', '.mjs', '.py', '.toml', '.yml', '.yaml', '.html', '.css', '.swift')
SKIP_DIRS = {'.git', 'node_modules', 'target', 'out', '.next', 'third_party', 'dist'}


def words(text):
    out = []
    for t in TOKEN.findall(text.lower()):
        t = t.strip(".:-'")
        out.append(t if PROSE.match(t or '_') else None)
    return out


def shingles_of(w, n):
    for i in range(len(w) - n + 1):
        seg = w[i:i + n]
        if None not in seg:
            yield ' '.join(seg)


def h64(s):
    return struct.unpack('<Q', hashlib.blake2b(s.encode(), digest_size=8, key=SALT).digest())[0]


def load_index(path):
    with open(path, 'rb') as f:
        head = f.read(6)
        if head[:5] != b'FPSH1':
            raise SystemExit(f'{path}: not a shingle index')
        n = head[5]
        arr = array('Q')
        arr.frombytes(f.read())
    return n, arr


def contains(arr, x):
    i = bisect_left(arr, x)
    return i < len(arr) and arr[i] == x


def files_under(paths):
    for p in paths:
        if os.path.isfile(p):
            yield p
            continue
        for root, dirs, files in os.walk(p):
            dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
            for f in files:
                if f.endswith(EXTS) and not f.endswith('.bin'):
                    yield os.path.join(root, f)


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    ap = argparse.ArgumentParser()
    ap.add_argument('paths', nargs='+')
    ap.add_argument('--index', default=os.path.join(here, 'corpus-shingles-8.bin'))
    ap.add_argument('--json')
    a = ap.parse_args()
    n, arr = load_index(a.index)
    hits = []
    for f in files_under(a.paths):
        in_sql = False
        try:
            lines = open(f, encoding='utf-8', errors='replace').read().splitlines()
        except OSError:
            continue
        for ln, line in enumerate(lines, 1):
            if line.lstrip().startswith('```'):
                in_sql = line.strip().lower().startswith('```sql') and not in_sql
                continue
            if in_sql or SQL_LINE.match(line):
                continue
            runs = [s for s in shingles_of(words(line), n) if contains(arr, h64(s))]
            if runs:
                hits.append({'file': f, 'line': ln, 'runs': runs[:3], 'text': line.strip()[:240]})
    if a.json:
        json.dump(hits, open(a.json, 'w', encoding='utf-8'), indent=1)
    for h in hits:
        print(f"{h['file']}:{h['line']}: {h['text'][:160]}")
    print(f'{len(hits)} line(s) share a {n}-word prose run with the target corpus')
    sys.exit(1 if hits else 0)


if __name__ == '__main__':
    main()
