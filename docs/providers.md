# Provider compatibility

## Built-in transport

Pactrail currently implements the OpenAI Chat Completions tool-calling protocol
through a provider-neutral `ModelDriver` interface.

| Endpoint | CLI provider | Authentication | Status |
|---|---|---|---|
| Ollama `/v1` | `ollama` | None | First-class local default |
| OpenAI API | `open-ai` | `OPENAI_API_KEY` by default | Supported |
| vLLM OpenAI server | `open-ai-compatible` | Optional environment key | Supported |
| llama.cpp server | `open-ai-compatible` | Optional environment key | Supported |
| SGLang | `open-ai-compatible` | Optional environment key | Supported |
| LM Studio | `open-ai-compatible` | Optional environment key | Supported |
| LocalAI | `open-ai-compatible` | Optional environment key | Supported |
| Compatible hosted gateways | `open-ai-compatible` | Optional environment key | Supported over HTTPS |

Endpoint URLs containing credentials are rejected. Non-loopback HTTP is rejected.
Responses are capped at 16 MiB, malformed tool arguments fail explicitly, and
rate-limit/server failures use bounded exponential retries.

## Adding a provider

Implement `ModelDriver` and translate the provider protocol to these normalized types:

- ordered `ConversationItem` values;
- capability metadata;
- text plus typed `ToolCall` values;
- explicit finish reason;
- input, output, and cached-token usage;
- non-sensitive response metadata.

Provider implementations must preserve assistant tool-call messages before tool
results, redact secrets from errors, enforce bounded response bodies, reject
insecure remote transport, and provide recorded protocol fixtures.

