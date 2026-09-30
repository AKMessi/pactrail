# Design 0016: durable, evidence-driven workspace runtime

Status: proposed; not implemented by the CLI experience upgrade.
Research date: September 30, 2026.

## Recommendation in plain words

Give Pactrail one durable engine for a workspace. The terminal, browser, editor,
and automation should all be controls for that same engine. Then let that engine
spend its next turn on the most useful missing fact or check, with explicit
budgets and measurable results.

This is the strongest next upgrade because it connects three things that users
currently experience separately: smooth interaction, honest observability, and
reliable task completion. A prettier terminal does not remove those gaps.

“Best harness” is a target, not an established result. This proposal defines how
to measure progress toward it; it does not claim a benchmark victory.

## What already exists

V2 already supplies provider-neutral requests, native provider adapters,
capability profiles, stable tool catalogs, context budgeting and compaction,
content-addressed observation reuse, bounded routing, cost reservations,
isolated candidates, contract-bound acceptance checks, effect fences,
checkpoint recovery, and receipt-bound apply. These are foundations to reuse,
not features to reimplement or rename.

The relevant implementation is in:

- `crates/pactrail-cli/src/interactive.rs`: the terminal calls the engine and
  waits while observing progress. Cancellation is available; a second prompt
  cannot currently steer an executing turn.
- `crates/pactrail-cli/src/web.rs`: the browser starts CLI subprocesses and owns
  an in-memory job map. Jobs and durable runs have different IDs; a job does not
  have a run ID while running. Browser summaries are handled separately.
- `crates/pactrail-cli/src/commands.rs`: much of run setup, provider construction,
  persistence, resume, and completion projection lives in the CLI crate.
- `crates/pactrail-engine/src/{engine,controller,adaptive,verification}.rs`:
  deterministic execution, existing progress guidance, bounded routing, and
  verification already operate behind the tool and policy boundaries.
- `crates/pactrail-models/src/types.rs`: normalized `Usage` uses integer fields.
  Missing provider fields can lose their absence when normalized to zero.
- `crates/pactrail-context/src/lib.rs`: exact hashing, cached analysis, graph
  retrieval, and bounded context compilation already exist.

The browser's one-active-job check is local to that server. It is not a shared
workspace coordinator for every CLI and SDK process. Existing run ownership
locks remain useful, but do not solve this front-end coordination problem.

## Lessons from the reviewed harnesses

