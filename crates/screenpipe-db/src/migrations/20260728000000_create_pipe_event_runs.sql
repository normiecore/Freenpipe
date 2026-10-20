-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- Idempotency for event-triggered pipe runs. The scheduler claims
-- (pipe, event, key) with INSERT OR IGNORE before it starts a run; when no row
-- was inserted the event instance has already been handled (for example a
-- meeting_ended event fired again for a rejoined meeting). Old claims are
-- pruned by claimed_at.
CREATE TABLE IF NOT EXISTS pipe_event_runs (
    pipe_name  TEXT NOT NULL,
    event_name TEXT NOT NULL,
    event_key  TEXT NOT NULL,
    claimed_at TEXT NOT NULL,
    PRIMARY KEY (pipe_name, event_name, event_key)
);

CREATE INDEX IF NOT EXISTS idx_pipe_event_runs_claimed_at ON pipe_event_runs(claimed_at);

-- Which event instance produced an execution (event name and instance key).
ALTER TABLE pipe_executions ADD COLUMN trigger_event TEXT;
ALTER TABLE pipe_executions ADD COLUMN trigger_key TEXT;
