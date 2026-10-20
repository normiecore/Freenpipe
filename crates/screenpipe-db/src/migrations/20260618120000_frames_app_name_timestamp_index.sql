-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- Search caps how many hits a single app may contribute by ranking frames per
-- app in time order. This index serves that per-app window and plain
-- app-name filters over a time range.
CREATE INDEX IF NOT EXISTS idx_frames_app_name_timestamp
    ON frames(app_name, timestamp);
