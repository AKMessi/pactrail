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
    │         │ exact ApprovalBinding                   ▼
    │         └──> ProcessBackend (disabled/native/OCI)
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
| `pactrail-git` | Bounded, process-free HEAD/index/raw-worktree evidence | Commands, hooks, filters, credentials, remotes, or candidate mutation |
| `pactrail-tools` | Tool contracts, annotations, registry, capability policy, bounded built-ins | Run lifecycle or source landing |
| `pactrail-models` | Provider-neutral conversation IR and bounded model transport | Tool execution or workspace access |
| `pactrail-mcp` | Governed MCP manifests, pinned catalogs, bounded transports, and Tool Kernel adapters | Local policy assignment or implicit discovery |
| `pactrail-engine` | Orchestration, tool scheduling, effect reconciliation, budgets, verification | CLI persistence policy |
| `pactrail-sdk` | Stable embedding facade over provider, tool, engine, MCP, and durable-state contracts | A second execution path or dynamic native plugin loading |
| `pactrail` | Interactive/scriptable UX, settings, run artifacts, apply/discard commands | Weakening lower-layer invariants |

Dependencies point inward. The domain layer cannot call a provider or mutate a
workspace, and a provider implementation cannot execute a tool.

## Task and capability contracts

A `TaskContract` defines the goal, workspace, write prefixes, obligations,
budget, and explicit allow/deny capability sets. Runtime policy is checked
against the contract before a run starts; an effective runtime grant absent
from the durable contract is an invalid configuration.

The built-in capabilities distinguish file reads, file writes, memory reads,
MCP invocation, process execution, network, secret use, and external writes.
Denial wins.
Undeclared process authority requires approval; it is never silently granted.
Each approval is bound to the run, exact non-secret request, executable actor,
backend identity, and resource-profile digest. Policy evaluation and human or
automation decisions are distinct hash-linked events and durable receipt data.

The process backend is selected before the run starts. `disabled` rejects every
process. `native_trusted` executes directly on the host and records its complete
effective authority. `oci_restricted` invokes a locally attested Docker or
Podman runtime with a locally resolved immutable image, candidate-only mount,
read-only root, private bounded temporary space, no network or capabilities, a
numeric Unix UID/GID where supported, and CPU/memory/PID/time/output bounds.
Initialization fails closed and never downgrades to native execution.

## Context compiler

Repository indexing is deterministic and model-free. It records stable portable
paths, BLAKE3 digests, sizes, coarse languages, imports, and symbol-like
declarations. The default build uses bounded embedded Tree-sitter grammars for
Rust, Python, JavaScript, and TypeScript/TSX; unsupported, oversized, cancelled,
or node-budget-exhausted parses use the deterministic lexical analyzer. A
feature-disabled build removes the parser dependencies entirely. Non-UTF-8
files remain visible in topology without semantic scanning.

Every build hashes current file bytes. Bounded per-file derived structure is
reused from a content-addressed cache keyed by content digest, language,
analysis revision, and analyzer profile; source text, previews, and instructions
are never supplied by that cache. A malformed or unavailable entry is measured and recomputed.
This makes invalidation exact while keeping the cache outside durable authority.

The index also derives a bounded repository evidence graph. Definition nodes
come from project symbol declarations; default edges point to exact file and
line locations where the same project-defined identifier occurs. An embedder
may explicitly merge one bounded integrity-checked LSP reference snapshot for
the exact repository digest. Every reference is labelled lexical,
language-server, or corroborated; none is a runtime call claim. Construction is capped at
200,000 definitions, 500,000 references, and 256 references per symbol. Cached
identifier locations also have a per-file bound. The graph is built from the
same current-byte analysis, eliminating a second filesystem read.

For each run, the compiler:

1. Rewrites the model-visible workspace root to `.` so host paths never enter
   tool instructions.
2. Derives a conservative context byte ceiling from declared context and output
   token limits after subtracting the integrity-bound image artifact estimate,
   reserving most of the remaining window for tool schemas, conversation, tool
   results, and generation.
3. Requires the task contract and root `AGENTS.md` to fit in full. Failure is
   explicit rather than silently dropping authoritative instructions.
4. Labels nested `AGENTS.md` files with their virtual directory scope.
5. Falls back to conventional manifests, READMEs, and entrypoints for broad
   repository questions, adding bounded current previews labelled as untrusted
   file evidence.
6. Expands task-matched symbols to bounded definition and reference locations,
   giving initial retrieval one repository-wide relationship hop.
