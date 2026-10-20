-- Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
-- https://github.com/normiecore/Freenpipe
--
-- Schema v2.7.104 interop migration: the version, file name and DDL match the
-- schema screenpipe v2.7.104 writes; the comments are our own. Freenpipe pins
-- the upstream checksum for this version (src/schema_migrations.rs), so the
-- text here can differ without causing a checksum mismatch on either side.

-- Structured context parsed out of captured screens (conversations, messages,
-- documents, tasks, calendar events, pages).
--
-- semantic_runs      one immutable parse of one input, shared by every frame
--                    whose input has the same 32-byte fingerprint
-- semantic_items     canonical items, unique per version fingerprint and
--                    grouped into entities by entity fingerprint
-- semantic_run_items which items a run produced, in run-local order, with the
--                    run-local parent and the source node ids
-- semantic_items_fts external-content FTS over the item text, kept in step by
--                    the three triggers below
-- frames.semantic_run_id points a frame at the run parsed from it.

CREATE TABLE semantic_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    input_fingerprint BLOB NOT NULL UNIQUE
        CHECK (length(input_fingerprint) = 32),
    parser_id TEXT NOT NULL,
    parser_version TEXT NOT NULL,
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    app_platform TEXT NOT NULL
        CHECK (app_platform IN ('macos', 'windows', 'linux')),
    app_id TEXT,
    app_executable TEXT,
    app_version TEXT,
    input_content_hash BLOB NOT NULL
        CHECK (length(input_content_hash) = 8),
    status TEXT NOT NULL
        CHECK (status IN ('handled', 'empty')),
    item_count INTEGER NOT NULL DEFAULT 0 CHECK (item_count >= 0),
    text_bytes INTEGER NOT NULL DEFAULT 0 CHECK (text_bytes >= 0),
    parse_duration_us INTEGER NOT NULL DEFAULT 0 CHECK (parse_duration_us >= 0),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_semantic_runs_parser
    ON semantic_runs(parser_id, parser_version, schema_version);

CREATE TABLE semantic_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_fingerprint BLOB NOT NULL CHECK (length(entity_fingerprint) = 32),
    version_fingerprint BLOB NOT NULL UNIQUE
        CHECK (length(version_fingerprint) = 32),
    kind TEXT NOT NULL
        CHECK (kind IN ('conversation', 'message', 'document', 'task', 'calendar_event', 'page')),
    item_key TEXT NOT NULL,
    identity_quality TEXT NOT NULL
        CHECK (identity_quality IN ('stable', 'derived', 'ephemeral')),
    title TEXT,
    body TEXT,
    actor TEXT,
    occurred_at TEXT,
    timestamp_precision TEXT,
    status TEXT,
    metadata_json TEXT NOT NULL DEFAULT '{}'
        CHECK (json_valid(metadata_json)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_semantic_items_entity
    ON semantic_items(entity_fingerprint);
CREATE INDEX idx_semantic_items_kind_time
    ON semantic_items(kind, occurred_at);

CREATE TABLE semantic_run_items (
    run_id INTEGER NOT NULL,
    local_id TEXT NOT NULL,
    item_id INTEGER NOT NULL,
    parent_local_id TEXT,
    sort_order INTEGER NOT NULL CHECK (sort_order >= 0),
    source_node_ids BLOB NOT NULL DEFAULT X'',
    PRIMARY KEY (run_id, local_id),
    UNIQUE (run_id, sort_order),
    FOREIGN KEY (run_id) REFERENCES semantic_runs(id) ON DELETE CASCADE,
    FOREIGN KEY (item_id) REFERENCES semantic_items(id) ON DELETE RESTRICT,
    FOREIGN KEY (run_id, parent_local_id)
        REFERENCES semantic_run_items(run_id, local_id)
        DEFERRABLE INITIALLY DEFERRED
);

CREATE INDEX idx_semantic_run_items_item
    ON semantic_run_items(item_id);

CREATE VIRTUAL TABLE semantic_items_fts USING fts5(
    title,
    body,
    actor,
    status,
    metadata_json,
    content='semantic_items',
    content_rowid='id',
    tokenize='unicode61'
);

CREATE TRIGGER semantic_items_ai AFTER INSERT ON semantic_items BEGIN
    INSERT INTO semantic_items_fts(rowid, title, body, actor, status, metadata_json)
    VALUES (new.id, new.title, new.body, new.actor, new.status, new.metadata_json);
END;

CREATE TRIGGER semantic_items_ad AFTER DELETE ON semantic_items BEGIN
    INSERT INTO semantic_items_fts(
        semantic_items_fts, rowid, title, body, actor, status, metadata_json
    ) VALUES (
        'delete', old.id, old.title, old.body, old.actor, old.status, old.metadata_json
    );
END;

CREATE TRIGGER semantic_items_au AFTER UPDATE ON semantic_items BEGIN
    INSERT INTO semantic_items_fts(
        semantic_items_fts, rowid, title, body, actor, status, metadata_json
    ) VALUES (
        'delete', old.id, old.title, old.body, old.actor, old.status, old.metadata_json
    );
    INSERT INTO semantic_items_fts(rowid, title, body, actor, status, metadata_json)
    VALUES (new.id, new.title, new.body, new.actor, new.status, new.metadata_json);
END;

ALTER TABLE frames ADD COLUMN semantic_run_id INTEGER
    REFERENCES semantic_runs(id) ON DELETE SET NULL;

CREATE INDEX idx_frames_semantic_run_id
    ON frames(semantic_run_id)
    WHERE semantic_run_id IS NOT NULL;
