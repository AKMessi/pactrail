# Pactrail architecture

Pactrail separates probabilistic reasoning from deterministic authority. A
model can propose typed actions. It never owns policy, filesystem resolution,
durable state, evidence grading, or the apply boundary.

## System flow

```text
TaskContract
    │
    ├── RepositoryIndex ──> budgeted ContextPack ──> ModelDriver
    │           ▲                   ▲                    │
    │           │             scoped instructions       │ typed ToolCall
    │      current files        + retrieved memory       ▼
    │                                                Tool Kernel
    │                                                   │
    ├── PolicyEngine <──────── capability check ────────┤
    │                                                   ▼
    ├── EventStore <──── reconciled effects ── WorkspaceTransaction
    │      │                                            │
    │      └── hash-linked trace                        │ candidate diff
    │                                                   ▼
    └── obligations ──> Evidence ──> ChangeReceipt ──> apply/discard
```

Every arrow crossing from the model into the kernel is validated. Every
mutation is made in the candidate tree. Every source-workspace mutation is
receipt-bound.

## Crate boundaries

| Crate | Owns | Must not own |
|---|---|---|
| `pactrail-core` | Versioned contracts, capabilities, events, evidence, lifecycle reduction, receipts | Databases, networks, UI, processes |
| `pactrail-store` | SQLite event append/replay and compressed content-addressed artifact primitive | Model or workspace semantics |
| `pactrail-memory` | Provenance-aware SQLite memories, ranking, soft deletion, applied-receipt ingestion | Prompt authority or model-authored writes |
| `pactrail-workspace` | Safe relative paths, manifests, candidate copies, diffs, apply journal, rollback | Provider calls or evidence claims |
| `pactrail-context` | Repository index, instruction scopes, retrieval, model-aware pack budgeting | Filesystem mutation or provider tokenization |
| `pactrail-tools` | Tool contracts, annotations, registry, capability policy, bounded built-ins | Run lifecycle or source landing |
| `pactrail-models` | Provider-neutral conversation IR and bounded model transport | Tool execution or workspace access |
| `pactrail-engine` | Orchestration, tool scheduling, effect reconciliation, budgets, verification | CLI persistence policy |
| `pactrail` | Interactive/scriptable UX, settings, run artifacts, apply/discard commands | Weakening lower-layer invariants |

Dependencies point inward. The domain layer cannot call a provider or mutate a
workspace, and a provider implementation cannot execute a tool.

## Task and capability contracts

A `TaskContract` defines the goal, workspace, write prefixes, obligations,
budget, and explicit allow/deny capability sets. Runtime policy is checked
against the contract before a run starts; an effective runtime grant absent
from the durable contract is an invalid configuration.

The built-in capabilities distinguish file reads, file writes, memory reads,
process execution, network, secret use, and external writes. Denial wins.
Native processes require the complete effective authority to be recorded rather
than representing process execution as a narrow filesystem permission.

## Context compiler

Repository indexing is deterministic and model-free. It records stable portable
paths, BLAKE3 digests, sizes, coarse languages, imports, and symbol-like
declarations. Oversized and non-UTF-8 files remain visible in topology without
being retained for semantic scanning.

For each run, the compiler:

1. Rewrites the model-visible workspace root to `.` so host paths never enter
   tool instructions.
2. Derives a conservative context byte ceiling from declared context and output
   token limits, reserving most of the window for tool schemas, conversation,
   tool results, and generation.
3. Requires the task contract and root `AGENTS.md` to fit in full. Failure is
   explicit rather than silently dropping authoritative instructions.
4. Labels nested `AGENTS.md` files with their virtual directory scope.
5. Adds complete relevant memory and topology entries in priority order.
6. Omits optional entries whole, records inclusion metadata, and shows the model
   a visible budget-exhaustion notice.

Memory is advisory. It includes an identifier, kind, source, title, and content;
it never overrides the task contract, scoped instructions, or current files.

## Tool Kernel v2

