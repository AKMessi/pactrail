# Pactrail v0.6.0

Pactrail v0.6.0 makes provider behavior live without making partial output
authoritative. It adds bounded streaming across every built-in protocol,
faithful native Anthropic and Gemini adapters, explicit capability profiles,
and an opt-in no-execution model probe.

## Highlights

- OpenAI-compatible, Anthropic, and Gemini streams are incrementally decoded
  under wire, event, text, tool-count, argument, identifier, and metadata
  limits. A tool call reaches the kernel only after the complete response has
  passed protocol validation.
- Anthropic Messages retains typed content blocks, tool-use IDs, tool results,
  API-version semantics, and cache-read usage.
- Gemini GenerateContent retains native system instructions, function IDs and
  results, safety finishes, cache usage, model metadata, and thought signatures
  required by later function turns.
- `/status` exposes the effective capability profile. `/capability` and the
  corresponding one-shot flags provide explicit `auto`, `on`, and `off`
  overrides.
- `/probe` and `pactrail probe` use one synthetic read-only tool turn. Returned
  calls are inspected but never executed, and non-observation is reported as
  inconclusive rather than silently changing configuration.
- Durable model trace rows include adapter, stream mode, provider request ID,
  bounded safe metadata, token accounting, total latency, and time to first
  response bytes where the transport reports it.

## Safety and compatibility

Partial provider output is transient UI data. It is not journaled as a complete
assistant turn, checkpointed as authority, or executable. A disconnect,
contradiction, safety block, malformed argument, oversized response, or missing
terminal event fails the turn at its preceding safe checkpoint. Pactrail never
falls back to another provider, model, protocol, or tool mode automatically.

Settings schema 4 adds capability overrides. Schemas 1–3 migrate atomically;
existing schema 1–2 configurations retain buffered transport, and schema 3
retains its explicit stream selection. Older runs and receipts remain
inspectable. Resume still requires the exact stored model profile.

## Verification

The release gate runs formatting, warning-free Clippy, the full workspace test
suite, documentation, dependency policy, Linux/macOS/Windows builds, and the
hostile-repository OCI fixture. Provider tests use local scripted HTTP servers
and recorded protocol data; they require no API key or public network call.

Install from source:

```console
cargo install --git https://github.com/AKMessi/pactrail.git --tag v0.6.0 --locked pactrail
```

See [provider compatibility](providers.md), the [streaming design](design/0003-provider-streaming.md),
the [threat model](threat-model.md), and the [changelog](../CHANGELOG.md).
