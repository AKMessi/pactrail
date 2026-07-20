use std::path::Path;
use std::time::Duration;

use pactrail_core::{
    EVENT_SCHEMA_VERSION, EventEnvelope, EventHash, RunEvent, RunId, RunSnapshot, StateError,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use thiserror::Error;
use time::OffsetDateTime;

/// Current on-disk schema for the append-only event database.
pub const EVENT_DATABASE_SCHEMA_VERSION: i64 = 2;

/// Oldest initialized event database schema this binary migrates in place.
pub const MIN_EVENT_DATABASE_SCHEMA_VERSION: i64 = 1;

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
        if !(0..=EVENT_DATABASE_SCHEMA_VERSION).contains(&database_version) {
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
                 CREATE TABLE IF NOT EXISTS run_leases (
                    run_id TEXT PRIMARY KEY,
                    owner TEXT NOT NULL,
                    expires_unix_ms INTEGER NOT NULL CHECK (expires_unix_ms >= 0)
                 ) STRICT;
                 COMMIT;",
            )
            .map_err(StoreError::Database)?;
        if database_version < EVENT_DATABASE_SCHEMA_VERSION {
            connection
                .pragma_update(None, "user_version", EVENT_DATABASE_SCHEMA_VERSION)
                .map_err(StoreError::Database)?;
        }
        Ok(Self { connection })
    }

    /// Acquires exclusive, expiring ownership of a run's active execution.
    ///
    /// A crashed owner leaves a bounded lease which can be replaced only after
    /// expiry. Supplying the same owner renews its lease atomically.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::LeaseHeld`] while another live owner exists, or a
    /// validation/database error when the lease cannot be created.
    pub fn acquire_run_lease(
        &mut self,
        run_id: RunId,
        owner: &str,
        ttl: Duration,
    ) -> Result<RunLease, StoreError> {
        let now = OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000;
        let now = i64::try_from(now).map_err(|_| StoreError::LeaseClockRange)?;
        self.acquire_run_lease_at(run_id, owner, ttl, now)
    }

    fn acquire_run_lease_at(
        &mut self,
        run_id: RunId,
        owner: &str,
        ttl: Duration,
        now_unix_ms: i64,
    ) -> Result<RunLease, StoreError> {
        validate_lease_owner(owner)?;
        let ttl_ms = i64::try_from(ttl.as_millis()).map_err(|_| StoreError::LeaseTtlRange)?;
        if ttl_ms == 0 || ttl > Duration::from_hours(30 * 24) || now_unix_ms < 0 {
            return Err(StoreError::LeaseTtlRange);
        }
        let expires_unix_ms = now_unix_ms
            .checked_add(ttl_ms)
            .ok_or(StoreError::LeaseTtlRange)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Database)?;
        let changed = transaction
            .execute(
                "INSERT INTO run_leases (run_id, owner, expires_unix_ms)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(run_id) DO UPDATE SET
                    owner = excluded.owner,
                    expires_unix_ms = excluded.expires_unix_ms
                 WHERE run_leases.expires_unix_ms <= ?4 OR run_leases.owner = ?2",
                params![run_id.to_string(), owner, expires_unix_ms, now_unix_ms],
            )
            .map_err(StoreError::Database)?;
        if changed == 0 {
            let (holder, expires_unix_ms) = transaction
                .query_row(
                    "SELECT owner, expires_unix_ms FROM run_leases WHERE run_id = ?1",
                    [run_id.to_string()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                )
                .map_err(StoreError::Database)?;
            return Err(StoreError::LeaseHeld {
                run_id,
                owner: holder,
                expires_unix_ms,
            });
        }
        transaction.commit().map_err(StoreError::Database)?;
        Ok(RunLease {
            run_id,
            owner: owner.to_owned(),
            expires_unix_ms,
        })
    }

    /// Releases a lease only when its exact owner still holds it.
    ///
    /// # Errors
    ///
    /// Returns a database error, or [`StoreError::LeaseOwnership`] if a
    /// different owner replaced the lease after expiry.
    pub fn release_run_lease(&mut self, lease: &RunLease) -> Result<(), StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Database)?;
        let current = transaction
            .query_row(
                "SELECT owner FROM run_leases WHERE run_id = ?1",
                [lease.run_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(StoreError::Database)?;
        if current.as_deref().is_some_and(|owner| owner != lease.owner) {
            return Err(StoreError::LeaseOwnership {
                run_id: lease.run_id,
            });
        }
        if current.is_some() {
            transaction
                .execute(
                    "DELETE FROM run_leases WHERE run_id = ?1 AND owner = ?2",
                    params![lease.run_id.to_string(), lease.owner],
                )
                .map_err(StoreError::Database)?;
        }
        transaction.commit().map_err(StoreError::Database)
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

/// Exact ownership token for a bounded local run lease.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunLease {
    pub run_id: RunId,
    pub owner: String,
    pub expires_unix_ms: i64,
}

fn validate_lease_owner(owner: &str) -> Result<(), StoreError> {
    if owner.is_empty()
        || owner.len() > 128
        || owner
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        Err(StoreError::InvalidLeaseOwner)
    } else {
        Ok(())
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
    #[error(
        "run {run_id} is already active under owner {owner}; lease expires at Unix millisecond {expires_unix_ms}"
    )]
    LeaseHeld {
        run_id: RunId,
        owner: String,
        expires_unix_ms: i64,
    },
    #[error("run {run_id} lease ownership changed before release")]
    LeaseOwnership { run_id: RunId },
    #[error("run lease owner must be 1-128 non-whitespace, non-control bytes")]
    InvalidLeaseOwner,
    #[error("run lease duration or clock value is outside the supported range")]
    LeaseTtlRange,
    #[error("system clock cannot be represented as a run-lease timestamp")]
    LeaseClockRange,
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
    fn live_lease_excludes_a_second_owner_and_stale_lease_is_replaceable() {
        let mut store = EventStore::open_in_memory()
            .unwrap_or_else(|error| unreachable!("event store: {error}"));
        let run_id = RunId::new();
        let first = store
            .acquire_run_lease_at(run_id, "owner-a", Duration::from_millis(100), 1_000)
            .unwrap_or_else(|error| unreachable!("first lease: {error}"));
        assert!(matches!(
            store.acquire_run_lease_at(run_id, "owner-b", Duration::from_millis(100), 1_099),
            Err(StoreError::LeaseHeld { .. })
        ));
        let second = store
            .acquire_run_lease_at(run_id, "owner-b", Duration::from_millis(100), 1_100)
            .unwrap_or_else(|error| unreachable!("replacement lease: {error}"));
        assert_ne!(first.owner, second.owner);
        assert!(matches!(
            store.release_run_lease(&first),
            Err(StoreError::LeaseOwnership { .. })
        ));
        store
            .release_run_lease(&second)
            .unwrap_or_else(|error| unreachable!("release: {error}"));
        store
            .release_run_lease(&second)
            .unwrap_or_else(|error| unreachable!("idempotent release: {error}"));
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
