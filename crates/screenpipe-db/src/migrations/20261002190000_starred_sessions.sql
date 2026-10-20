-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- Time ranges the user starred. A session covers [start, end); hd_requested
-- marks ranges whose media should be kept at full quality, and revision counts
-- edits to the range.
CREATE TABLE starred_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    start TEXT NOT NULL,
    end TEXT NOT NULL,
    hd_requested INTEGER NOT NULL DEFAULT 0,
    revision INTEGER NOT NULL DEFAULT 1,
    CHECK (end > start)
);

CREATE INDEX idx_starred_sessions_start_end ON starred_sessions(start, end);
