# Design 0018: bounded agents and integrity-bound latent communication

Status: implementation in progress; experimental, off by default.
Target: 2.1. Published 2.0.0 is immutable.

## 1. Problem and research hypothesis

Specialists need independent conversations, bounded cooperation, and auditable
attribution without duplicating Pactrail's authority plane. The research question
is whether a bounded internal-state channel conveys useful task information at
comparable resources, not whether moving tensors is possible. Shipping a runtime
does not establish a reasoning, latency, or cost improvement.

## 2. Non-goals

No recursive spawning, distributed agents, uncontrolled concurrent writes,
cross-checkpoint/model translation, learned compression, automatic Apply,
decoded latent summaries, or new filesystem/process APIs. Initial scheduling is
serial. Parallel reasoning can follow measured replay and cancellation tests.
No implicit transport fallback. A CPU mock is runtime-test infrastructure only.

## 3. Trust boundaries

Models and their text/tensor outputs are untrusted. A local inference backend is
trusted to report its checkpoint identity and implement its declared fusion
semantics; Pactrail cannot attest an arbitrary process's neural computations.
Descriptors and bytes remain validated even for that backend. Agents never own
workspace, policy, evidence grading, approvals, or source landing. Latents are
advisory information, never evidence or permissions.

## 4. Agent execution and scheduling

A deterministic coordinator selects stable run-local IDs and a fixed bounded
coding topology. Localizer, solver, and critic are read-only; implementer may
request candidate mutations through the existing registry; optional model
verification remains advisory and separately permission-bound. The existing
kernel verifier produces actual evidence. Every participant has independent
conversation state, a route, explicit authority restrictions, and a turn limit.
Roles do not grant authority. Configuration is validated before provider I/O.
Small tasks remain single-agent by default. Scheduling order, round progression,
message IDs and deliveries are deterministic; model responses need not be.

## 5. One authority plane

Agent restrictions intersect tool descriptor capabilities and the task policy.
Read-only annotations alone cannot grant process, MCP, secret, external-write or
network authority. A disallowed tool request is rejected before execution even
if the model invents a tool absent from its catalog. All permitted calls use
normal schemas, policy audit, approvals, safe paths, cancellation and effect
fences. Unknown effects still prevent recovery. Mutations remain serialized.

## 6. Communication

Modes are single, text and latent. Single preserves the existing path. Text and
latent use the same topology and receiver authority. Routing is a bounded
directed graph, not default broadcast. Messages have run/sender/receiver/round/
sequence identities. Text is labelled untrusted peer advice and stored by
artifact reference, not as evidence. Agent-generated text usage is reported with
coverage: provider output usage is not an exact tokenizer count of a message.

## 7. Latent representation and model extension

An optional provider-neutral extension negotiates an exact checkpoint digest,
architecture, tokenizer digest, representation/fusion version, layer, dtype,
hidden width, and slot ceiling. Ordinary ModelDriver implementations need no
latent implementation. Tensor bytes never enter ordinary conversation IR.
The initial codec is selected slots: a bounded dense [slots, hidden_width]
little-endian tensor. No implicit conversions, lossy truncation or executable
deserialization. Raw tensors are not arbitrary full activation dumps. Only
tested codecs are accepted. Cross-model translation is explicitly unsupported.

The extension must consume validated imported slots in model execution and
export internal state without first generating/embedding a prose message.
Hosted adapters do not have that capability. The mock exercises this contract
but is never selected as a user model. A genuine local adapter is a separately
gated integration, not a fake HTTP embedding shim or a launch claim.

## 8. Storage and durability

Use the existing content-addressed artifact store. Validate lengths before
allocation and bound decompression by the descriptor, with digest verification.
State envelopes bind run identity, source agent, model/format identity, shape,
dtype, logical size, stored size and digest. Receiver delivery records bind the
full descriptor, not just tensor content (identical tensor bytes can occur in
different runs). Retention follows run/checkpoint artifacts; no eager garbage
collection of referenced tensors. Unnamed crash orphans are safe to retain.

Agent checkpoint schema 1 wraps the unchanged RunCheckpoint plus coordinator
state. Existing checkpoint artifacts/readers remain accepted. The existing
CheckpointCreated variant names the new bundle using a distinct versioned
prefix. Typed lifecycle/communication records use a versioned ActionCompleted
extension to avoid breaking the stable Rust RunEvent enum or event envelope.
Agent transition events and their naming checkpoint must commit atomically.
Artifact bytes are persisted first; sequence/head checks prevent competing
writers from committing a stale batch.

## 9. Recovery semantics

Persist independent conversations, agent lifecycles, reserved attempts, rounds,
message descriptors, delivery cursor and accounting alongside candidate and
runtime identity. Validate every referenced artifact and model identity before
resuming. Before-model checkpoints may repeat an interrupted probabilistic
invocation, spending resources again; reserved attempt limits remain consumed.
Completed provider usage is checkpointed before message preparation, so a crash
between artifact storage and delivery does not erase billed inference. Recovery
may recompute the unfinished reasoning transition using another reserved attempt.
Completed deliveries are not delivered twice. Artifact-store/delivery crashes
leave either an orphan plus the old checkpoint or the whole new transition.
BeforeTools and uncertain effects still fail closed. Unknown provider billing
remains unknown; cost-capped failed invocation recovery remains refused.

