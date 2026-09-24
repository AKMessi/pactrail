# Provider compatibility

## Built-in transports

Pactrail normalizes four protocol families through one provider-neutral
`ModelDriver` contract. Native adapters are used where compatibility layers
would lose tool IDs, cache usage, finish semantics, or continuation state.

| Endpoint | CLI provider | Protocol | Authentication | Status |
|---|---|---|---|---|
| Ollama `/v1` | `ollama` | OpenAI Chat Completions | None | First-class local default |
| OpenAI API | `open-ai` | OpenAI Chat Completions | `OPENAI_API_KEY` | Supported |
| OpenAI Responses API | `open-ai-responses` | Native Responses, buffered | `OPENAI_API_KEY` | Supported |
| Anthropic API | `anthropic` | Native Messages | `ANTHROPIC_API_KEY` | Supported |
| Gemini API | `gemini` | Native GenerateContent | `GEMINI_API_KEY` | Supported |
| vLLM, llama.cpp, SGLang, LM Studio, LocalAI | `open-ai-compatible` | OpenAI Chat Completions | Optional environment key | Supported |
| Compatible hosted gateways | `open-ai-compatible` | OpenAI Chat Completions | Optional environment key | Supported over HTTPS |

Endpoint URLs containing credentials are rejected. Non-loopback HTTP is rejected.
Responses are capped at 16 MiB, malformed tool arguments fail explicitly, and
rate-limit/server failures use bounded retries only before response acceptance.
The Chat Completions, Anthropic, and Gemini transports support explicit bounded streaming. A malformed,
contradictory, oversized, or disconnected stream fails the turn; partial text
and tool arguments never reach durable conversation or tool execution.

