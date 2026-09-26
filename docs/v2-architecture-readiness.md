# V2 architecture readiness

This document defines the implementation gate for the architecture branch. It
is separate from the model and harness benchmark session. Architecture readiness
means the provider-neutral contracts, bounded execution, efficiency controls,
evidence policy, recovery, compatibility, and platform gates are implemented
and verified. It does not claim that Pactrail is more effective or cheaper than
another coding agent on real tasks.

## Implementation scope

| Requirement | Implementation evidence | Verification gate |
| --- | --- | --- |
| Model portability | Provider-neutral requests in `pactrail-models`; Chat Completions, Responses, Anthropic, and Gemini drivers; explicit capability profiles and reasoning controls | Workspace adapter tests and cross-platform CI |
| Token efficiency | Stable tool descriptors across phases; bounded context compaction, deduplicated observation artifacts, cache coverage telemetry, and versioned request encoding | Context, model, and controller tests; descriptor budget gate |
| Cost control | Explicit rate cards and provenance, worst-case reservation before model I/O, durable per-route ledger, resume reconciliation, and bounded investigation routing | Engine cost and checkpoint tests |
| Action control | Deterministic discovery allowance, phase guidance without hiding ordinary tools, repeated-evidence detection, and a bounded edit deadline | Controller and scripted-model tests |
| Safe edits | Typed capability-gated tools, exact patch application, isolated candidate, source-drift checks, and explicit review/apply | Tool, workspace, and recovery tests |
| Verification and honest evidence | Digest-bound candidate checks, bounded repair, approval-aware disposable verification workspace, optional contract-bound acceptance checks, and inconclusive task obligations when only general repository tests pass | Engine verification tests and compatibility fixtures |
| Durable recovery | Hash-linked events, versioned checkpoint and request codecs, effect fences, crash replay, and uncertainty refusal | Checkpoint, event, transaction, and historical fixture tests |
| Process and plugin containment | Explicit native/OCI backend, exact process and MCP approvals, schema/output limits, no ambient OCI authority | Docker hostile-repository gate and dependency policy |
| Public compatibility | Documented CLI, SDK, contract, receipt, and trace formats with historical fixtures | `cargo test --workspace compatibility_fixture` and CI |

The implementation detail and threat boundary for these rows are in
[`architecture.md`](architecture.md), [`compatibility.md`](compatibility.md), and
designs [0011](design/0011-proactive-verification.md) through
[0015](design/0015-stable-tool-catalog.md). Design 0010's phase tool narrowing was
superseded by design 0015 after it prevented the model from using needed read
and search tools.

## Release gate

The architecture branch must pass formatting, all-feature Clippy and workspace
tests, rustdoc, and release build on Linux, macOS, and Windows, plus the Docker
containment and dependency-policy jobs in `.github/workflows/ci.yml`. Historical
compatibility fixtures must pass. The repository-scale budget workflow remains
a release performance gate when relevant context or tool code changes; it is a
synthetic architecture check, not the deferred model benchmark session.

Generic repository tests are not proof of the requested behavior. A receipt can
be ready for human review while its functional obligation is inconclusive.
Only a contract-bound task-specific check or later independent evaluation can
raise that conclusion. No benchmark success rate, model ranking, or superiority claim
is part of this readiness gate.
