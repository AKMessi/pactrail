# Design 0011: proactive candidate verification

Status: implemented for the next 1.x release

## Problem

Completion-triggered verification gives the model an avoidable opportunity to
declare success before the harness discovers a compile or test failure. The
subsequent repair loop works, but spends a model turn on a false completion and
makes smaller models reason from stale confidence rather than immediate,
structured feedback.

Running checks after every write is also wrong: it can dominate latency, repeat
trusted-host process authority, and consume a run's remaining time without
improving the candidate.

## Decision

Pactrail proactively verifies at most two distinct changed-candidate digests per
run. A check is eligible only when:

- the tool turn changed the isolated candidate;
- at least one model turn remains;
- process authority was explicitly granted without another approval;
- a supported non-installing manifest check exists; and
- the exact candidate digest has not already been checked.

The first candidate consumes attempt one. If it fails with a normal non-zero
exit, the existing single automatic repair budget returns bounded diagnostics
to the next model turn. A changed repair may consume attempt two. Further edits
are still checked by the final verification gate, but cannot create an
unbounded test/model feedback loop.

## Evidence boundary

Every proactive process executes in the existing disposable candidate snapshot.
Compiler artifacts, coverage, caches, and bytecode are discarded. The process
backend, capability policy, approval audit, output bounds, timeout, cancellation,
and native-versus-OCI risk reporting are identical to final verification.

The verifier journals each command with phase `controller_gate`. The controller
then journals `proactive_verification` with attempt, command count, status,
repair decision, and the complete candidate digest. Raw stdout and stderr are
not added to the portable trace.

A passing `VerificationResult` remains in memory and becomes final receipt
evidence only if the final candidate digest is identical. Any later mutation
invalidates it automatically. When no accepted result matches, final
verification runs normally.

## Repair feedback

Only a repairable non-zero check exit can trigger automatic source repair.
Authorization denial, missing process authority, tool launch failure, cleanup
failure, or infrastructure error never instructs the model to edit code.

The repair message includes the candidate digest, diagnostics digest, original
byte count, truncation flag, and a bounded structured preview. Process output is
delimited and labelled untrusted repository data. Exactly one automatic repair
message is permitted per run.

## Restart behavior

After each proactive attempt, a controller-owned system marker records the
attempt number, candidate digest, status, and command count in the
provider-neutral conversation. It is sealed by the normal post-tool checkpoint.
Resume accepts only valid system-role markers with a 64-hex digest and restores
the maximum attempt count and latest digest. Model text cannot forge this state.

A crash before the post-tool checkpoint does not expose a resumable event head;
Pactrail's existing effect-reconciliation rule fails closed. A resumed passing
gate may be rerun at finalization to regenerate receipt evidence, but it is not
silently repeated as another proactive attempt.

## Rejected alternatives

- **Verify only after model completion:** wastes a turn on false confidence.
- **Verify after every mutation:** creates an unbounded latency and process loop.
- **Treat tool errors as repair signals:** can turn infrastructure failure into
  destructive source edits.
- **Accept model-reported test output:** does not produce independent evidence.
- **Cache by turn number:** a different candidate can exist at the same phase;
  the complete change-set digest is the correct identity.

## Verification

Engine tests prove that a broken Rust candidate is checked immediately, receives
one repair packet, is rechecked after mutation, and completes with passed
deterministic evidence. A passing first candidate runs once and is reused at
finalization. Marker parsing ignores assistant text and malformed digests. The
full workspace, strict Clippy, rustdoc, and platform release gates remain
authoritative.
