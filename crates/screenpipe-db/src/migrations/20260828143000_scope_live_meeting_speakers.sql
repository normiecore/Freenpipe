-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- A live transcription provider's speaker labels only mean something inside the
-- streaming connection that produced them. Record which stream a segment came
-- from (rows written before this change are 'legacy') and keep the provider's
-- label as session_speaker_id. speaker_id remains the global, cross-meeting
-- identity resolved later from voice embeddings.
ALTER TABLE meeting_transcript_segments
    ADD COLUMN stream_id TEXT NOT NULL DEFAULT 'legacy';
ALTER TABLE meeting_transcript_segments
    ADD COLUMN session_speaker_id TEXT;

CREATE INDEX IF NOT EXISTS idx_meeting_transcript_segments_session_speaker
    ON meeting_transcript_segments(meeting_id, session_speaker_id, captured_at);
