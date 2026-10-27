# Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
# https://github.com/normiecore/Freenpipe
"""Seed a scratch Freenpipe database with synthetic parsed content (W3-02 smoke).

    python -I docs/dev/smoke/W3-02-seed.py "$FP_DD/db.sqlite" [--bulk-frames N]

The database must already carry the Freenpipe schema: start the engine once on
the scratch data dir, stop it, then run this script, then start the engine
again. Every row is invented here; nothing is read from a real recording.

What it writes (timestamps a few minutes before now, device `smoke_monitor`):

- a "SmokeChat" frame with a parser run holding a conversation and two
  messages, one by "Ada Smoke" (mentions "xylophone") and one by "Bob Smoke";
- a "SmokeNotes" frame with a run holding one document (also "xylophone");
- a "SmokeChat" frame whose run is `empty` and a plain frame with no run,
  neither of which `content_type=parsed` may return;
- the two actors and their item assignments.

`--bulk-frames N` adds N plain frames of filler text (each holding the word
"a") spread over the past 30 days, for the slow-search check. Each run of the
script uses fresh fingerprints, so it can be run more than once.

The script refuses a database inside ~/.screenpipe.
"""

import argparse
import datetime as dt
import hashlib
import os
import sqlite3
import sys

DEVICE = "smoke_monitor"


