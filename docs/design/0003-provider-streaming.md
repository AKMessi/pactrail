# Design 0003: bounded native provider streaming

Status: accepted for implementation on `feat/v06-providers`
Target: v0.6

## Problem

An OpenAI-compatible JSON response is a useful interoperability floor, but it
is not a faithful universal model protocol. Anthropic represents tool use as
typed content blocks and streams tool input as partial JSON. Gemini represents
system instructions, function calls, and function responses as native content
parts. Flattening either protocol through a nominally compatible endpoint can
discard IDs, usage, finish reasons, cache accounting, or provider-specific
continuation state.

Streaming also introduces a new trust boundary. Fragments can be duplicated,
truncated, reordered, malformed, arbitrarily large, or disconnected before a
turn becomes valid. Partial model output must improve interactivity without
becoming durable authority or reaching the tool kernel.

## Provider-neutral stream contract

`ModelDriver` retains its complete-turn interface for embedders and test
drivers. A second, observer-driven invocation method defaults to the complete
interface and can be overridden by streaming transports. It emits bounded,
typed progress only:

- response start and time to first response byte;
- text deltas;
- tool-call start and argument-byte progress, never executable partial input;
- cumulative or final usage updates; and
- non-sensitive provider status.

The driver owns a per-request accumulator. The accumulator enforces limits on
wire bytes, event bytes, text bytes, tool count, tool-argument bytes, IDs, and
provider metadata. Tool arguments are parsed exactly once, after their stream
block closes. Duplicate terminal events, missing block starts/stops, conflicting
IDs/names, non-object arguments, usage regressions, and a disconnect without a
terminal event fail the turn.

The engine receives only a complete normalized `ModelResponse`. It may expose
transient stream progress to the active UI, but appends no text or tool fragment
to the event journal and executes no tool until normalization succeeds. A
failed stream therefore leaves the preceding pre-model checkpoint as the only
resume authority. Retrying that model request can spend tokens again, but can
never duplicate a Pactrail tool effect.

## Native adapters

### Anthropic Messages

The adapter sends the required API version header, a separate coalesced system
instruction, alternating Messages content, typed `tool_use`/`tool_result`
blocks, and JSON-schema tool definitions. Its SSE state machine accepts pings
and future unknown event types, accumulates text and `input_json_delta` blocks
by index, verifies the documented start/delta/stop ordering, treats usage in
`message_delta` as cumulative, and maps provider errors without reflecting
credentials or raw request bodies.

### Gemini GenerateContent

The adapter sends the key in `x-goog-api-key`, uses a separate
`systemInstruction`, maps assistant content to the `model` role, and preserves
function call IDs through function responses. It consumes
`streamGenerateContent?alt=sse`, validates candidate/part structure, coalesces
incremental text, de-duplicates cumulative function-call parts, and accepts
usage only from `usageMetadata`. Safety-blocked or incomplete candidates become
explicit failures rather than empty successful answers.

### OpenAI-compatible Chat Completions

The existing adapter gains SSE accumulation for delta text, indexed tool calls,
partial argument strings, finish reasons, usage, and `[DONE]`. Streaming is an
explicit configuration choice. If an endpoint rejects streaming, Pactrail
reports that capability mismatch; it does not silently retry a different
protocol or model. Users can select the buffered profile explicitly.

## Capability profiles and diagnostics

Capabilities are independent facts, not a provider label. The profile records
native tools, parallel tools, structured output, vision, prompt caching,
streaming, reasoning controls, context, and output limits together with its
source: built-in conservative default, user declaration, or successful probe.
The complete effective profile is part of Pactrail's model identity and durable
resume binding.

Probes are credential-safe, read-only, bounded, and opt-in. A failed or
ambiguous probe never enables a feature. User overrides are validated against
hard limits and shown by `/status` and `/doctor`. Pactrail never silently
changes providers, endpoints, models, tool mode, reasoning mode, or stream mode.

## Cancellation, retry, and observability

The request timeout covers connection, the entire stream, and accumulation.
Dropping a cancelled stream closes the response body. HTTP retry behavior is
limited to failures before a successful response body begins; Pactrail never
retries after accepting stream fragments inside the same invocation. Retry
headers remain bounded. Endpoint diagnostics show scheme/host, adapter,
profile, status, request ID, and redacted error text—never URL credentials,
headers, key values, prompts, tool arguments, or response text.

Durable model actions record adapter, stream mode, latency to first event,
total latency, normalized byte/call/token counts, terminal reason, request ID,
and bounded protocol diagnostics. Live text is terminal-sanitized and transient.

## Compatibility and tests

The provider-neutral conversation and response schemas remain additive.
Checkpoint schema advances only if new durable fields are required. Settings
migrate atomically and old OpenAI-compatible configurations retain buffered
behavior unless the migration can prove streaming was selected.

Each adapter is tested against local scripted HTTP fixtures covering:

- fragmented UTF-8 and SSE frames across arbitrary transport chunks;
- CRLF/LF framing, comments, pings, and future unknown events;
- interleaved and parallel tool calls with partial JSON arguments;
- duplicate, missing, reordered, oversized, and contradictory events;
- usage disagreement/regression, content filtering, provider error events;
- disconnect before terminal completion and cancellation during a stream;
- credential and terminal-control redaction; and
- identical normalized conversations, tool calls, usage, and finish semantics
  across buffered and streaming fixtures where the protocols overlap.

## Deliberate exclusions

- Provider-hosted tools do not enter the tool kernel in v0.6.
- Partial assistant text is not crash-resumed as if it were a complete turn.
- A provider name does not imply undocumented model limits.
- Live bidirectional audio/video protocols are outside this milestone.
