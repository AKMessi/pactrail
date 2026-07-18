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

Pactrail does not assume that model listing is available. `GET /models` is a UX
convenience; a configured model ID remains usable when discovery returns 404 or
another unsupported response.

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

## Running a local GGUF model

Pactrail is a harness, not a GGUF inference runtime. Start the model with a
server that exposes the OpenAI Chat Completions tool-calling protocol, then point
Pactrail at its loopback `/v1` URL. For llama.cpp-style servers:

```text
/key-env PACTRAIL_LOCAL_API_KEY
/connect http://127.0.0.1:8080/v1 model-id
/context 4096
/output-tokens 512
/turns 8
/process off
```

If the server requires a bearer header, set the selected variable to a non-empty
local placeholder. If it accepts unauthenticated loopback requests, leaving the
variable unset is supported by the `open-ai-compatible` configuration.

Tool quality is model-dependent. Very small models may repeat an invalid call or
provide host-absolute paths. Pactrail returns a model-safe path correction,
stops bounded non-progress loops, and preserves any coherent candidate changes
for explicit `/diff` and `/apply` review.

The declared context and output values must reflect the server configuration.
Pactrail uses them to budget repository context and reject impossible output
limits; the inference server remains authoritative for tokenizer-specific limits.

The model-request deadline defaults to 300 seconds. CPU-only serving of large
local models can exceed that on the first prompt. Increase it for an individual
scripted run with `--request-timeout-seconds`, or set
`PACTRAIL_REQUEST_TIMEOUT_SECONDS`; Pactrail rejects values outside 1–3,600
seconds so an unavailable endpoint cannot wait forever:

```powershell
pactrail run --provider open-ai-compatible `
  --base-url http://127.0.0.1:8080/v1 `
  --model model-id `
  --request-timeout-seconds 900 `
  "Describe this repository"
```

Some compatible providers default to a reasoning mode that carries
provider-specific hidden state between tool calls. For providers implementing
DeepSeek's OpenAI extension, pass `--disable-thinking` to send
`{"thinking":{"type":"disabled"}}`. Pactrail never sends this non-standard
field unless it is explicitly requested:

```powershell
pactrail run --provider open-ai-compatible `
  --base-url https://api.deepseek.com `
  --model deepseek-v4-pro `
  --api-key-env DEEPSEEK_API_KEY `
  --disable-thinking `
  "Describe this repository"
```

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

The current adapter is non-streaming. Native Anthropic/Gemini protocols, SSE
streaming, prompt-cache controls, and provider-specific token counting are
tracked as roadmap work rather than emulated through provider-name conditionals.