def ts(t):
    """The engine's spelling of a UTC instant (RFC 3339, millisecond, +00:00)."""
    return t.strftime("%Y-%m-%dT%H:%M:%S.") + "%03d+00:00" % (t.microsecond // 1000)


def digest(nonce, label, size=32):
    return hashlib.sha256(("w3-02-smoke:%s:%s" % (nonce, label)).encode()).digest()[:size]


def refuse_real_data_dir(path):
    real = os.path.normcase(os.path.realpath(os.path.expanduser("~/.screenpipe")))
    target = os.path.normcase(os.path.realpath(path))
    if target == real or target.startswith(real + os.sep):
        sys.exit("refusing to write inside ~/.screenpipe; use a scratch FP_DD")


def check_schema(con):
    tables = {r[0] for r in con.execute("SELECT name FROM sqlite_master WHERE type = 'table'")}
    missing = {"frames", "semantic_runs", "semantic_items", "semantic_run_items",
               "semantic_actors", "semantic_item_actors"} - tables
    columns = {r[1] for r in con.execute("PRAGMA table_info(frames)")}
    if missing or "semantic_run_id" not in columns:
        sys.exit("no Freenpipe schema here (missing %s); start the engine once on this data dir"
                 % ", ".join(sorted(missing) or ["frames.semantic_run_id"]))


def add_frame(con, when, app, window, text):
    cur = con.execute(
        "INSERT INTO frames (timestamp, app_name, window_name, device_name, full_text) "
        "VALUES (?, ?, ?, ?, ?)",
        (ts(when), app, window, DEVICE, text),
    )
    return cur.lastrowid


def add_actor(con, name):
    return con.execute("INSERT INTO semantic_actors (name) VALUES (?)", (name,)).lastrowid


def add_run(con, nonce, frame_id, label, items, platform="windows"):
    """items: (local_id, parent_local_id, kind, title, body, actor_label, actor_id)."""
    text_bytes = sum(len((i[3] or "") + (i[4] or "")) for i in items)
    run_id = con.execute(
        "INSERT INTO semantic_runs (input_fingerprint, parser_id, parser_version, "
        "schema_version, app_platform, app_id, app_executable, app_version, "
        "input_content_hash, status, item_count, text_bytes, parse_duration_us) "
        "VALUES (?, 'smoke.parser', '1.0.0', 1, ?, ?, ?, '1.0', ?, ?, ?, ?, 750)",
        (
            digest(nonce, "input:" + label),
            platform,
            label + ".exe",
            "C:\\Smoke\\" + label + ".exe",
            digest(nonce, "hash:" + label, 8),
            "handled" if items else "empty",
            len(items),
            text_bytes,
        ),
    ).lastrowid
    for order, (local, parent, kind, title, body, actor, actor_id) in enumerate(items):
        key = "%s/%s" % (label, local)
        item_id = con.execute(
            "INSERT INTO semantic_items (entity_fingerprint, version_fingerprint, kind, "
            "item_key, identity_quality, title, body, actor) "
            "VALUES (?, ?, ?, ?, 'derived', ?, ?, ?)",
            (digest(nonce, "entity:" + key), digest(nonce, "version:" + key), kind, key,
             title, body, actor),
        ).lastrowid
        con.execute(
            "INSERT INTO semantic_run_items (run_id, local_id, item_id, parent_local_id, "
            "sort_order) VALUES (?, ?, ?, ?, ?)",
            (run_id, local, item_id, parent, order),
        )
        if actor_id is not None:
            con.execute(
                "INSERT INTO semantic_item_actors (item_id, actor_id) VALUES (?, ?)",
                (item_id, actor_id),
            )
    con.execute("UPDATE frames SET semantic_run_id = ? WHERE id = ?", (run_id, frame_id))
    return run_id


def add_bulk(con, count, now):
    con.execute(
        "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < ? - 1) "
        "INSERT INTO frames (timestamp, app_name, window_name, device_name, full_text) "
        "SELECT strftime('%Y-%m-%dT%H:%M:%S', ?, '-' || (i * ?) || ' seconds') || '.000+00:00', "
        "'SmokeBulk' || (i % 9), 'Bulk window ' || (i % 40), ?, "
        "'bulk filler: a note about apples and amber archives, number ' || i FROM n",
        (count, now.strftime("%Y-%m-%d %H:%M:%S"), max(1, 30 * 86400 // max(count, 1)), DEVICE),
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("db", help="path to the scratch db.sqlite")
    parser.add_argument("--bulk-frames", type=int, default=0, metavar="N")
    args = parser.parse_args()
    if not os.path.exists(args.db):
        sys.exit("%s does not exist; start the engine once on the scratch data dir" % args.db)
    refuse_real_data_dir(args.db)

    con = sqlite3.connect(args.db, timeout=10, isolation_level=None)
    con.execute("PRAGMA foreign_keys = ON")
    check_schema(con)
    now = dt.datetime.now(dt.timezone.utc).replace(microsecond=0)
    nonce = now.isoformat()
    con.execute("BEGIN IMMEDIATE")
    try:
        chat = add_frame(con, now - dt.timedelta(minutes=4), "SmokeChat", "Launch room",
                         "chat window")
        notes = add_frame(con, now - dt.timedelta(minutes=3), "SmokeNotes", "Budget",
                          "notes window")
        empty = add_frame(con, now - dt.timedelta(minutes=2), "SmokeChat", "Idle room",
                          "nothing to parse")
        plain = add_frame(con, now - dt.timedelta(minutes=1), "SmokeEditor", "Draft",
                          "plain frame without a parser run")
        ada = add_actor(con, "Ada Smoke")
        bob = add_actor(con, "Bob Smoke")
        add_run(con, nonce, chat, "SmokeChat", [
            ("c", None, "conversation", "Launch room", None, None, None),
            ("m1", "c", "message", None, "ship the xylophone build on friday", "ada", ada),
            ("m2", "c", "message", None, "agreed, friday works", "bob", bob),
        ])
        add_run(con, nonce, notes, "SmokeNotes", [
            ("d", None, "document", "Budget", "xylophone budget for the quarter", None, None),
        ])
        add_run(con, nonce, empty, "SmokeIdle", [])
        if args.bulk_frames > 0:
            add_bulk(con, args.bulk_frames, now)
        con.execute("COMMIT")
    except BaseException:
        con.execute("ROLLBACK")
        raise
    finally:
        con.close()

    print("parsed frames: chat=%d notes=%d" % (chat, notes))
    print("not parsed:    empty_run=%d plain=%d" % (empty, plain))
    print("actors:        ada=%d bob=%d" % (ada, bob))
    if args.bulk_frames > 0:
        print("bulk frames:   %d" % args.bulk_frames)


if __name__ == "__main__":
    main()
