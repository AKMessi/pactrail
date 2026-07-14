# Interactive CLI

Start Pactrail from the root of any Git or plain-directory workspace:

```console
pactrail
```

The session keeps the familiar coding-agent flow—type a task and press Enter—
while retaining Pactrail's explicit transaction boundary. The model works in a
run-local copy. Source files change only after `/apply` validates the receipt,
candidate contents, and baseline state.

## First session

Pactrail uses local Ollama by default. When no model has been selected, startup
attempts model discovery and selects the first reported model. Use these
commands to inspect or change the selection:

```text
/models
/model 2
/status
```

For a local llama.cpp server or another OpenAI-compatible API:

```text
/connect http://127.0.0.1:8080/v1 model-id
```

`/connect` validates the endpoint, stores only the URL and model identifier,
and switches the provider atomically. Remote endpoints must use HTTPS, URLs
containing credentials are rejected, and API keys are read only from the
environment variable selected with `/key-env`.

## Task and review loop

Enter a natural-language task without a leading slash:

```text
Refactor the parser error type and update its regression tests
```

After the run, Pactrail prints its receipt, verification summary, changed paths,
unresolved risks, and token usage. The normal landing loop is:

```text
/diff
/apply
```

Use `/review` to show both receipt and diff, `/discard` to reject the candidate,
and `/runs` to select an older completed run. Run IDs may be abbreviated to any
unique prefix shown by `/runs`. Review diffs are immutable run artifacts, so
they remain available after apply or discard.

## Commands

| Command | Purpose |
|---|---|
| `/help` | Show the command palette and editing shortcuts. |
| `/status` | Show workspace, provider, model, limits, and process policy. |
| `/models` | Discover models from the active endpoint. |
| `/model <name\|number>` | Persist a model selection. |
| `/connect <url> <model>` | Configure an OpenAI-compatible endpoint and model. |
| `/provider <kind> [url]` | Select Ollama, OpenAI, or a compatible provider. |
| `/endpoint <url>` | Change only the active endpoint. |
| `/key-env <name>` | Select the environment variable holding the API key. |
| `/context <tokens>` | Set declared model context capacity. |
| `/output-tokens <tokens>` | Set maximum output tokens per model turn. |
| `/turns <count>` | Set the maximum model turns per run. |
| `/process on\|off` | Control trusted native process execution. |
| `/runs` | Show recent completed runs. |
| `/inspect [run]` | Inspect a run receipt. |
| `/review [run]` | Inspect a receipt and its immutable diff. |
| `/diff [run]` | Show the immutable unified diff. |
| `/apply [run]` | Land a ready transaction after safety checks. |
| `/discard [run]` | Remove a candidate while preserving evidence. |
| `/clear` | Clear the terminal. |
| `/quit` | Exit the session. |

Arrow keys navigate persistent history and Ctrl-R searches it. Ctrl-C cancels
the current input; Ctrl-D closes the session. Prefix a task with `//` when the
task itself must begin with `/`.

## Process safety

Native process execution defaults to off. `/process on` permits model-triggered
verification commands and grants them the host process's filesystem, network,
environment, and external-service authority. The isolated workspace protects
the apply boundary; it is not an operating-system sandbox. Enable processes
only for repositories and toolchains you trust.

## Automation

No-subcommand mode deliberately requires an interactive terminal. Scripts,
redirected input, and CI should use the stable subcommand interface:

```console
pactrail run "Fix the parser" --model qwen3-coder --output json
pactrail inspect <RUN_ID> --json
pactrail apply <RUN_ID> --json
```
