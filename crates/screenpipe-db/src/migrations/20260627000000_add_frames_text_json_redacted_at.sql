-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- Watermark for frames.text_json (OCR word boxes). Only the text of each box is
-- rewritten when it is scrubbed; the geometry stays as captured.
ALTER TABLE frames ADD COLUMN text_json_redacted_at INTEGER;

CREATE INDEX IF NOT EXISTS idx_frames_text_json_redacted_at
    ON frames(text_json_redacted_at);
