-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- A calendar event can be bound to at most one meeting. Writers treat a unique
-- violation on this index as "already attached elsewhere", not as a failure.
ALTER TABLE meetings ADD COLUMN calendar_event_id TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_meetings_calendar_event_id
    ON meetings (calendar_event_id)
    WHERE calendar_event_id IS NOT NULL AND calendar_event_id != '';
