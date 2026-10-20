-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- The redaction worker only ever asks for frames it has not processed yet
-- (watermark IS NULL). Full indexes over these watermarks grow with every
-- processed row; rebuild all six as partial indexes over the backlog instead.
DROP INDEX IF EXISTS idx_frames_accessibility_redacted_at;
CREATE INDEX IF NOT EXISTS idx_frames_accessibility_redacted_at
    ON frames(accessibility_redacted_at)
    WHERE accessibility_redacted_at IS NULL;

DROP INDEX IF EXISTS idx_frames_full_text_redacted_at;
CREATE INDEX IF NOT EXISTS idx_frames_full_text_redacted_at
    ON frames(full_text_redacted_at)
    WHERE full_text_redacted_at IS NULL;

DROP INDEX IF EXISTS idx_frames_accessibility_tree_redacted_at;
CREATE INDEX IF NOT EXISTS idx_frames_accessibility_tree_redacted_at
    ON frames(accessibility_tree_redacted_at)
    WHERE accessibility_tree_redacted_at IS NULL;

DROP INDEX IF EXISTS idx_frames_window_name_redacted_at;
CREATE INDEX IF NOT EXISTS idx_frames_window_name_redacted_at
    ON frames(window_name_redacted_at)
    WHERE window_name_redacted_at IS NULL;

DROP INDEX IF EXISTS idx_frames_browser_url_redacted_at;
CREATE INDEX IF NOT EXISTS idx_frames_browser_url_redacted_at
    ON frames(browser_url_redacted_at)
    WHERE browser_url_redacted_at IS NULL;

DROP INDEX IF EXISTS idx_frames_text_json_redacted_at;
CREATE INDEX IF NOT EXISTS idx_frames_text_json_redacted_at
    ON frames(text_json_redacted_at)
    WHERE text_json_redacted_at IS NULL;
