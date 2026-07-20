# Design 0002: durable checkpoints and effect-safe resume

Status: accepted for implementation on `feat/v05-resume`
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
or host path. It contains the provider-neutral conversation, token usage,
controller counters, seen tool-call IDs, accepted verification-gate metadata,
and the next safe phase.

## Safe points and effect fencing

Automatic checkpoints are written:

- after deterministic context construction and before the first model request;
- after a model response is normalized but before requested tools begin;
- after every complete tool batch and its results are journaled;
- after a repair prompt is assembled;
- before deterministic final verification; and
- after a terminal receipt is sealed.

Every effectful tool call receives a write-ahead `EffectPrepared` event bound to
its call ID, tool name, argument digest, candidate digest, risk class, and
backend profile. `EffectCompleted` records the reconciled action/result digest.

On restart:

- a call with no prepared event was never admitted and may be executed;
- a completed call is reconstructed from its checkpoint/result and is never
  executed again;
- an incomplete read-only call may be re-executed;
- an incomplete candidate mutation is not re-executed. Pactrail captures the
  current candidate, reports an uncertain result to the model, and requires
  inspection before another mutation;
- an incomplete process or external effect suspends automatic resume. The
  candidate remains reviewable, and a user must explicitly abandon the effect
  or the run. Pactrail does not pretend it can infer an external side effect.

This favors a possible skipped action over a duplicated action.

## Lifecycle

`Interrupted` is a resumable non-terminal state. A normally running process does
not eagerly write it; startup recovery projects any non-terminal run without an
active owner as interrupted. Acquiring a resume lease appends
`Interrupted -> Executing` only after the checkpoint and transaction validate.

Terminal `Failed`, `Cancelled`, `Completed`, `Applied`, and `Discarded` runs do
not resume. `AwaitingApply` is already recoverable through review/apply and does
not enter the model loop again.

Only one local owner may resume a run. The event store uses an atomic lease with
an owner nonce and bounded expiry; stale leases can be replaced, while a live
lease produces an actionable conflict instead of two model/tool loops.

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
- crash after normalized model output retains the same tool calls;
- crash before/after each read-only tool can resume safely;
- crash around candidate mutation never repeats the mutation;
- crash around process execution never repeats an uncertain process;
- cancellation and wall-time cleanup still produce terminal, non-resumable runs;
- older event/checkpoint-free runs remain inspectable;
- Windows, Linux, and macOS use identical portable checkpoint bytes.

## Deliberately excluded from v0.5

Distributed leases, remote state stores, cross-machine migration, background
agents, automatic replay of uncertain external effects, and provider-specific
conversation serialization are not part of this milestone.