For the native Anthropic adapter, `--prompt-caching on` adds the documented
top-level ephemeral cache control to each Messages request. The default
leaves caching disabled. The API determines whether a prompt is long enough
to cache; Pactrail records cache reads and writes from reported usage.
See [Anthropic prompt caching](https://platform.claude.com/docs/en/build-with-claude/prompt-caching).

The opt-in `open-ai-responses` adapter uses `store: false` and replays the
provider's exact output items, including encrypted reasoning, function calls,
and function outputs across tool turns. It currently uses a bounded buffered
response; live text deltas are unavailable for this adapter. Other OpenAI
protocol selections retain their existing streaming behavior. See the
[official OpenAI function calling guide](https://developers.openai.com/api/docs/guides/function-calling)
and [conversation state guide](https://developers.openai.com/api/docs/guides/conversation-state).

Pactrail does not assume that model listing is available. `GET /models` is a UX
convenience; a configured model ID remains usable when discovery returns 404 or
another unsupported response.

## Opt-in investigation routing

Set `--investigation-model` to route only controller investigation turns to a
second model. `--investigation-provider` defaults to the primary provider;
`--investigation-base-url` and `--investigation-api-key-env` override its
endpoint and key variable. Implementation, validation, synthesis, recovery,
and model probes use the primary model. Pactrail does not silently fall back
to another route after a provider error. The route, provider, and model are
recorded in the model trace, and both model identities are bound to resume.

```console
pactrail run --provider anthropic --model PRIMARY_MODEL --investigation-provider open-ai-compatible --investigation-model INVESTIGATION_MODEL --investigation-base-url https://models.example.com/v1 "Fix the parser"
```

The router advertises only capabilities supported by both models and uses the
smaller declared context and output limits. Set `--context-tokens` and
`--max-output-tokens` to values valid for both configured models. Routing is
opt-in for a run; interactive settings continue to use one model.

## Explicit cost accounting

Provide all four current rates in micro-US dollars per million tokens. For
example, a rate of $0.20 per million tokens is `200000`. Pactrail never
guesses prices from a model name. `--max-cost-microusd` sets a cap for a
generated task; a task file may set `budget.cost_microusd` instead.

```console
pactrail run "Fix the parser" --model MODEL_ID --input-price 200000 --cached-input-price 20000 --cache-creation-price 250000 --output-price 1000000 --max-cost-microusd 500000
```

The estimate uses provider-reported input, cache-read, cache-creation, and
output tokens. Anthropic cache tokens are added to its uncached input count;
Gemini thinking tokens are included in output. The cap is checked after each
response, so one response can exceed it. A cost-capped run fails if a provider
omits usage. Tiered pricing and separately billed provider features are outside
this estimate; set rates conservatively when a hard spending ceiling matters.

For investigation routing, provide all four `--investigation-*-price` rates
as well as the primary rates. Pactrail uses the larger rate for each token
category across the two models. This is a conservative estimate that can
overstate actual routed cost; it cannot understate cost from choosing a cheaper
route when the declared rate cards are accurate. Without two complete cards,
cost accounting is unavailable and cost-capped runs are rejected.

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

Use native hosted adapters without an OpenAI compatibility shim:

```text
/provider anthropic
/model claude-model-id
/key-env ANTHROPIC_API_KEY
```

```text
/provider gemini
/model gemini-model-id
/key-env GEMINI_API_KEY
```

`/stream on|off` is explicit and persistent. Pactrail never retries a rejected
stream using a different protocol, provider, model, or weaker tool mode.

## Capability profiles and probes

The effective profile independently records native tools, parallel tool calls,
structured output, vision, prompt caching, streaming, reasoning controls,
context capacity, and output capacity. `/status` shows the profile and its
`auto`, `on`, or `off` provenance. Override one fact explicitly with:

```text
/capability parallel-tools off
/capability vision on
/capability prompt-caching auto
```

The equivalent one-shot flags are `--native-tools`, `--parallel-tools`,
`--structured-output`, `--vision`, `--prompt-caching`, and
`--reasoning-controls`, each accepting `auto`, `on`, or `off`. Contradictory
profiles fail before network access or durable run creation.

`/probe` spends one bounded model turn with a synthetic read-only tool. Returned
calls are normalized but never executed. It can positively observe native tool,
parallel-call, streaming, and cache behavior; a missing observation is always
reported as inconclusive. The scriptable form is:

```console
pactrail probe --provider gemini --model MODEL_ID --output json
```

## Image input

Pactrail exposes vision as an explicit model capability, not a provider-name
assumption. Enable it only for a compatible model, then attach one or more local
images:

```text
/capability vision on
/image add screenshot.png
/image list
Inspect the screenshot and fix the layout bug.
```

The non-interactive equivalent is:

```console
pactrail run --provider anthropic --model MODEL_ID --vision on --image screenshot.png "Fix the visible layout bug"
```

The portable envelope is intentionally narrower than any individual provider:
PNG, JPEG, or WebP; at most four images; at most 4 MiB decoded per image and
12 MiB decoded in total; non-zero dimensions no larger than 8,000 pixels per
edge. Pactrail recognizes the byte signature and bounded header/container
structure instead of trusting the extension. Symlinks and special files are
rejected. Duplicate content is rejected by digest.

The CLI erases the host path after reading. A portable filename, media type,
dimensions, byte count, BLAKE3 digest, and base64 payload live in the ordered
provider-neutral user turn and its local checkpoint. OpenAI-compatible drivers
emit a base64 data URL, Anthropic emits a base64 image source block, and Gemini
emits `inlineData`. No adapter fetches a remote image URL or uploads to an
implicit provider file store.

Pactrail reserves a conservative visual-token estimate before repository
context compilation. A declared context window too small for the images and
output reservation fails before model access. Provider token accounting remains
authoritative. Every first-party adapter also rejects a serialized inline
request above 20 MiB. Because inline bytes are resent with conversation history,
image runs can cost more and take longer; use only the visual evidence the task
needs.

The wire shapes follow the providers' primary documentation: OpenAI Chat
Completions accepts base64 data URLs, Anthropic Messages accepts base64 image
content blocks, and Gemini GenerateContent accepts base64 inline data with a
20 MiB total inline request limit. See [OpenAI images and
vision](https://developers.openai.com/api/docs/guides/images-vision),
[Anthropic vision](https://platform.claude.com/docs/en/build-with-claude/vision),
and [Gemini image understanding](https://ai.google.dev/gemini-api/docs/generate-content/image-understanding).

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
results, redact secrets from errors, enforce bounded response bodies and event
frames, reject insecure remote transport, and provide deterministic buffered
and fragmented-stream fixtures. Native adapters must preserve protocol-specific
continuation data without allowing it to influence tool policy or execution.
