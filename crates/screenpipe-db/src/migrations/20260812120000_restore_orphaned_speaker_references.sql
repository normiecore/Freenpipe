-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- Rows in five tables can point at speaker ids whose speakers row is gone.
-- Recreate those speakers as nameless rows instead of clearing the references,
-- so voices that were grouped together stay grouped. speakers is
-- AUTOINCREMENT, so a missing id was never handed out again; inserting it back
-- cannot collide with a different speaker, and running this twice adds nothing.
INSERT INTO speakers (id, name)
SELECT DISTINCT speaker_id, ''
FROM (
    SELECT speaker_id FROM audio_transcriptions
    WHERE speaker_id IS NOT NULL
    UNION
    SELECT speaker_id FROM meeting_transcript_segments
    WHERE speaker_id IS NOT NULL
    UNION
    SELECT speaker_id FROM speaker_embeddings
    WHERE speaker_id IS NOT NULL
    UNION
    SELECT speaker_id FROM diarization_segments
    WHERE speaker_id IS NOT NULL
    UNION
    SELECT speaker_id FROM speaker_identity_evidence
    WHERE speaker_id IS NOT NULL
)
WHERE speaker_id NOT IN (SELECT id FROM speakers);
