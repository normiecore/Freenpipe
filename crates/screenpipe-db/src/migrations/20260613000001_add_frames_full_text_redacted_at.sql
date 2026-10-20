-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- Processing watermark for frames.full_text, the text that search indexes.
-- The index is created as a plain index here; 20260709000000 turns it into a
-- partial index over the unprocessed rows.
ALTER TABLE frames ADD COLUMN full_text_redacted_at INTEGER;

CREATE INDEX IF NOT EXISTS idx_frames_full_text_redacted_at ON frames(full_text_redacted_at);