7. Produces a deterministic project profile from root ecosystem manifests and
   conventional entrypoints so tiny models do not have to infer basic topology.
8. Adds complete relevant memory and topology entries in priority order.
9. Omits optional entries whole, records inclusion metadata, and shows the model
   a visible budget-exhaustion notice.

The context action records bytes hashed, cache reuse/rejection, retrieval and
graph counts, and kernel-derived citation coverage. Coverage measures how much
of the selected file set fit in the bounded pack; it is not a model-authored
relevance or correctness score.

A deterministic repository-scale runner exercises cold, warm, and one-file
incremental builds over a generated polyglot monorepo, then compiles targeted
context under an explicit byte budget. It emits versioned JSON rather than a
human-only benchmark line. A separate descriptor gate caps built-in tool count,
aggregate/single JSON weight, and schema depth. Dedicated Linux CI builds the
runner before measurement, records release-mode phase latency and maximum RSS,
retains raw artifacts, and applies generous regression ceilings rather than
presenting shared-runner timing as a universal benchmark.

Tree-sitter file count, lexical-fallback count, and syntax-error file count are
also live and durable telemetry. Optional LSP evidence never starts a server:
the SDK accepts only a prebuilt strict snapshot, validates every symbol/path/
line before mutation, and folds the snapshot digest into context identity.

Memory is advisory. It includes an identifier, kind, source, title, and content;
it never overrides the task contract, scoped instructions, or current files.

## Tool Kernel v2

Every tool exposes a JSON Schema input contract, required capability, and four
UX/scheduling annotations: read-only, idempotent, parallel-safe, and risk class.
The production registry currently provides:

- `list_files`, `read_file`, `read_many_files`, and `search`;
- `search_code_graph` for project definitions and bounded reference evidence;
- `search_change_impact` for bounded one-hop definition/reference relationships;
- `git_status`, `git_diff`, and `git_history` for process-free source repository evidence;
- `write_file`, `replace_text`, atomic `edit_file`, and `remove_file`;
- `workspace_changes` and `recall_memory`;
- capability-gated `run_process` for detected verification.

The engine executes consecutive parallel-safe calls concurrently. A mutation,
unknown tool, or host-execution call closes the read batch; later calls cannot
overtake it. Results are journaled in the model's original call order, keeping
replay deterministic.

`search_code_graph` rebuilds the evidence graph from the current isolated
candidate on each call. This avoids serving a stale pre-edit graph and keeps
cache invalidation outside the trust boundary. The output carries the current
repository digest, explicit truncation state, definition provenance, and a
warning to read cited source before editing.

`search_change_impact` uses direct task matches as seeds, then identifies files
that reference seed-defined symbols and files defining symbols referenced by a
seed. Scores and reasons are deterministic and bounded, and reference reasons
retain lexical, language-server, or corroborated provenance. The result is
navigation evidence, not a type-resolved dependency or runtime-impact claim,
and is rebuilt from the current candidate for the same freshness guarantee.

Git evidence is intentionally a separate crate and a narrower boundary than a
shell wrapper. It opens only `.git` at the exact source root with isolated,
strict, trusted configuration. Pactrail enables object, index, and revision
reads but none of Gitoxide's command, network-client, credential, status/filter,
or remote-operation features; its private inspector contains no such call site.
`git_status` presents source
HEAD-to-index, index-to-raw-worktree, and isolated-candidate changes as distinct
records. `git_diff` is a bounded raw HEAD-to-source navigation artifact;
candidate inspection remains transaction-owned. `git_history` is newest-first,
bounded, and omits email addresses. Huge trees, indexes, files, histories, and
outputs either truncate visibly or become explicit inconclusive evidence.

Each tool result is normalized, output-bounded, and compared against transaction
manifests before and after execution. The event record contains a digest of the
arguments rather than raw potentially sensitive inputs, plus duration, risk,
call ID, output size/truncation, declared capability, and observed effects.

Successful write and exact-edit results additionally contain bounded post-edit
evidence derived from the current isolated candidate: final content digest and
size, the first and last changed line, and line-numbered source windows with
per-line truncation labels. Nearby changes use one window; distant changes use
bounded windows at both edges. The result explicitly states whether every
changed line is shown and directs the model to a narrow `read_file` call when it
is not. No-op exact replacements are rejected because they create neither a
candidate delta nor useful evidence.

### Trajectory context controller

Before every model invocation, the engine measures the exact provider-neutral
JSON representation of the conversation and tool descriptors. Its conservative
high-water and target marks are derived from declared context and output token
limits; provider token accounting remains authoritative.