Every tool exposes a JSON Schema input contract, required capability, and four
UX/scheduling annotations: read-only, idempotent, parallel-safe, and risk class.
The production registry currently provides:

- `list_files`, `read_file`, `read_many_files`, and `search`;
- `write_file`, `replace_text`, atomic `edit_file`, and `remove_file`;
- `workspace_changes` and `recall_memory`;
- capability-gated `run_process` for detected verification.

The engine executes consecutive parallel-safe calls concurrently. A mutation,
unknown tool, or host-execution call closes the read batch; later calls cannot
overtake it. Results are journaled in the model's original call order, keeping
replay deterministic.

Each tool result is normalized, output-bounded, and compared against transaction
manifests before and after execution. The event record contains a digest of the
arguments rather than raw potentially sensitive inputs, plus duration, risk,
call ID, output size/truncation, declared capability, and observed effects.

## Durable memory

Workspace memory uses a separate SQLite database in WAL mode with full
synchronization and fail-closed schema migration. Supported user-authored kinds
are conventions, decisions, and warnings. Inputs and tags are bounded and
validated; forgetting is a durable soft delete.

Applied-run memories are accepted only from an integrity-valid `Applied`
receipt. Run ID and receipt hash provide idempotency and provenance. The model
has a read-only recall tool; no model tool can add, rewrite, or forget memory,
which prevents straightforward prompt-driven memory poisoning.

## Event protocol and traces

A run is a monotonic sequence of schema-versioned `EventEnvelope` values. Each
envelope includes run ID, sequence, RFC 3339 timestamp, previous hash, payload,
and its own BLAKE3 hash. Loading a run verifies the entire chain and replays
events through the same lifecycle reducer. Sequence gaps, cross-run records,
unknown schema versions, invalid transitions, and tampering fail closed.

Action events cover context compilation, model requests, tools, and verifier
commands. Policy decisions, evidence, checkpoints, notes, and state transitions
share the same journal. The CLI exports the verified stream to run-local
`trace.jsonl` atomically after run, apply, and discard transitions. The SQLite
journal remains authoritative; JSONL is the portable inspection artifact.

Pactrail intentionally does not persist raw model prompts, responses, API keys,
or raw tool arguments in traces.

## Workspace transaction and apply

Creation records a sorted baseline manifest and copies non-ignored regular files
to `runs/<id>/workspace`. Model tools receive only this virtual root. Safe-path
resolution rejects absolute paths, platform prefixes, parent traversal,
symlinks, and special files; writes additionally require an allowed prefix.

Apply performs the following sequence:

1. Rescan the candidate and require an exact match with the receipt change set.
2. Verify receipt integrity and evidence coverage.
3. Verify every touched source path still matches its baseline bytes and mode.
4. Back up touched paths into a synchronized run-local apply journal.
5. Revalidate candidate bytes/modes, land them, and synchronize writes.
6. Roll back from the journal on failure.
7. Record `Applied`, reissue the receipt, export the trace, and ingest the
   integrity-checked applied-run memory.

Apply is idempotent. If files landed but event persistence was interrupted, a
retry recognizes candidate-equivalent source files and completes the state
transition without blindly rewriting them. Foreign changes are never
overwritten.

## Verification and evidence

Manifest discovery selects non-installing checks for Rust, Go, Python, and
JavaScript projects. Execution is possible only when native process authority is
granted. Retained output and wall time are bounded.

Verification results become deterministic evidence. Model statements do not.
Each required obligation receives a grade and status; missing process permission
creates explicit inconclusive evidence and an unresolved risk rather than a
fictional pass.

## Compatibility policy

Contracts, event envelopes, receipts, memory databases, transaction metadata,
and interactive settings carry explicit schema versions. Unknown persisted
versions fail closed. User-visible behavior follows semantic versioning;
breaking public Rust API or local-format changes remain possible during the
`0.x` developer-preview line and will be recorded in `CHANGELOG.md`.
