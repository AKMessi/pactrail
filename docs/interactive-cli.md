# Interactive CLI

Start Pactrail from the root of any Git or plain-directory workspace:

```console
pactrail
```

An optional positional task starts the same interactive session and executes
immediately before opening the next prompt:

```console
pactrail "Refactor the parser error type and update its regression tests"
```

The session keeps the familiar coding-agent flow—type a task and press Enter—
while retaining Pactrail's explicit transaction boundary. The model works in a
run-local copy. Source files change only after `/apply` validates the receipt,
candidate contents, and baseline state.

The activity line reports the engine's real lifecycle rather than displaying a
generic waiting animation: repository indexing, model turn number, typed tool,
changed file, verification command, and receipt sealing. Internal diagnostic
logs stay out of the normal transcript even when another tool exported
`RUST_LOG`. Set the Pactrail-specific `PACTRAIL_LOG` filter when diagnosing an
interactive session; non-interactive commands continue to honor `RUST_LOG`.

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

Some compatible APIs do not implement `GET /models`. In that case `/models`
explains that discovery is unavailable without clearing the configured model;
use `/model <known-id>` to select it directly. `/status` reports only whether
the selected environment variable is present and never displays its value.

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

When one or more candidates are waiting, the right side of the prompt displays
the review count. Commands without a run ID focus the newest ready candidate,
including after a restart or after landing another candidate. This prevents an
older review from becoming hidden behind a newer applied or discarded run.

## Commands

| Command | Purpose |
|---|---|
| `/help [command]` | Show the grouped command palette or focused command help. |
| `/status` | Show workspace, provider, model, limits, and process policy. |
| `/doctor` | Check local runtimes and explain the native isolation boundary. |
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

Command completion is available with Tab. Unknown slash commands include a
bounded typo suggestion when there is a close unambiguous match.

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

Generate completion scripts with `pactrail completion <shell>`. Supported
shells are Bash, Elvish, Fish, PowerShell (`powershell` or `pwsh`), and Zsh.
