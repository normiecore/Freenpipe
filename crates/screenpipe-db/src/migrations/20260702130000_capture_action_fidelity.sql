-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- element_ancestors: compact JSON array (root first) of {"role", "name"}
-- objects for the container chain of the element a UI event hit.
ALTER TABLE ui_events ADD COLUMN element_ancestors TEXT;

-- display_layout: one row per observed monitor arrangement, written when
-- capture starts (reason 'startup') and whenever the layout changes
-- (reason 'change'). layout_json is a JSON array of displays sorted by id.
CREATE TABLE IF NOT EXISTS display_layout (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    reason TEXT NOT NULL DEFAULT 'change',
    layout_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_display_layout_timestamp
    ON display_layout(timestamp);
