# Bounded agents: implementation and qualification audit

Status: implemented on `feat/v2.1-multi-agent`, experimental and opt-in. This is
not a v2.1 release announcement. Package versions remain 2.0.0 pending release
qualification. Preserve the published v2.0.0 tag; release this addition as v2.1
after its gates pass rather than changing an already published version.

## What is implemented

- Validated participants, independent conversations, fixed addressed routing,
  role capability ceilings, deterministic serial scheduling, cancellation and
  bounded attempts, rounds, messages, bytes and slots.
- A functioning text coding profile: localizer, solver, critic, implementer.
  Optional read-only reviewers precede the final implementer. Roles do not
  confer authority; the existing task policy and Tool Kernel authorize effects.
- A genuine latent transport abstraction: sealed, finite, bounded f32 selected
  slots bound to a run, source agent and exact representation/model identity.
  Payloads are content-addressed binary artifacts, not conversation JSON arrays.
- Optional `LatentModelDriver`, without changing ordinary textual request,
  response or conversation types. Intermediate collaboration cannot produce
  human-facing prose. Hosted providers reject latent mode without fallback.
- Agent checkpoints, atomic lifecycle/delivery events, history reconciliation,
  charged inference checkpoints, retained attempt reservations and fail-closed
  recovery. Existing uncertain tool completion rules still apply.
- Correct, absent, zero, seeded random, scalar-shuffled and explicit wrong-task
  interventions, with original artifacts, provenance and resource charges.
- CLI templates, frozen run configuration, validated human/JSON agent status,
  SDK experimental exports, compatibility fixtures, fuzz target and matched
  experiment runner with external grading and retained raw artifacts.

## Main implementation locations

| Area | Files |
| --- | --- |
| Config and invariants | `crates/pactrail-core/src/agent.rs` |
| Optional model extension | `crates/pactrail-models/src/latent.rs`, `driver.rs` |
| Scheduling, bus, lifecycle | `crates/pactrail-engine/src/agents.rs`, `engine.rs` |
| Interventions | `crates/pactrail-engine/src/interventions.rs` |
| Recovery | `crates/pactrail-engine/src/checkpoint.rs` |
| Bounded artifacts and atomic journal | `crates/pactrail-store/src/{artifact,event_store}.rs` |
| CLI | `crates/pactrail-cli/src/{cli,commands,interactive}.rs` |
| SDK | `crates/pactrail-sdk/src/lib.rs` |
| Runtime integration tests | `crates/pactrail-engine/tests/agents.rs` |
| Experiments | `benchmarks/agent-runtime-v1/` |

## Compatibility and authority review

The SDK API revision is 8. Existing model drivers inherit no latent backend and
need no dummy implementation. Stable `RunEvent`, receipt and schema-three ordinary
checkpoint formats are unchanged. Exact schema-one agent config/session/latent
descriptor fixtures are additive. Agent sessions use the `agents-v1:` reference
namespace. Lifecycle records use existing action events with versioned agent
attributes. Old single-agent fixtures remain readable.

Every agent tool batch is serialized, including custom parallel-safe tools.
Only the implementer can request writes/processes, and ordinary contract policy
must independently permit them. Read-only participants cannot launder authority
through peer messages. Verification evidence comes from the kernel, not an agent
role or latent message. Independent conversations and tensors are not printed in
standard status output.

Tensor shape products, lengths, finite values, digests, versions, model identity
and allocation bounds are validated. There is no tensor compression or executable
deserialization. Artifact reads are bounded before decompression. Normal delivery
rejects cross-run state; wrong-task research injection is an explicit recorded
exception with compatible sealed donor state. Backends/adapters are trusted host
code: metadata compatibility does not attest actual model weights or sandbox a
malicious native provider.

Delivery and checkpoint naming commit together. Uncommitted artifacts may remain
unreachable after a crash. A consumed inference attempt is never refunded;
charged responses are checkpointed before communication persistence. Missing or
corrupt artifacts, history disagreement and uncertain consequential effects fail
closed. Model recomputation is not represented as deterministic reasoning.

Provider tokens remain separate from communication counters. Unknown text
communication token counts are absent, not fabricated zero. Latent channel text
generation is zero only under the strict latent intermediate-output contract.
Logical/stored channel bytes and repeated import/export operations/bytes are
distinct. They do not measure all GPU traffic, compute or memory.

## Qualification results

Local Linux checks on the implementation:

- Full workspace/all-features locked test suite: **453 passed, 0 failed,
  1 ignored**. The ignored pre-existing OCI hostile-containment test requires
  the dedicated Docker CI job; it was not executed locally.
- Workspace/all-targets/all-features Clippy with warnings denied: passed.
- Rust formatting and whitespace checks: passed.
- The `agent_state` fuzz target compiles with its locked dependency graph.
  No local fuzz campaign was run; the scheduled CI target is configured.
- Six Python runner/analysis/real-CLI smoke tests passed against the final release build,
  using a scripted local endpoint, not a real reasoning model.
- Locked workspace release build and all-feature documentation with warnings
  denied: passed.
- Local installer smoke: accepted a checksum-verified release asset; rejected
  tampering without changing the previously installed binary.

Tests exercise role serialization, malformed descriptors, model mismatch,
oversized/corrupt artifacts, exact ledger reconstruction, atomic SQLite rollback,
read-only mutation attempts, denied-contract escalation, text and latent budgets,
round limits, cancellation, resume, missing payloads, crashes around delivery,
governed mutation and approved deterministic kernel checks. The mock causal test
changes an actual candidate file under all six interventions. That validates
channel mechanics only.

## Deliberately deferred qualification

There is **no first-party real hidden-state inference adapter**. This follows the
staged backend option in the design; neither a mock nor a Chat Completions server
is advertised as a latent model. A real adapter must validate pinned checkpoint
identity, fusion semantics, cancellation, finite bounded exports and actual use
of imported state. It must pass the same authority and recovery tests.

Real repository-task experiments, held-out grading, hardware measurements and
causal model-quality comparisons have **not** been run. The matched runner is
ready for a qualified adapter. The bundled CLI adapter supports single/text
and returns exit 78 for latent arms. No success, speed, cost or superiority claim
is justified yet. Serial reasoning is intentional; parallel reasoning and
heterogeneous representation translation remain future work.

## Commands

See [the user guide](../multi-agent.md) for single/text run configuration and
[the experiment protocol](../../benchmarks/agent-runtime-v1/README.md) for frozen
tasks, external graders, resource normalization and paired uncertainty analysis.

```console
# Existing single-agent baseline, using the configured provider/model:
pactrail run "Fix the parser regression and add a test"

# Functioning text profile:
pactrail agent-template > agents.json
pactrail run "Fix the parser regression and add a test" --agent-config agents.json --max-turns 24
pactrail agents RUN_ID --json

# Latent configuration; first-party CLI providers reject the subsequent run:
pactrail agent-template --latent > latent-agents.json
pactrail run "Fix the parser regression and add a test" --agent-config latent-agents.json --max-turns 24

# CPU channel correctness, not model-quality evidence:
cargo test -p pactrail-engine --test agents causal_channel_interventions_change_a_governed_downstream_effect

# Real experiments require a qualified latent SDK adapter and frozen protocol:
python3 benchmarks/agent-runtime-v1/run.py frozen-protocol.json --output benchmark-results/agents-trial-01
python3 benchmarks/agent-runtime-v1/analyze.py benchmark-results/agents-trial-01/results.json --control latent-correct --treatment latent-zero
```
