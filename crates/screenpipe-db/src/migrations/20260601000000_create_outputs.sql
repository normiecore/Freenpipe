-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- Registry of files that pipes, chat and agents produced and that were copied
-- into the data dir's outputs/ folder. One row per stored file.
CREATE TABLE IF NOT EXISTS outputs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source TEXT NOT NULL,
    source_type TEXT NOT NULL DEFAULT 'pipe',
    title TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'text',
    original_path TEXT,
    output_path TEXT NOT NULL,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    preview TEXT,
    metadata TEXT DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_outputs_source ON outputs(source);
CREATE INDEX IF NOT EXISTS idx_outputs_source_type ON outputs(source_type);
CREATE INDEX IF NOT EXISTS idx_outputs_created_at ON outputs(created_at DESC);
CREATE UNIQUE INDEX IF NOT EXISTS idx_outputs_output_path ON outputs(output_path);
