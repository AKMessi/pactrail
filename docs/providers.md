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

## Interactive configuration

Running `pactrail` without a subcommand opens the interactive session. The
default Ollama endpoint is `http://127.0.0.1:11434/v1`; `/models` queries the
active endpoint and `/model <name|number>` persists the selection.

Configure any OpenAI-compatible server without restarting:

```text
/connect http://127.0.0.1:8080/v1 model-id
```

For hosted endpoints, keep credentials out of the URL and shell history:

```text
/provider open-ai-compatible https://models.example.com/v1
/key-env MODELS_API_KEY
/model coding-model
```

Pactrail reads the named environment variable at request time. It never writes
the secret value to settings. Plain HTTP is accepted only for loopback hosts.

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
