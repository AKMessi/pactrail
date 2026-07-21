PRAGMA user_version = 1;

CREATE TABLE events (
    run_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    schema_version INTEGER NOT NULL,
    timestamp TEXT NOT NULL,
    previous_hash TEXT NOT NULL,
    event_json TEXT NOT NULL,
    hash TEXT NOT NULL,
    PRIMARY KEY (run_id, sequence),
    UNIQUE (run_id, hash)
) STRICT;

CREATE INDEX events_by_run ON events (run_id, sequence);
