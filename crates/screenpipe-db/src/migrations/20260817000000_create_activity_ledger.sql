-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- Activity ledger: what the user was working on, as a tree of tasks with time
-- intervals, the notable actions inside each interval and the capture rows
-- (frames, UI events, audio) that back them up.
--
-- Lifetime is driven by evidence. Deleting a capture row removes the actions
-- and evidence that cite it; an interval goes when its last evidence row goes;
-- a task that is not user-locked goes once it has no intervals and no child
-- tasks, and that check then repeats for its parent.

CREATE TABLE IF NOT EXISTS activity_tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_key TEXT NOT NULL UNIQUE,
    parent_task_id INTEGER REFERENCES activity_tasks(id) ON DELETE SET NULL,
    kind TEXT NOT NULL CHECK (kind IN ('category', 'task', 'unobserved')),
    title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 256),
    app_name TEXT,
    confidence REAL NOT NULL CHECK (confidence BETWEEN 0.0 AND 1.0),
    producer TEXT NOT NULL,
    user_locked INTEGER NOT NULL DEFAULT 0 CHECK (user_locked IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_activity_tasks_parent ON activity_tasks(parent_task_id, id);
CREATE INDEX IF NOT EXISTS idx_activity_tasks_app ON activity_tasks(app_name, id);

CREATE TABLE IF NOT EXISTS activity_intervals (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    interval_key TEXT NOT NULL UNIQUE,
    task_id INTEGER NOT NULL REFERENCES activity_tasks(id) ON DELETE CASCADE,
    start_at TEXT NOT NULL,
    end_at TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('provisional', 'final')),
    confidence REAL NOT NULL CHECK (confidence BETWEEN 0.0 AND 1.0),
    producer TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (julianday(end_at) > julianday(start_at))
);

CREATE INDEX IF NOT EXISTS idx_activity_intervals_range
    ON activity_intervals(start_at, end_at, id);
CREATE INDEX IF NOT EXISTS idx_activity_intervals_task
    ON activity_intervals(task_id, start_at, id);

CREATE TABLE IF NOT EXISTS activity_actions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    action_key TEXT NOT NULL UNIQUE,
    interval_id INTEGER NOT NULL REFERENCES activity_intervals(id) ON DELETE CASCADE,
    occurred_at TEXT NOT NULL,
    action_type TEXT NOT NULL,
    summary TEXT NOT NULL CHECK (length(summary) BETWEEN 1 AND 512),
    app_name TEXT,
    confidence REAL NOT NULL CHECK (confidence BETWEEN 0.0 AND 1.0),
    source_type TEXT NOT NULL CHECK (source_type IN ('frame', 'ui_event', 'audio')),
    source_id INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_activity_actions_interval
    ON activity_actions(interval_id, occurred_at, id);
CREATE INDEX IF NOT EXISTS idx_activity_actions_source
    ON activity_actions(source_type, source_id);

CREATE TABLE IF NOT EXISTS activity_evidence (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    interval_id INTEGER NOT NULL REFERENCES activity_intervals(id) ON DELETE CASCADE,
    action_id INTEGER REFERENCES activity_actions(id) ON DELETE CASCADE,
    source_type TEXT NOT NULL CHECK (source_type IN ('frame', 'ui_event', 'audio')),
    source_id INTEGER NOT NULL,
    occurred_at TEXT NOT NULL,
    UNIQUE(interval_id, source_type, source_id)
);

CREATE INDEX IF NOT EXISTS idx_activity_evidence_interval
    ON activity_evidence(interval_id, occurred_at, id);
CREATE INDEX IF NOT EXISTS idx_activity_evidence_source
    ON activity_evidence(source_type, source_id);

-- Per-producer reconciliation cursor.
CREATE TABLE IF NOT EXISTS activity_ledger_state (
    producer TEXT PRIMARY KEY,
    last_reconciled_at TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- Capture rows going away take their ledger references with them.
CREATE TRIGGER IF NOT EXISTS activity_frames_ad AFTER DELETE ON frames BEGIN
    DELETE FROM activity_actions
      WHERE source_type = 'frame' AND source_id = OLD.id;
    DELETE FROM activity_evidence
      WHERE source_type = 'frame' AND source_id = OLD.id;
END;

CREATE TRIGGER IF NOT EXISTS activity_ui_events_ad AFTER DELETE ON ui_events BEGIN
    DELETE FROM activity_actions
      WHERE source_type = 'ui_event' AND source_id = OLD.id;
    DELETE FROM activity_evidence
      WHERE source_type = 'ui_event' AND source_id = OLD.id;
END;

CREATE TRIGGER IF NOT EXISTS activity_audio_ad AFTER DELETE ON audio_transcriptions BEGIN
    DELETE FROM activity_actions
      WHERE source_type = 'audio' AND source_id = OLD.id;
    DELETE FROM activity_evidence
      WHERE source_type = 'audio' AND source_id = OLD.id;
END;

-- An interval without evidence is dropped.
CREATE TRIGGER IF NOT EXISTS activity_evidence_ad AFTER DELETE ON activity_evidence BEGIN
    DELETE FROM activity_intervals
      WHERE id = OLD.interval_id
        AND NOT EXISTS (
            SELECT 1 FROM activity_evidence
            WHERE interval_id = OLD.interval_id
        );
END;

-- An unlocked task with no intervals and no children is dropped.
CREATE TRIGGER IF NOT EXISTS activity_intervals_ad AFTER DELETE ON activity_intervals BEGIN
    DELETE FROM activity_tasks
      WHERE id = OLD.task_id
        AND user_locked = 0
        AND NOT EXISTS (
            SELECT 1 FROM activity_intervals WHERE task_id = OLD.task_id
        )
        AND NOT EXISTS (
            SELECT 1 FROM activity_tasks WHERE parent_task_id = OLD.task_id
        );
END;

-- The same check walks up to the parent when a child task is dropped.
CREATE TRIGGER IF NOT EXISTS activity_tasks_ad AFTER DELETE ON activity_tasks
WHEN OLD.parent_task_id IS NOT NULL BEGIN
    DELETE FROM activity_tasks
      WHERE id = OLD.parent_task_id
        AND user_locked = 0
        AND NOT EXISTS (
            SELECT 1 FROM activity_intervals
            WHERE task_id = OLD.parent_task_id
        )
        AND NOT EXISTS (
            SELECT 1 FROM activity_tasks
            WHERE parent_task_id = OLD.parent_task_id
        );
END;
