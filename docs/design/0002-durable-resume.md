# Design 0002: durable checkpoints and effect-safe resume

Status: implemented in v0.5
Target: v0.5

## Problem

Pactrail's event log and workspace transaction survive a process crash, but the
provider-neutral conversation and loop controller currently live only in
memory. Restarting can discover the run and candidate, yet cannot continue the
same bounded trajectory. Re-running from the original prompt would spend tokens
again and could repeat a process or mutation whose result reached the candidate
before the crash.

Resume must never convert uncertain history into authority. In particular,
"probably did not run" is not a sufficient basis for repeating a side effect.

## Durable authority

Resume uses two complementary stores:

1. SQLite remains the authoritative, sequence-checked, hash-linked lifecycle.
2. Provider-neutral checkpoints are canonical JSON in Pactrail's bounded,
   compressed, content-addressed artifact store.

A checkpoint becomes eligible only when a `CheckpointCreated` event names its
BLAKE3 digest. Artifact persistence happens before the event append. A crash can
therefore leave an unreachable artifact, but never an event that refers to bytes
which were not durably written. Loading rechecks the artifact address, schema,
run ID, event head, contract digest, candidate digest, and model/tool profile.

The checkpoint contains no API key, raw provider client object, runtime handle,
or source-workspace path. It contains the provider-neutral conversation, token
usage, controller counters, seen tool-call IDs, the pending final response, and
the next safe phase.

## Safe points and effect fencing

Automatic checkpoints are written:

- after deterministic context construction and before the first model request;
- after a model response is normalized but before requested tools begin;
- after every complete tool batch and its results are journaled;
- after a repair prompt is assembled;
- before deterministic final verification.

Every effectful tool call receives a write-ahead `EffectPrepared` event bound to
its call ID, tool name, argument digest, candidate digest, risk class, and
backend profile. `EffectCompleted` records the reconciled action/result digest.

Automatic continuation is allowed only when the current event head names a
checkpoint at a pre-model or pre-verification safe point. A pre-tool checkpoint
is retained for exact diagnosis but is not automatically executed. Any event
after the last checkpoint makes it stale. In particular, an unmatched
`EffectPrepared` event produces a specific uncertain-effect error naming the
tool and risk. Pactrail does not replay read-only calls, candidate mutations, or
processes from an incomplete effect interval in v0.5.

This favors a possible skipped action over a duplicated action.

## Lifecycle

Interrupted work remains in the non-terminal `Executing` state; `/runs` makes
that state discoverable even without a receipt. Resume appends a note and a new
head-bound checkpoint only after all restart identities validate.

Terminal `Failed`, `Cancelled`, `Completed`, `Applied`, and `Discarded` runs do
not resume. `AwaitingApply` is already recoverable through review/apply and does
not enter the model loop again.

Only one local owner may run or resume a run. A kernel file lock is the live
authority and is released immediately when the process dies. SQLite schema 2
retains bounded ownership metadata; after acquiring the kernel lock, a new
process can replace the killed owner's stale metadata immediately. A second
live process receives an actionable conflict.

## Compatibility and retention

Checkpoint schema 1 is additive. Unknown schemas fail closed. Runs created by
older versions have no session checkpoint and remain inspectable/applyable, but
are reported as not resumable. Checkpoint artifacts are capped at 64 MiB before
compression and use the artifact store's decompression and integrity ceilings.

The CLI exposes `pactrail resume <RUN_ID>` and `/resume [RUN_ID]`. Resume uses
the run's durable contract and provider profile; model or containment overrides
must be explicit and must match the checkpointed capability/profile digests.

## Required tests

- artifact persisted without a corresponding event is ignored;
- an event pointing to missing, corrupt, wrong-run, or wrong-schema bytes fails;
- contract, event-head, candidate, model, tool, and backend drift fail closed;
- two owners cannot acquire the same live lease;
- a stale lease can be recovered without racing an append;
- crash before model I/O repeats no effect;
- crash after normalized model output retains a diagnostic pre-tool checkpoint;
- prepared/completed effect ordering projects deterministically;
- any incomplete tool effect is reported and never replayed;
- a real killed CLI process resumes the same run while a concurrent live CLI is rejected;
- cancellation and wall-time cleanup still produce terminal, non-resumable runs;
- older event/checkpoint-free runs remain inspectable;
- Windows, Linux, and macOS use identical portable checkpoint bytes.

## Deliberately excluded from v0.5

Distributed leases, remote state stores, cross-machine migration, background
agents, automatic replay of uncertain external effects, and provider-specific
conversation serialization are not part of this milestone.
