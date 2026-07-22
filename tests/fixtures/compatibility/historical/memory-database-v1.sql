PRAGMA user_version = 1;

CREATE TABLE memories (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL,
    source TEXT NOT NULL,
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    tags_json TEXT NOT NULL,
    source_run_id TEXT UNIQUE,
    source_integrity_hash TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    last_accessed_at TEXT,
    access_count INTEGER NOT NULL DEFAULT 0,
    active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1))
) STRICT;

CREATE INDEX memories_active_updated
    ON memories (active, updated_at DESC);

INSERT INTO memories (
    id, kind, source, title, content, tags_json, created_at, updated_at
) VALUES (
    '018f53d2-a0d8-7c6a-8e22-6b6a4b0b0f54',
    'decision',
    'user',
    'Historical decision',
    'Preserve this schema-one memory during migration.',
    '["compatibility"]',
    '2025-01-01T00:00:00Z',
    '2025-01-01T00:00:00Z'
);
