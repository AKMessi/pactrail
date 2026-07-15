use std::path::Path;

use pactrail_core::{
    EVENT_SCHEMA_VERSION, EventEnvelope, EventHash, RunEvent, RunId, RunSnapshot, StateError,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use thiserror::Error;
use time::OffsetDateTime;

/// SQLite-backed append-only event store.
#[derive(Debug)]
pub struct EventStore {
    connection: Connection,
}

impl EventStore {
    /// Opens a durable event database and applies supported migrations.
    ///
    /// # Errors
    ///
    /// Returns a database error when the file cannot be opened or initialized.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let connection = Connection::open(path).map_err(StoreError::Database)?;
        Self::initialize(connection)
    }

    /// Opens an event database for tests and ephemeral operation.
    ///
    /// # Errors
    ///
    /// Returns a database error when `SQLite` initialization fails.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let connection = Connection::open_in_memory().map_err(StoreError::Database)?;
        Self::initialize(connection)
    }

    fn initialize(connection: Connection) -> Result<Self, StoreError> {
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .and_then(|()| connection.pragma_update(None, "journal_mode", "WAL"))
            .and_then(|()| connection.pragma_update(None, "synchronous", "FULL"))
            .map_err(StoreError::Database)?;
        let database_version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(StoreError::Database)?;
        if !matches!(database_version, 0 | 1) {
            return Err(StoreError::UnsupportedDatabaseSchema(database_version));
        }
        connection
            .execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE IF NOT EXISTS events (
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
                 CREATE INDEX IF NOT EXISTS events_by_run
                    ON events (run_id, sequence);
                 COMMIT;",
            )
            .map_err(StoreError::Database)?;
        if database_version == 0 {
            connection
                .pragma_update(None, "user_version", 1)
                .map_err(StoreError::Database)?;
        }
        Ok(Self { connection })
    }

    /// Appends one event if `expected_sequence` still matches the durable head.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Concurrency`] for a stale writer, or another store
    /// error when serialization or the atomic database transaction fails.
    pub fn append(
        &mut self,
        run_id: RunId,
        expected_sequence: u64,
        event: RunEvent,
    ) -> Result<EventEnvelope, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Database)?;
        let head: Option<(i64, String)> = transaction
            .query_row(
                "SELECT sequence, hash FROM events
                 WHERE run_id = ?1 ORDER BY sequence DESC LIMIT 1",
                [run_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(StoreError::Database)?;
        let actual_sequence = match head.as_ref() {
            Some((sequence, _)) => {
                u64::try_from(*sequence).map_err(|_| StoreError::InvalidSequence(*sequence))? + 1
            }
            None => 0,
        };
        if actual_sequence != expected_sequence {
            return Err(StoreError::Concurrency {
                expected: expected_sequence,
                actual: actual_sequence,
            });
        }
        let previous_hash = head.map_or_else(EventHash::genesis, |(_, hash)| EventHash(hash));
        let timestamp = OffsetDateTime::now_utc();
        let envelope =
            EventEnvelope::new(run_id, expected_sequence, timestamp, previous_hash, event)
                .map_err(StoreError::Serialization)?;
        let event_json =
            serde_json::to_string(&envelope.event).map_err(StoreError::Serialization)?;
        let sequence = i64::try_from(envelope.sequence)
            .map_err(|_| StoreError::SequenceOverflow(envelope.sequence))?;
        transaction
            .execute(
                "INSERT INTO events
                 (run_id, sequence, schema_version, timestamp, previous_hash, event_json, hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    envelope.run_id.to_string(),
                    sequence,
                    i64::from(envelope.schema_version),
                    envelope
                        .timestamp
                        .format(&time::format_description::well_known::Rfc3339)
                        .map_err(StoreError::Time)?,
                    envelope.previous_hash.0,
                    event_json,
                    envelope.hash.0,
                ],
            )
            .map_err(StoreError::Database)?;
        transaction.commit().map_err(StoreError::Database)?;
        Ok(envelope)
    }

    /// Loads and integrity-checks all events for a run.
    ///
    /// # Errors
    ///
    /// Returns an error if stored data cannot be decoded or violates the run's
    /// sequence, hash-chain, contract, or lifecycle invariants.
    pub fn load(&self, run_id: RunId) -> Result<Vec<EventEnvelope>, StoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT sequence, schema_version, timestamp, previous_hash, event_json, hash
                 FROM events WHERE run_id = ?1 ORDER BY sequence ASC",
            )
            .map_err(StoreError::Database)?;
        let rows = statement
            .query_map([run_id.to_string()], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })
            .map_err(StoreError::Database)?;

        let mut events = Vec::new();
        let mut snapshot = RunSnapshot::new(run_id);
        for row in rows {
            let (sequence, schema_version, timestamp, previous_hash, event_json, hash) =
                row.map_err(StoreError::Database)?;
            let sequence =
                u64::try_from(sequence).map_err(|_| StoreError::InvalidSequence(sequence))?;
            let schema_version = u32::try_from(schema_version)
                .map_err(|_| StoreError::InvalidSchema(schema_version))?;
            let timestamp =
                OffsetDateTime::parse(&timestamp, &time::format_description::well_known::Rfc3339)
                    .map_err(StoreError::TimeParse)?;
            let event = serde_json::from_str(&event_json).map_err(StoreError::Serialization)?;
            let envelope = EventEnvelope {
                schema_version,
                run_id,
                sequence,
                timestamp,
                previous_hash: EventHash(previous_hash),
                event,
                hash: EventHash(hash),
            };
            snapshot.apply(&envelope).map_err(StoreError::State)?;
            events.push(envelope);
        }
        Ok(events)
    }

    /// Lists distinct durable run identifiers from newest to oldest.
    ///
    /// # Errors
    ///
    /// Returns an error when the database cannot be queried or contains a
    /// malformed run identifier.
    pub fn list_run_ids(&self) -> Result<Vec<RunId>, StoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT run_id FROM events
                 GROUP BY run_id
                 ORDER BY MIN(timestamp) DESC, run_id DESC",
            )
            .map_err(StoreError::Database)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(StoreError::Database)?;
        let mut run_ids = Vec::new();
        for row in rows {
            let value = row.map_err(StoreError::Database)?;
            let run_id = value.parse().map_err(|_| StoreError::InvalidRunId(value))?;
            run_ids.push(run_id);
        }
        Ok(run_ids)
    }

    /// Reconstructs the current deterministic projection for one run.
    ///
    /// # Errors
    ///
    /// Returns an error when any durable event fails decoding or validation.
    pub fn snapshot(&self, run_id: RunId) -> Result<RunSnapshot, StoreError> {
        let mut snapshot = RunSnapshot::new(run_id);
        for event in self.load(run_id)? {
            snapshot.apply(&event).map_err(StoreError::State)?;
        }
        Ok(snapshot)
    }
}

