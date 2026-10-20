-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- Meeting timeline queries filter and sort live transcript segments by
-- julianday(captured_at); an expression index on exactly that expression lets
-- them use an index range scan. Queries must spell the expression the same way.
CREATE INDEX IF NOT EXISTS idx_meeting_transcript_segments_captured_at_julianday
    ON meeting_transcript_segments(julianday(captured_at));
