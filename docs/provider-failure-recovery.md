# Provider failure recovery

Buffered OpenAI-compatible calls share one limit of three retries after the
initial request. Truncated JSON (EOF) in the outer response or encoded tool arguments, interrupted bodies, and connect/timeout
failures can retry; complete invalid JSON, oversized responses, authentication,
and invalid requests fail closed. Requests can be billed more than once by the
provider; usage from an unreadable response is unknown, not zero. Streaming
responses already accepted by the UI are not replayed automatically.

After exhaustion, the run remains Failed and its candidate is retained. Explicit
`pactrail resume <run-id>` or `/resume <run-id>` can recover only when the very
last event is Executing -> Failed and the immediately preceding event names an
intact BeforeModel checkpoint. No backward search crosses actions or effects.
Candidate, contract, model/tool profile, checkpoint digest, permissions, leases,
and remaining budgets are still validated. Pending effects forbid recovery.
Cost-capped failed runs are rejected because unreadable billing is unknown.

The original events are never edited. A validated recovery appends Failed ->
Executing and a recovery note. Time between the last checkpoint and failure is
charged to the active wall-time budget; time spent waiting for the user is not.
Repeated failures can be resumed under the same checks and remaining budgets.
Older binaries may reject the new recovery transition, so use the updated binary
for all subsequent inspection/resume of a recovered run.

Apply requires a completed receipt. Recovery does not apply candidate files or
claim verification. After successful continuation, inspect the diff and evidence
and apply through the usual review boundary.

Tests use loopback HTTP fixtures for truncated, permanently invalid, and exhausted
responses, and real isolated workspace/checkpoint execution to prove completed
writes are not replayed. Existing checkpoint/effect/candidate negative tests
remain required. This is resilience against a known failure class, not a guarantee
against provider outages or every possible task failure.
