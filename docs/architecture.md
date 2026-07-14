# Pactrail architecture

Pactrail separates probabilistic reasoning from deterministic safety and state.
The model may propose actions; it never owns the filesystem, policy, event log,
or final apply operation.

## Data flow

```text
TaskContract
    │
    ├── RepositoryIndex ──> ContextPack ──> ModelDriver
    │                                      │
    │                                      v
    ├── PolicyEngine <── ToolRegistry <── tool calls
    │                       │
    │                       v
    ├── EventStore <── WorkspaceTransaction
    │                       │
    │                       v
    └── Evidence ──────> ChangeReceipt ──> explicit apply/discard
```

## Crate boundaries

- `pactrail-core` contains versioned contracts, events, evidence, policy types,
  lifecycle projection, and receipts. It has no database, network, UI, or process dependency.
- `pactrail-store` owns SQLite append semantics and compressed content-addressed artifacts.
- `pactrail-workspace` owns snapshots, path confinement, manifests, diffs, landing journals, and rollback.
- `pactrail-context` extracts repository topology and compiles bounded context packs.
- `pactrail-tools` defines the object-safe tool API, policy evaluator, registry, and built-in tools.
- `pactrail-models` defines the provider-neutral conversation IR and model-driver API.
- `pactrail-engine` owns lifecycle orchestration, budgets, effect reconciliation, and verification.
- `pactrail` is the CLI and durable local run manager.

Dependencies point inward. The core domain cannot call a provider or mutate a workspace.

## Durable state

Each run is a monotonic sequence of schema-versioned events. An event includes
the previous event hash and its own BLAKE3 hash. Loading a run replays every event
through the same state reducer and rejects sequence gaps, cross-run records,
invalid transitions, broken hash chains, and tampering.

SQLite uses WAL mode, full synchronization, strict tables, and an immediate
transaction for optimistic sequence checks. Large artifacts are compressed and
stored under their uncompressed BLAKE3 digest.

## Transaction landing

The transaction records a sorted baseline manifest, then copies non-ignored
regular files into a separate workspace. Model tools receive only that root.

Apply performs these checks:

1. Rescan the transaction and determine touched paths.
2. Verify every touched source path still has its baseline digest.
3. Back up existing touched files into a run-local journal.
4. Write and synchronize candidate files.
5. Roll back from the journal if any write fails.
6. Record the `Applied` event and reissue the receipt.

Apply is idempotent. If the filesystem landing succeeded but event persistence
was interrupted, a retry recognizes that every touched source file already
matches the candidate and completes the event transition without rewriting it.
Foreign source changes are never overwritten.

## Model loop

Provider messages, assistant tool calls, and tool results use a lossless internal
conversation sequence. Tool calls are ID-unique within the run. Each call records
model/provider identity, declared capability, result, and filesystem effects
calculated by comparing transaction manifests before and after the call.

The engine bounds model attempts and aggregate reported tokens. A final response
does not become evidence merely because the model states it. Verification results
are recorded separately and graded.

## Compatibility policy

Contracts, event envelopes, and receipts carry independent schema versions.
Unknown persisted versions fail closed. Public behavior follows semantic
versioning; breaking Rust API changes are permitted only during the `0.x`
developer-preview line.

