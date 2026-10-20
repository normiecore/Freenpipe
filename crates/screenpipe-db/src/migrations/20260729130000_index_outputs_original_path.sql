-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- Look up the newest output for a given source path quickly; plain files that
-- have no original path are left out of the index.
CREATE INDEX IF NOT EXISTS idx_outputs_source_type_original_path_id
ON outputs(source_type, original_path, id DESC)
WHERE original_path IS NOT NULL;
