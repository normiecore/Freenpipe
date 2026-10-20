-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- Artifact envelope metadata. All three stay NULL for plain files; an artifact
-- row carries its kind, a stable artifact id and a version that only grows.
ALTER TABLE outputs ADD COLUMN saf_kind TEXT;
ALTER TABLE outputs ADD COLUMN artifact_id TEXT;
ALTER TABLE outputs ADD COLUMN saf_version INTEGER;