## 10. Accounting

Global text/token/time/cost constraints remain the engine's existing ledger.
Additional hard limits cover agents, attempts per agent/global attempts, rounds,
messages, text bytes, logical latent bytes, stored latent bytes, per-message
bytes, and slots. Reserve before I/O/admission; reject overflow, never saturate
it into apparent success. A fan-out is multiple deliveries and is charged as
such. Latent compute/export/import time and operation counts are distinct from
provider token usage. Absent compute/peak-memory measurement is absent, not zero.
Zero intermediate generated language is claimed only for a channel that never
decodes its messages. Tool/final human text and model computation still cost.

## 11. Provider compatibility

Text uses existing native and text-action transports. Phase routing must retain
its declared response identity. Latent mode requires explicit extension support
and exact compatible representation metadata; a router without such support
rejects it. No provider-specific behavior enters core, policy or workspace.

## 12. Backward compatibility

Stable ModelRequest, ModelResponse, ModelCapabilities, ConversationItem,
TaskContract, RunCheckpoint, receipts and existing event schemas remain intact.
New optional run configuration is default-absent and skip-serialized, preserving
old manifest bytes and runtime digests. Experimental SDK types are namespaced;
new formats enter the compatibility inventory with real reader fixtures and
unknown-version rejection. Old runs do not become multi-agent on resume.

## 13. Observability

Versioned agent records expose identity, role, lifecycle, round, route, attempts,
message sequence, sizes/digests and usage coverage. CLI trace/inspect JSON retains
them; concise human status summarizes agents and communication. Raw vectors and
credential-bearing provider state never appear in standard output. Model and
tool actions retain parent run and agent attribution. Human interpretation of a
latent is not generated by default and would be explicitly labelled if added.

## 14. Security analysis

Checked dimensions/byte products, exact dtype/codec identity, strict schemas,
bounded metadata and bounded artifact reads reject allocation/decompression
bombs. Run/sender/message identity rejects cross-run injection and replay.
Receiver authority is independent of sender role; peer suggestions cannot grant
capabilities. Exact checkpoint/profile/artifact bindings reject corrupt state.
No anonymous broadcast, agent-selected child creation, executable pickle, network
downloads, or hidden queues. Backend cancellation must drop pending work and
release backend resources according to its extension contract. A remote backend
can lie about weights or representations; capability metadata alone is not
attestation. Qualification must hash local weights/tokenizers and test fusion.
A statically linked provider is trusted host code and, like any plugin compiled
into the process, can bypass Rust traits using host APIs. These interfaces do
not sandbox such code. Model-generated proposals still have no direct tool
authority; remote untrusted state remains bounded and policy-independent.

## 15. Tests

Unit and generated-input tests cover identities, strict decoding, checked
budgets, topology, lifecycle reconstruction, dtype/shape/model/run bounds,
artifact corruption and unsupported capabilities. Integration uses real Tool
Kernel/workspace/EventStore with scripted providers and a deterministic latent
mock: governed reads/writes, read-only escalation refusal, cancellation, delivery
atomicity, crash/restart, missing/corrupt artifacts, global accounting, unchanged
single-agent behavior and historical compatibility. No GPU/provider required in
CI. A mock passing these tests proves runtime mechanics only.

## 16. Benchmarks and causal acceptance

Freeze task manifests, model/checkpoint/tokenizer/representation identities,
versions, seeds, topology, tool permissions, budgets and grading before trials.
Compare single, text, latent and bandwidth-constrained latent arms. Record real
task correctness/regressions, usage coverage, attempts, tools, elapsed/inference
time, bytes/messages/rounds, memory when measured and priced cost when reported.
Retain raw traces, receipts, candidates and resource logs. Use external graders
not visible to agents. Equalize or explicitly normalize all resource budgets.

Interventions: correct, none, zero, seeded random, shuffled and wrong-task.
Wrong-task substitution is an explicit benchmark operation that creates a new
valid run-bound descriptor; production does not accept cross-run descriptors.
Random values must be finite in the negotiated dtype. Report paired differences
and uncertainty, every failure and coverage gap. Flat correct/ablated results
are evidence against causal channel usefulness. Do not extrapolate paper results
or mock scores to repository-task performance.

Research grounding: [Coconut](https://arxiv.org/abs/2412.06769) motivates hidden
state feedback; [LatentMAS](https://arxiv.org/abs/2511.20639) motivates compatible
internal collaboration; the [communication taxonomy](https://arxiv.org/abs/2606.05711)
separates representation, alignment and fusion; [causal/adversarial analysis](https://arxiv.org/abs/2512.21711)
motivates interventions rather than assuming latent tokens encode useful work.
These are research hypotheses and reported experiments, not Pactrail results.

## 17. Rollout

1. Ship validated experimental types, bounded storage, atomic checkpoint batches.
2. Integrate opt-in serial text specialists in the existing engine and SDK/CLI.
3. Test latent transport/recovery using the CPU mock, rejecting hosted providers.
4. Integrate and qualify an explicitly supported open-weight backend separately.
5. Run matched, causal real-model repository experiments before headline claims.
6. Consider parallel read-only reasoning only after measured replay guarantees.

v2.1 release readiness requires all normal quality gates and a final security,
durability, compatibility and accounting audit. A public launch must distinguish
working text collaboration, latent runtime support and actual model qualification.
