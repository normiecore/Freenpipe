-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- Watermark for the browser URL copy on each frame. browser_url is a frames_fts
-- column, so a scrubbed URL is re-indexed by the frames_au trigger.
ALTER TABLE frames ADD COLUMN browser_url_redacted_at INTEGER;

CREATE INDEX IF NOT EXISTS idx_frames_browser_url_redacted_at
    ON frames(browser_url_redacted_at);
