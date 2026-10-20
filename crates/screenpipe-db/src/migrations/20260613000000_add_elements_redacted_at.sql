-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- The PII worker rewrites elements.text in place and stamps the time here.
-- NULL means the row has not been processed yet (the worker's backlog).
-- elements_fts follows the rewrite through the existing update trigger.
ALTER TABLE elements ADD COLUMN redacted_at INTEGER;

CREATE INDEX IF NOT EXISTS idx_elements_redacted_at ON elements(redacted_at);
