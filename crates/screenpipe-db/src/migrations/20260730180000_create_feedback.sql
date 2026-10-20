-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- Local thumbs-up / thumbs-down ratings on AI output. One row per rating
-- author and device for a given target version; rows are upserted by id.
CREATE TABLE IF NOT EXISTS feedback (
    id TEXT PRIMARY KEY,
    target_kind TEXT NOT NULL,
    target_id TEXT NOT NULL,
    target_version TEXT NOT NULL DEFAULT '',
    producer_ref TEXT,
    actor_id TEXT NOT NULL,
    rating TEXT NOT NULL CHECK (rating IN ('up', 'down')),
    comment TEXT,
    snapshot TEXT,
    context TEXT NOT NULL DEFAULT '{}',
    device_id TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (target_kind, target_id, target_version, actor_id, device_id)
);

CREATE INDEX IF NOT EXISTS idx_feedback_target
    ON feedback (target_kind, target_id, target_version);
CREATE INDEX IF NOT EXISTS idx_feedback_producer_updated
    ON feedback (producer_ref, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_feedback_rating_updated
    ON feedback (rating, updated_at DESC);
