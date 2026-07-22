# ADR 0014: Capability-adaptive runtime profiles

## Status

Accepted for Pactrail 1.1.

## Context

One fixed loop shape is wasteful for a large endpoint and destabilizing for a
small local model. Model-name allowlists are brittle, provider-biased, and
incorrect for custom or fine-tuned endpoints. Unbounded multi-call responses
also let a malformed model create excessive work even though every individual
tool remains bounded.

## Decision

Pactrail derives an immutable `compact`, `balanced`, or `expanded` profile from
the effective `ModelCapabilities` after image-token reservation. Only declared
or probed context capacity, output capacity, capability provenance, and
parallel-tool support participate. Provider and model names do not.

The profile sets four controller limits:

- discovery turns reserved before implementation;
- maximum generated tokens for one turn;
- maximum tool calls accepted from one response; and
- maximum width of a parallel-safe read batch.

Parallel width is one unless `parallel_tools` is effective. Mutations remain
serial regardless of tier. Tool annotations still decide which reads are safe
to overlap. The existing contract turn/token/wall-time budgets remain outer
ceilings and cannot be enlarged by a profile.

The selection is sent through `RunProgress` and appended to the action journal
with all derived limits before model execution. Checkpoint runtime identity
already binds the effective model configuration, so resume recomputes the same
profile or fails identity validation.

## Consequences

Small endpoints enter action sooner, emit smaller bounded turns, and cannot
burst dozens of tool calls. Large endpoints can use wider evidence batches
without giving mutations concurrency. Custom and open-source models receive
the same treatment as API models with identical capabilities. An inaccurate
user declaration remains visible through capability provenance; Pactrail never
silently infers a stronger capability from branding.