When the high-water mark is crossed, older tool results are replaced in place
with deterministic compaction envelopes. Each envelope retains the tool name,
call ID, error state, original byte count and BLAKE3 digest, bounded scalar
anchors such as paths and line cursors, a small exact JSON prefix, and explicit
instructions to re-run the original call more narrowly. Assistant tool calls
and conversation order are never removed, preserving provider protocol
validity. The latest tool turn remains unmodified unless it alone threatens the
window. Model-generated summaries are never used for compaction.

Each compaction writes before/after request digests, byte counts, thresholds,
and reclaimed bytes to the hash-linked action journal and appears in the live
CLI timeline. Raw observations remain intentionally absent from the durable
trace.

The loop controller separately tracks identical tool turns and all-failed tool
turns. A repeated successful read-only call receives explicit steering. If a
conservatively classified informational goal still repeats three times, Pactrail
permits exactly one additional model attempt with no tools and an evidence-only
synthesis instruction. This recovery consumes the normal model-attempt and
token budgets and is fully journaled. Generated CLI contracts derive those
budgets from the configured context, output, and turn ceilings; explicit task
contracts retain their declared resource governance. Failed calls, change requests, and
mutation loops do not receive this fallback.

## Provider and streaming boundary

Image input extends the ordered conversation IR with `UserContent`, not with a
provider-specific side channel. Each `ImageArtifact` contains only a portable
filename, fixed media type, dimensions, byte count, complete base64 payload,
and BLAKE3 digest. The constructors and deserializer enforce the same bounds, so
checkpoint loading cannot bypass validation. Payload storage uses shared
immutable ownership to keep per-turn conversation cloning constant-time for the
large field. Context fingerprinting substitutes the image digest for base64;
the engine separately reserves a conservative 768-pixel-tile token estimate
before it compiles repository context. This avoids counting transport encoding
as text while still failing an impossible model window before invocation.

The initial user turn owns the images and is checkpointed as one atomic
provider-neutral item. OpenAI-compatible adapters map it to text and data-URL
content parts, Anthropic to labelled base64 image/text blocks, and Gemini to
text plus `inlineData`. Every adapter independently rejects image content when
the effective model profile does not declare vision. Resume obtains images only
from the head-bound checkpoint and forbids injection of new image bytes into an
existing run.

`pactrail-models` translates OpenAI-compatible Chat Completions, Anthropic
Messages, and Gemini GenerateContent into the same ordered conversation,
complete response, tool-call, finish-reason, and usage types. Protocol-specific
continuation metadata is carried only in bounded extension maps; it cannot
grant a capability or bypass the tool kernel.

Streaming drivers emit transient observer events for response start, sanitized
text, tool-call discovery, argument-byte progress, and cumulative usage. Each
driver owns a bounded state machine that validates framing, ordering, IDs,
argument JSON, finish semantics, usage monotonicity, and terminal completion.
The engine receives only the final normalized `ModelResponse`. Partial text is
not appended to durable conversation and partial tool input is never executed.
A failed stream therefore resumes, when safe, from the preceding complete-turn
checkpoint.

The effective `ModelCapabilities` profile is compiled independently of the
provider label and participates in checkpoint identity. User overrides are
explicit. The optional capability probe sends one synthetic read-only tool
request and executes nothing; it can produce positive observations but cannot
infer support from a provider name or turn a missing observation into a
negative capability decision.

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

Action events cover context compilation and compaction, model requests, tools,
and verifier commands. Policy decisions, evidence, checkpoints, notes, and state
transitions share the same journal. The CLI exports the verified stream to run-local
`trace.jsonl` atomically after run, apply, and discard transitions. The SQLite
journal remains authoritative; JSONL is the portable inspection artifact.

Pactrail intentionally does not persist raw model prompts, responses, API keys,
or raw tool arguments in traces.

Read-only informational runs transition from `Reviewing` to terminal
`Completed` and issue an `Answered` receipt. Change runs retain the explicit
`AwaitingApply` boundary. For broad workspace answers the engine prepends the
deterministic project profile to a separately labelled model explanation and
records the grounding action in the trace.

## Checkpoint and restart protocol

While a run is executing, Pactrail serializes provider-neutral conversation and
controller state into bounded content-addressed checkpoint artifacts. A
`CheckpointCreated` event names the artifact only after its bytes are durable.
The checkpoint points back to the preceding event sequence/hash and also binds
the task contract, candidate change set, repository context, model/tool
profiles, secret-free CLI manifest, resolved process-runtime/image profile,
sealed input artifacts, token use, turn counters, repair state, and elapsed
active budget.

