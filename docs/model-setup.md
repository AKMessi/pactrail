# Connect a model

Start `pactrail` and type your task. If no model is configured, guided setup
opens and keeps your task. Or run `pactrail setup` / type `/setup` at any time.

1. Choose OpenRouter, Ollama, OpenAI, Anthropic, Gemini, OpenAI Responses, or
   another OpenAI-compatible service. Built-in choices supply the endpoint.
2. Existing provider environment keys are reused. A matching workspace `.env`
   key is offered explicitly; the rest of that file is never imported. Otherwise
   paste a key into hidden input. Ollama needs no key. Other compatible endpoints
   also support explicit no authentication (Enter at the key prompt).
3. Enter a model ID or search a part of its name; choose a numbered match.
   If discovery is unavailable, enter a known exact identifier.

The selection is saved for CLI sessions and browser bootstrap defaults. Existing
browser-local composer preferences can still override defaults. Setup changes no
process permissions and executes no model inference or tools. Discovery only
checks the catalog; the first task exercises inference/tool compatibility.

`/cancel`, Ctrl-C or EOF cancels; existing settings stay intact. A cancelled
first task stays available through `/retry`. Active `PACTRAIL_MODEL` or
`PACTRAIL_BASE_URL` overrides must be unset before setup so the saved selection
can actually take effect.

## Keys

Keys never appear in settings TOML, task history, traces, receipts or browser
responses. On Unix, pasted/imported keys are saved outside the workspace using
bounded, atomic, non-symlink local storage: 0700 directory and 0600 files. This
storage is **not encrypted**. Use an environment variable instead if you manage
secrets externally. Saved-key identifiers are bound to the exact configured base
endpoint (trailing slash ignored); changing the endpoint requires new setup.

On Windows, saved keys are encrypted with DPAPI for the current Windows user,
using the system Windows PowerShell host with profiles disabled. Plaintext keys
travel over a private pipe, never command arguments. Only encrypted content is
written to disk. If protection/decryption is unavailable, setup fails with an
environment-key alternative; it never falls back to a plaintext Windows file.

Replace an expired/wrong key with `/setup new-key` or `pactrail setup --replace-key`.
The old stored key remains intact if setup is cancelled. Keys cannot be saved
inside the workspace; use the normal user config directory or an environment key.

Remove the configured saved key with `pactrail setup --forget-key`. This never
edits environment variables or `.env` files. Keys for previously configured
endpoints can be removed by selecting that endpoint and forgetting its key.

Advanced `/provider`, `/endpoint`, `/model` and `/key-env` commands and explicit
one-shot run flags remain available. One-shot runs keep their explicit flag
semantics; use `pactrail` for the saved interactive configuration.

Endpoint references: [OpenRouter](https://openrouter.ai/docs/quickstart) and
[Ollama compatibility](https://docs.ollama.com/api/openai-compatibility).
