-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- Snapshot compaction and media retention look for frames that still point at
-- an uncompacted JPEG, oldest first per device. Keeping only those rows in the
-- index keeps it small once most snapshots have been compacted.
CREATE INDEX IF NOT EXISTS idx_frames_snapshot_pending
    ON frames(device_name, timestamp)
    WHERE snapshot_path IS NOT NULL;
