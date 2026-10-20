-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- Watermarks for two more per-frame text copies that the PII worker scrubs by
-- reusing the detections it made on full_text: the accessibility tree JSON and
-- the window title. Rows redacted before these columns existed keep their raw
-- copies; the scrub only applies going forward.
ALTER TABLE frames ADD COLUMN accessibility_tree_redacted_at INTEGER;
ALTER TABLE frames ADD COLUMN window_name_redacted_at INTEGER;

CREATE INDEX IF NOT EXISTS idx_frames_accessibility_tree_redacted_at
    ON frames(accessibility_tree_redacted_at);
CREATE INDEX IF NOT EXISTS idx_frames_window_name_redacted_at
    ON frames(window_name_redacted_at);