| Primary source | Useful lesson | Application to Pactrail |
| --- | --- | --- |
| [OpenCode TUI](https://opencode.ai/v2/docs/cli/tui/) | Discoverable commands, multiline input, external editing, and steering make the interaction continuous. | Ship composition improvements now; add steering only after it has a durable safe-boundary protocol. |
| [OpenCode server](https://opencode.ai/docs/server/) | A client/server split supports several interfaces over one execution service. | Extract a workspace runtime rather than keep separate CLI/browser run ownership. |
| [Aider commands](https://aider.chat/docs/usage/commands.html) | Explicit task, model, editor, and review commands reduce workflow friction. | Keep concise, discoverable actions and readable candidate review. |
| [Aider repository map](https://aider.chat/docs/repomap.html) | Budgeted structural context is useful when the whole repository cannot fit. | Improve retrieval selection using Pactrail's existing exact index and graph. |
| [Pi terminal usage](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/usage.md) | Normal scrollback, editor composition, and explicit continuation/branching support long work. | Preserve ordinary scrollback now; make continuation and branching explicit durable operations later. |
| [Crush](https://github.com/charmbracelet/crush) | Session navigation, model portability, and language-server context belong in the developer workflow. | Share session/run projections; introduce governed language-server execution only through the existing process policy. |
| [Gemini CLI shortcuts](https://geminicli.com/docs/reference/keyboard-shortcuts/) | Consistent editing, history, cancellation, and navigation shortcuts reduce memorization. | Use familiar keys and cancel by default; document terminal-specific alternatives. |
| [Goose](https://github.com/aaif-goose/goose) | Extensibility is a first-class harness concern. | Reuse governed MCP and the SDK, without a second privileged tool path. |
| [OpenHands SDK](https://docs.openhands.dev/sdk) | Separating agent software from its consuming application supports embedding. | Make runtime operations available through a stable Rust facade and bounded local protocol. |
| [SWE-agent](https://github.com/SWE-agent/SWE-agent) | Issue-solving agents need reproducible evaluation. | Judge improvements by independently checked issue outcomes and retained trajectories. |

This is a representative review of major public harnesses, not an assertion
that every repository on the internet has been audited. The architecture below
is a proposal inferred from these lessons and Pactrail's implementation gaps.

## Architecture

```text
Terminal / Browser / Editor / Automation
                 |
        Typed workspace protocol
                 |
    Workspace coordinator + durable command journal
                 |
    Run supervisor + shared run projection
                 |
    Evidence-driven controller and context selector
                 |
    Existing model drivers / tool kernel / policy
                 |
    Existing candidate / effect fences / verification
                 |
          Receipt-bound human decision
```

### 1. One workspace runtime

Extract run construction and persistence orchestration into a new
`pactrail-runtime` crate. Keep the engine free of transport and UI policy. The
CLI, web server, and SDK become clients of the same runtime operations.

A coordinator owns a canonical workspace and an exclusive workspace lease.
Initially permit one executing run per workspace, while retaining multiple
read-only review clients and pending candidates. Do not silently add a queue.
A start request allocates the durable run ID before execution, so accepted job
responses can identify the run immediately.

Use a private Unix socket or Windows named pipe for local clients; keep the
browser's existing loopback HTTP adapter, origin checks, and CSP. Any loopback
IPC alternative needs an unpredictable session credential and strict origin
validation. Do not enable remote binding as a side effect of this refactor.

A request should carry:

```text
command_id, workspace_id, run_id, expected_sequence, expected_head, command
```

Proposed commands: Start, Observe, Cancel, RequestApproval, DecideApproval,
Steer, PauseAtBoundary, Resume, Apply, Discard. Each mutation validates workspace
identity, owner, expected head, capability, and lifecycle before admission.
Persist command admission and result; repeat `command_id` returns the existing
result rather than executing twice. Unknown-result commands are reconciled
against the journal and trace after a crash. Rejected commands remain visible.

Closing a client must have an explicit policy. Preserve cancel-on-exit for an
attached CLI run initially; offer durable detach only as an explicit operation.
Service death still invokes checkpoint recovery and uncertain-effect refusal.

### 2. Shared projections with honest measurement

Reduce the hash-linked event stream into one versioned `RunSnapshot`: state,
phase, active tool, job/run identity, summary reference, evidence coverage,
candidate revision, terminal time, and usage/cost coverage. Persist the final
summary and usage with provenance rather than depend on a browser session.

Add field-presence metadata to normalized usage and preserve it through every
provider adapter, stream accumulator, router, checkpoint, ledger, SDK, and
projection. Zero remains measured zero; absence remains absent; partial totals
remain partial. Legacy records have unknown coverage rather than invented full
coverage. A cost cap must retain conservative reservations when usage is
missing; do not release a reservation merely because normalization returned 0.

Serve events from an explicit sequence cursor with bounded batches. Snapshot
sequence and head allow gap detection, bounded catch-up, and duplicate rejection.
Transient output is a separate bounded channel, never execution authority.
Artifact reads require run membership, digest integrity, size limits, and
redaction policy. No arbitrary filesystem path or unbounded model output route.

### 3. Steering without weakening the contract

Accept follow-up instructions while execution continues, but apply them only
at a documented durable boundary after the complete model turn and its admitted
tools settle. Record requested, admitted, applied, and rejected steering events.
A text instruction cannot widen write paths, process authority, tool grants,
model limits, or cost limits. A contract change needs a separately approved new
contract/run. Pause must not freeze or reset the existing budget by accident.

Safe branching should create a new isolated transaction and parent reference;
never rewind live host effects. Bind its baseline, candidate provenance,
remaining policy, and model configuration explicitly. Branching is a later
milestone, after coordinator and steering crash tests pass.

### 4. Spend context on unresolved obligations

Extend existing retrieval rather than add an opaque vector database. Maintain a
bounded ledger of unresolved obligations, relevant symbol locations, exact
observation digests, candidate revision, and diagnostic deltas. Select the next
context pack by a declared, deterministic policy using those facts. Preserve
stable tool/system prefixes and scoped instruction precedence.

Use recorded dependencies to invalidate cached observations and verification
results after writes. Cache keys must include candidate digests, command,
backend/image identity, tool version, and relevant environment identity.
Filesystem watchers may suggest dirty paths; they cannot replace exact-byte
validation at authority boundaries. Network-dependent or secret-dependent
checks are not reusable without a sufficiently strong declared identity.

### 5. Acceptance checks as a first-class workflow

Build a guided acceptance-plan interface over the existing task-specific check
mechanism. Show each obligation's proposed check, reproduction command, scope,
permission requirement, and what it can actually prove. A human-approved or
externally supplied check may support deterministic evidence. A model-generated
check is not automatically an independent oracle just because it passed.

Run approved checks in disposable verification workspaces bound to candidate
revision. Invalidate stale passes on any relevant write. Let the existing repair
loop consume bounded diagnostic deltas and stop when budgets or declared repair
limits are exhausted. Report general repository regressions separately from
proof of the requested behavior.

### 6. Routing based on measured outcomes

Extend the existing bounded investigation route only after complete usage and
held-out evaluations exist. Keep model identity and capabilities explicit;
never silently substitute providers, relax schemas, or bypass approval to make
an unreliable model appear compatible. Start with offline-selected policies,
not online self-modifying routing. Include escalation cost in reservations and
record the reason for every route decision.

## Implementation order and exit gates

| Milestone | Deliverable | Required gate |
| --- | --- | --- |
| A | Runtime extraction, workspace lease, durable command IDs, immediate run IDs | CLI/web/SDK share identical lifecycle; simultaneous starts admit one; duplicate commands cannot duplicate effects; historical fixtures pass. |
| B | Usage presence, shared snapshots, persisted summaries, cursor-based events | Missing/zero/partial usage survives every adapter and resume; missing usage cannot under-release a capped reservation; reconnect has no gaps or duplicate presentation. |
| C | Safe steering, exact approval ownership, pause/detach semantics | Crash injection before/after admission and safe-boundary application; no authority expansion; second clients cannot answer another owner's approval by accident. |
| D | Obligation context ledger and revision-aware check reuse | Stale/poisoned cache rejected; scoped instructions preserved; repository-scale budget and latency gates pass; task checks stay digest-bound. |
| E | Guided acceptance and evaluated routing | Independent held-out checks, full cost coverage, bounded escalation, and reproducible comparison against V2. Branching follows these recovery gates. |

Avoid combining all milestones into a giant unreviewable rewrite. Make every
milestone a separate branch/PR with compatibility evidence and a migration
rehearsal. The current CLI work is independently usable before milestone A.

## Compatibility and failure behavior

Keep existing scriptable CLI commands, exit codes, and JSON formats stable.
Introduce new protocol/snapshot versions separately. Read historical event,
receipt, request, and checkpoint codecs; do not rewrite historical hashes.
Migrate new service metadata only after complete read-only preflight, with
atomic writes and refusal of newer unsupported formats. Keep an embedded
one-shot path that uses the same runtime facade, not a second execution engine.

Exercise disk-full, permission-denied, lost socket, client death, service death,
provider disconnect, concurrent source changes, duplicate commands, stale head,
uncertain effect, owner change, and approval expiry. A client failure is never
proof that the run failed or that an apply wrote nothing.

## Measurement plan

Compare V2 and each milestone on the same frozen repositories, tasks, models,
permissions, budgets, provider configurations, and independent graders. Include
several provider protocols, small/local models, paid models when authorized,
malformed/missing-usage fixtures, and cold/warm/incremental repository cases.
Keep calibration tasks separate from held-out tasks; randomize execution order
and repeat trials. Publish raw receipts, traces, checks, pricing provenance,
coverage, failures, and confidence intervals.

Primary metrics: independently accepted tasks; regression rate; tokens and
priced cost per accepted task; false deterministic claims; recovery correctness;
and human time to locate the cause, review the candidate, and make a decision.
Secondary metrics: p50/p95 latency, first visible event, redundant reads,
repeated prompt bytes, verified cache reuse, missed dirty files, and idle/live
client memory. Missing cost coverage excludes a trial from cost comparisons,
not from correctness reporting.

Release gates are zero regression in containment, effect replay, receipt/apply,
and usage honesty. Efficiency and success-rate improvements need statistically
supported held-out results. No percentage advantage is promised in advance.

## Alternatives considered

- **More autonomous workers first:** creates ownership, budget, candidate merge,
  and replay complexity before shared single-run supervision is solved.
- **Full-screen TUI rewrite first:** improves layout but does not fix durable
  jobs, shared execution, steering, accounting, or verification coverage.
- **A more elaborate agent prompt:** does not establish identity, authority,
  evidence freshness, or reliable cost accounting.
- **Embeddings everywhere:** may improve ranking, but adds compute and opaque
  invalidation before existing deterministic retrieval is measured adequately.
- **Only a daemon:** improves interaction; it cannot itself establish task
  correctness or reduce model spend. The feedback and measurement milestones
  are necessary to judge those benefits.
