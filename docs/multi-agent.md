# Experimental bounded agents (planned v2.1)

Single-agent execution remains the default. The experimental coordinator uses
independent conversations in one run and one isolated candidate. Scheduling is
serial and deterministic; models cannot spawn participants or change routing.
Agent tool batches are also serialized, even if a custom tool advertises parallel
safety. Ordinary single-agent scheduling retains its existing behavior.

## Text coding profile

```console
pactrail agent-template > agents.json
pactrail run "Fix the parser regression and add a test" --agent-config agents.json --provider open-ai-compatible --model YOUR_MODEL --base-url YOUR_ENDPOINT --api-key-env YOUR_KEY_VARIABLE --max-turns 24
pactrail agents RUN_ID
pactrail agents RUN_ID --json
pactrail trace RUN_ID --json
pactrail resume RUN_ID
```

The built-in profile orders localizer → solver → critic → implementer. Only the
implementer may request mutations or commands, and the task's ordinary policy
must still authorize them. Commands are not enabled by selecting a profile.
The kernel verifier performs approved checks separately and produces evidence;
a model's verification review is advisory. Additional read-only reviewers may
be explicitly configured before the final implementer. All agent IDs and turn,
round, message and byte ceilings are validated before inference.

The JSON file is admitted once. Its contents, not its path, become part of the
durable run manifest and runtime identity. Editing that file cannot silently
change an existing run. Per-agent attempts include an interrupted invocation;
resuming does not restore those attempts. Multi-agent execution adds calls and
can cost more or work worse; no improvement is implied by using specialists.

Peer advice travels by addressed artifact reference. Text tokens are reported
only when a backend supplies a distinct generated-text count. Generic provider
output counts are not a reliable tokenizer count of the peer message. Unknown
communication token counts remain `not reported`; ordinary provider usage is
still charged to the existing run ledger.

## Latent semantics and availability

Latent communication means exchanging model-internal representations without
first generating an intermediate natural-language message. It is not an
embeddings API, concealed text, free inference or zero-token reasoning.
Input processing, transformer computation, latent transfer, tool text and final
human output still consume resources.

```console
pactrail agent-template --latent > latent-agents.json
```

This generates a configuration, not a latent-capable model. **All current
first-party CLI provider adapters reject latent mode.** They do not expose
internal states, including OpenAI-compatible servers that only implement Chat
Completions. There is no silent fallback. A researcher must embed a genuinely
compatible `LatentModelDriver` through the experimental SDK extension; a
qualified open-weight adapter is a separate rollout gate.

`LatentTurnContext` supplies trusted run/agent/round metadata and an explicit
human-output boundary to the backend. Intermediate collaboration rounds do not
permit a human-facing completion; the final implementer does. This flag grants
no tool authority and does not change the textual request/response IR.

The selected-slots-v1 format admits finite little-endian f32 values in a bounded
[slots, hidden_width] layout. Exact checkpoint, architecture, tokenizer, layer
and representation/fusion identity must match. The profile's
`latent_slots_per_message` controls bandwidth; unsupported slot requests fail
before invocation. No automatic cross-model conversion or dtype coercion exists.
Payload bytes live in the existing content-addressed artifact store; traces
contain provenance, sizes and digests rather than raw vectors. Latents never
become evidence or permissions.

The deterministic CPU mock in engine integration tests exercises export, store,
delivery, consumption and ordinary governed actions. It is not an inference
model and its scores cannot establish usefulness of latent reasoning.

## Recovery and failure

Agent sessions wrap an ordinary checkpoint in a schema-one artifact named by
`agents-v1:DIGEST`. Independent conversations, lifecycle, reserved attempts,
rounds, messages and delivery state are bound to the event head and candidate.
Agent transition records and their checkpoint naming event commit in one SQLite
transaction. A crash before that transaction leaves only unreachable artifacts;
delivery cannot appear without its recoverable state. The pre-invocation
attempt remains consumed if probabilistic work has to be recomputed.

Normal effect fencing is unchanged. Unknown tool completion, pre-tool boundaries,
candidate/runtime/configuration drift, missing tensors, invalid digests and
incompatible representations fail closed. Failed cost-capped model calls still
require billing reconciliation. A checkpoint is not a promise of resumability.

## SDK

`pactrail_sdk::experimental::agents` contains the versioned experimental
configuration, status and latent backend types. Existing `ModelDriver`
implementations inherit `latent_backend() -> None` and need no changes.
Implement the optional extension only if imported state actually participates
in model inference. State must be cancellation-safe and exported from internal
representations, not generated prose. `RunEngine::with_agents(config)` uses the
same registry, policy, workspace and verification path as ordinary execution.

## Verification and research

```console
cargo test -p pactrail-engine --test agents
cargo test -p pactrail-core agent
cargo test -p pactrail-models latent
cargo test -p pactrail-store prepared_batch
```

The research acceptance protocol is in
[design 0018](design/0018-bounded-agent-runtime.md). Release readiness additionally
requires compatibility fixtures, adversarial recovery tests and the full normal
quality gates. No task-success, speed, cost or causal communication result is
claimed by this documentation.

## Explicit communication interventions

The research-only `intervention` configuration defaults to `{"kind":"correct"}`.
Latent experiments can explicitly use `none`, `zero`, `random` with a seed,
`shuffled` with a seed, or `wrong_task` with a sealed donor descriptor digest.
These cannot be enabled in text mode. Shuffled permutes scalar positions within
a message; it is not a task-message shuffle. Original and altered artifacts,
intervention provenance and storage charges remain auditable. Suppressed
communication sends no imported state and consumes a bounded suppression attempt.
See [the matched runner](../benchmarks/agent-runtime-v1/README.md) for setup,
scoring, raw artifacts and the causal acceptance protocol.

Transport logical/stored-byte budgets count addressed communication and retained
research payloads, not all transformer memory traffic. Repeated backend imports
and exports are recorded separately as per-invocation operation/byte attributes;
they still consume compute and memory bandwidth. Bounded model attempts and
message shapes limit that work. Benchmark them alongside transport counters.