`pactrail resume <run-id>` reopens the existing workspace transaction and reads
the original `run.json`; it never reloads a mutable task file. Before appending
anything, the engine requires the supplied checkpoint to be the exact artifact
named by the current event head and recomputes every identity digest. Keys are
resolved afresh from the recorded environment-variable name and are never part
of the manifest or checkpoint.

One local execution owner is enforced twice. A kernel file lock prevents two
live processes from driving the same run and disappears immediately on process
death. SQLite schema 2 retains a bounded ownership lease so concurrent and stale
ownership remain diagnosable. The same mechanism covers new and resumed runs.

Model-requested tools are effect-fenced. `EffectPrepared` is appended before a
tool receives control and binds its call ID, name, risk, arguments, candidate,
and runtime profile. `EffectCompleted` follows the normalized action/result and
binds the resulting candidate. A crash with a prepared but incomplete effect is
reported by tool and risk; Pactrail refuses automatic replay. A crash after an
effect but before the next complete conversation checkpoint also stops safely
because the event head is not resumable. This deliberately prefers explicit
human recovery over duplicating a candidate mutation or host/external effect.

Automatic continuation currently occurs at pre-model and pre-verification safe
points. A pre-tool checkpoint is retained for diagnosis, but resume refuses it
until a complete effect-reconciliation policy can preserve the exact tool
result topology. Terminal runs, cancelled runs, and ready-to-apply candidates
use their existing receipt/apply recovery paths rather than re-entering the
model loop.

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
JavaScript projects. Execution is possible only when process authority is
granted and a non-disabled backend is selected. Authorized checks run from a
disposable snapshot of the finished candidate, so compiler output, coverage
data, bytecode, and test-runner caches cannot enter the reviewed change set. The
trace labels this execution workspace explicitly. Retained output and wall time
are bounded.

Verification results become deterministic evidence. Model statements do not.
Each required obligation receives a grade and status; missing process permission
creates explicit inconclusive evidence and an unresolved risk rather than a
fictional pass.

When a model first declares a changed candidate complete, authorized discovered
checks run as a repair probe if at least one model turn remains. A non-zero
process exit can trigger exactly one automatic repair cycle. Pactrail returns a
model-window-sized preview of structured stdout/stderr diagnostics, its full
BLAKE3 digest and byte count, and an authoritative warning that repository
process output is untrusted data. Tool-launch, authorization, and infrastructure
errors do not trigger source repair. A successful gate on an unchanged candidate
becomes final evidence directly, avoiding a duplicate test run. After a repair
attempt, normal final verification runs again in a fresh disposable snapshot and
is the only result that becomes receipt evidence. Gate and final verifier actions
carry explicit `completion_gate` and `final` trace phases; the controller
decision records the candidate and diagnostics digests.

Native processes start from a cleared environment. Pactrail inherits an
explicit toolchain/operating-system allowlist rather than arbitrary variables;
Windows Visual C++ and SDK discovery paths are included, while API keys,
`CARGO_TARGET_DIR`, wrappers, and other undeclared variables are not. The OCI
backend forwards no ambient host environment at all. Explicit environment
entries are exact approval-bound request data in either mode.

One cancellation token spans provider requests, tool scheduling, native child
termination, OCI force-removal, verification, repair, and the CLI. Cancellation
stops new work, waits for bounded cleanup, records a terminal `Cancelled` state,
and preserves an integrity-checked candidate receipt when safe. Cleanup failure
is a hard error rather than a successful cancellation claim.

Model exploration is bounded independently of provider context size. A
`read_file` call without an explicit range returns at most 300 lines and exposes
the next line cursor; explicit ranges remain available up to 1,000 lines.
`search` accepts either a workspace-relative directory or a specific file so a
recoverable path-shape mistake does not consume another model turn. If the turn
budget ends after real candidate changes, Pactrail records the missing model
attestation as a risk and still runs deterministic verification. An unchanged
run still fails at the turn limit.

## Compatibility policy

Contracts, event envelopes, receipts, memory databases, transaction metadata,
and interactive settings carry explicit schema versions. Unknown persisted
versions fail closed. User-visible behavior follows semantic versioning;
breaking public Rust API or local-format changes remain possible during the
`0.x` developer-preview line and will be recorded in `CHANGELOG.md`.
