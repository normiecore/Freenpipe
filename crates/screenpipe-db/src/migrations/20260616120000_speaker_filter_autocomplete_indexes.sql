-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- Prefix autocomplete over speaker names and tag names (people filter, tag
-- picker) compares case-insensitively; give both lookups a NOCASE index.
CREATE INDEX IF NOT EXISTS idx_speakers_hallucination_name
  ON speakers(hallucination, name COLLATE NOCASE);
CREATE INDEX IF NOT EXISTS idx_tags_name_nocase
  ON tags(name COLLATE NOCASE);