/// Durable event store failure.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("event database error: {0}")]
    Database(rusqlite::Error),
    #[error("event serialization failed: {0}")]
    Serialization(serde_json::Error),
    #[error("event timestamp formatting failed: {0}")]
    Time(time::error::Format),
    #[error("event timestamp parsing failed: {0}")]
    TimeParse(time::error::Parse),
    #[error("event state is invalid: {0}")]
    State(StateError),
    #[error("event append raced: expected sequence {expected}, durable head is {actual}")]
    Concurrency { expected: u64, actual: u64 },
    #[error("event sequence {0} exceeds SQLite's signed integer range")]
    SequenceOverflow(u64),
    #[error("negative event sequence {0} found in storage")]
    InvalidSequence(i64),
    #[error("invalid event schema number {0} found in storage")]
    InvalidSchema(i64),
    #[error("invalid run identifier {0:?} found in event storage")]
    InvalidRunId(String),
    #[error("compiled event schema {EVENT_SCHEMA_VERSION} cannot read stored data")]
    UnsupportedSchema,
    #[error("event database schema version {0} is unsupported")]
    UnsupportedDatabaseSchema(i64),
}

#[cfg(test)]
mod tests {
    use super::*;
    use pactrail_core::{RunState, TaskContract};

    #[test]
    fn events_round_trip_and_project() {
        let mut store = EventStore::open_in_memory()
            .unwrap_or_else(|error| unreachable!("event store: {error}"));
        let run_id = RunId::new();
        store
            .append(
                run_id,
                0,
                RunEvent::ContractRegistered(TaskContract::new("fix bug", ".")),
            )
            .unwrap_or_else(|error| unreachable!("append contract: {error}"));
        store
            .append(
                run_id,
                1,
                RunEvent::StateChanged {
                    from: RunState::Created,
                    to: RunState::Contracting,
                },
            )
            .unwrap_or_else(|error| unreachable!("append transition: {error}"));

        let snapshot = store
            .snapshot(run_id)
            .unwrap_or_else(|error| unreachable!("project run: {error}"));
        assert_eq!(snapshot.state, RunState::Contracting);
        assert_eq!(snapshot.last_sequence, Some(1));
    }

    #[test]
    fn optimistic_sequence_prevents_lost_events() {
        let mut store = EventStore::open_in_memory()
            .unwrap_or_else(|error| unreachable!("event store: {error}"));
        let run_id = RunId::new();
        store
            .append(
                run_id,
                0,
                RunEvent::NoteRecorded {
                    message: "first".to_owned(),
                },
            )
            .unwrap_or_else(|error| unreachable!("append first: {error}"));
        assert!(matches!(
            store.append(
                run_id,
                0,
                RunEvent::NoteRecorded {
                    message: "stale".to_owned()
                }
            ),
            Err(StoreError::Concurrency {
                expected: 0,
                actual: 1
            })
        ));
    }

    #[test]
    fn lists_runs_newest_first() {
        let mut store = EventStore::open_in_memory()
            .unwrap_or_else(|error| unreachable!("event store: {error}"));
        let first = RunId::new();
        let second = RunId::new();
        for run_id in [first, second] {
            store
                .append(
                    run_id,
                    0,
                    RunEvent::NoteRecorded {
                        message: "run".to_owned(),
                    },
                )
                .unwrap_or_else(|error| unreachable!("append: {error}"));
        }

        assert_eq!(
            store
                .list_run_ids()
                .unwrap_or_else(|error| unreachable!("list: {error}")),
            vec![second, first]
        );
    }

    #[test]
    fn future_database_schema_is_rejected() {
        let directory = tempfile::tempdir()
            .unwrap_or_else(|error| unreachable!("temporary directory: {error}"));
        let path = directory.path().join("events.sqlite3");
        let connection = Connection::open(&path)
            .unwrap_or_else(|error| unreachable!("fixture database: {error}"));
        connection
            .pragma_update(None, "user_version", 99)
            .unwrap_or_else(|error| unreachable!("fixture schema: {error}"));
        drop(connection);

        assert!(matches!(
            EventStore::open(path),
            Err(StoreError::UnsupportedDatabaseSchema(99))
        ));
    }
}
