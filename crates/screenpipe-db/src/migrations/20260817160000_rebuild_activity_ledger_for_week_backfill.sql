-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- Throw away what the built-in reconciler (producer deterministic-v1) has
-- written so far, together with its cursor, so it rebuilds the ledger from
-- scratch over the longer backfill window. Tasks the user locked are kept.
DELETE FROM activity_intervals
WHERE producer = 'deterministic-v1';
DELETE FROM activity_tasks
WHERE producer = 'deterministic-v1'
  AND user_locked = 0;
DELETE FROM activity_ledger_state
WHERE producer = 'deterministic-v1';
