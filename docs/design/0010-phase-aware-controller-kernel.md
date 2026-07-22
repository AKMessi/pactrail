# Design 0010: phase-aware controller kernel

Status: implemented for the next 1.x release

## Problem

A tool-capable model can spend a fixed turn budget exploring without ever
changing the candidate, vary equivalent read requests to evade exact-loop
detection, or claim completion without producing a change. Larger models often
self-correct, but a production harness cannot delegate progress, budget
allocation, or completion semantics to model behavior.

The controller must improve weak and strong models without adding a second
model call, provider-specific behavior, hidden filesystem mutation, or a new
durable state format.

## Decision

Pactrail owns four execution phases:

1. `investigating` gathers bounded task-relevant evidence;
2. `implementing` makes the smallest supported candidate change;
3. `validating` inspects, verifies, and repairs the isolated candidate; and
4. `synthesizing` produces a bounded informational answer without tools.

The existing maximum-turn setting remains the single external budget. The
kernel deterministically derives a discovery allowance from it: no discovery
for ceilings of four turns or fewer, two turns at eight, five at sixteen, and a
hard maximum of six. When possible, at least four turns remain for action and
finalization.

Intent classification is conservative and local. Direct interrogatives and
explain/inspect/show requests are informational; explicit mutation verbs are
change-seeking; ambiguous imperative tasks default to change-seeking. This
classification grants no capability and never broadens policy.

## Enforced action space

The active phase compiles the tool descriptors placed in each `ModelRequest`.
Investigation advertises the configured registry. Implementation and validation
advertise workspace-mutation tools plus `workspace_changes`; the first focused
turn may also advertise `read_file`. Synthesis advertises no tools.

Dispatch independently checks the call name against the compiled set. A model
that emits a remembered, hallucinated, or protocol-injected hidden tool call
receives a typed error result. The registry is not invoked and the rejection is
effect-fenced and journaled. Prompt guidance therefore explains the boundary;
it does not enforce it.

## Semantic progress ledger

For each successful tool result, the kernel hashes the tool name and canonical
JSON value while excluding the provider-generated call ID. Previously unseen
content is novel evidence. Any isolated candidate mutation is progress even if
the tool's output resembles an older result. Failed results and equivalent
observations increment a consecutive no-progress counter.

After two stagnant turns, the kernel tells a change task to implement or name a
precise blocker, and tells an informational task to answer from existing
evidence. Exact repeated calls and three all-failed turns retain their stricter
legacy recovery behavior as separate safeguards.

## Durability and observability

Phase transitions and progress assessments are synchronous `RunProgress`
events. The durable journal records `enter_phase`, `assess_progress`, phase-tool
rejections, and intervention notes. It stores only bounded reasons and digests,
not raw observations.

The controller reconstructs seen evidence and announced phases from the
checkpointed provider-neutral conversation. A new pre-model checkpoint is
sealed after phase selection, prompt insertion, and context compaction. This
keeps resume behavior exact without changing the stable checkpoint schema.

## Completion rule

An informational task may complete with an `Answered` receipt and no candidate.
A change task with no isolated changes receives a `Failed` receipt and an
explicit unresolved risk. Model prose can describe work, but cannot manufacture
a ready-to-apply transaction.

## Rejected alternatives

- **Prompt-only phases:** a model can ignore them or emit a stale tool call.
- **One fixed tool set:** discovery remains available when action is urgent.
- **Exact call equality only:** trivially varied arguments can yield identical
  evidence indefinitely.
- **A second planning model:** increases cost and latency and moves control back
  into probabilistic output.
- **Persist a new controller object:** unnecessarily changes the v1 checkpoint
  schema when the authoritative conversation already contains the ledger.

## Verification

Unit tests cover budget derivation, tool narrowing, semantic-equivalence
detection, mutation progress, and phase selection. Engine tests prove that
hidden calls are rejected despite hostile model output, the advertised tool set
narrows on schedule, progress is observable, a valid mutation still completes,
empty change completions fail, and interruption resumes from the post-control
checkpoint. Workspace tests, Clippy, rustdoc, and compatibility fixtures remain
the release gates.
