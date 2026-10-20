-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- Retire the ocr_text side table. Every frame row already carries its
-- searchable text in frames.full_text; the only thing ocr_text still held on
-- its own was the per-word bounding boxes (text_json). Those move onto the frame
-- row, older rows are folded into their frames, and the table, its leftover
-- triggers and its FTS shadow are dropped. This also replaces the background
-- ocr_text-to-frames copier the base shipped but never started.
--
-- On a large database the backfill touches many frames (and, through the
-- frames_au trigger, frames_fts), so the engine logs a notice before it runs.

ALTER TABLE frames ADD COLUMN text_json TEXT DEFAULT NULL;

-- Fold ocr_text into frames. Each statement only fills a frame value that is
-- missing, and only from an ocr_text row of the same frame that has a value.
-- full_text_redacted_at is deliberately left NULL so the redaction worker
-- processes the folded text again; ocr_text.redacted_at is not carried over.

UPDATE frames
SET app_name = (
    SELECT ot.app_name FROM ocr_text ot
    WHERE ot.frame_id = frames.id AND ot.app_name IS NOT NULL AND ot.app_name != ''
    LIMIT 1
)
WHERE (app_name IS NULL OR app_name = '')
  AND EXISTS (
    SELECT 1 FROM ocr_text ot
    WHERE ot.frame_id = frames.id AND ot.app_name IS NOT NULL AND ot.app_name != ''
  );

UPDATE frames
SET window_name = (
    SELECT ot.window_name FROM ocr_text ot
    WHERE ot.frame_id = frames.id AND ot.window_name IS NOT NULL AND ot.window_name != ''
    LIMIT 1
)
WHERE window_name IS NULL
  AND EXISTS (
    SELECT 1 FROM ocr_text ot
    WHERE ot.frame_id = frames.id AND ot.window_name IS NOT NULL AND ot.window_name != ''
  );

UPDATE frames
SET focused = (
    SELECT ot.focused FROM ocr_text ot
    WHERE ot.frame_id = frames.id AND ot.focused IS NOT NULL
    LIMIT 1
)
WHERE focused IS NULL
  AND EXISTS (
    SELECT 1 FROM ocr_text ot
    WHERE ot.frame_id = frames.id AND ot.focused IS NOT NULL
  );

UPDATE frames
SET full_text = (
    SELECT ot.text FROM ocr_text ot
    WHERE ot.frame_id = frames.id AND ot.text IS NOT NULL AND ot.text != ''
    LIMIT 1
)
WHERE (full_text IS NULL OR full_text = '')
  AND EXISTS (
    SELECT 1 FROM ocr_text ot
    WHERE ot.frame_id = frames.id AND ot.text IS NOT NULL AND ot.text != ''
  );

UPDATE frames
SET text_json = (
    SELECT ot.text_json FROM ocr_text ot
    WHERE ot.frame_id = frames.id AND ot.text_json IS NOT NULL AND ot.text_json != ''
    LIMIT 1
)
WHERE text_json IS NULL
  AND EXISTS (
    SELECT 1 FROM ocr_text ot
    WHERE ot.frame_id = frames.id AND ot.text_json IS NOT NULL AND ot.text_json != ''
  );

-- Triggers that may survive from older schemas, then the FTS shadow and the
-- table itself (its indexes go with it). The OCR engine name has no new home:
-- search results report it as an empty string from now on.
DROP TRIGGER IF EXISTS ocr_text_ai;
DROP TRIGGER IF EXISTS ocr_text_au;
DROP TRIGGER IF EXISTS ocr_text_ad;
DROP TRIGGER IF EXISTS ocr_text_update;
DROP TRIGGER IF EXISTS ocr_text_delete;
DROP TABLE IF EXISTS ocr_text_fts;
DROP TABLE IF EXISTS ocr_text;
