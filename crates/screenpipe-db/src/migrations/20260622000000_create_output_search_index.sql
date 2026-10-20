-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- Full-text search over stored outputs. output_search_fts is a standalone FTS5
-- table whose rowid is outputs.id; output_search_index remembers which content
-- hash was indexed so unchanged files are not indexed twice.
CREATE TABLE IF NOT EXISTS output_search_index (
    output_id INTEGER PRIMARY KEY REFERENCES outputs(id) ON DELETE CASCADE,
    content_hash TEXT NOT NULL,
    bytes_indexed INTEGER NOT NULL DEFAULT 0,
    indexed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE VIRTUAL TABLE IF NOT EXISTS output_search_fts USING fts5(
    title,
    body,
    tokenize='unicode61'
);
