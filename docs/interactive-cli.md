# Interactive CLI

Start Pactrail from any Git or plain-directory workspace:

```console
pactrail
```

An optional positional task runs immediately before the first prompt:

```console
pactrail "Refactor the parser error type and add regression coverage"
```

The interface keeps the direct coding-agent flow—describe a change and press
Enter—while preserving an explicit transaction boundary. The model works in a
run-local candidate. Source files change only after `/apply` validates the
receipt, candidate contents, file modes, and source baseline.

## What the UI reports

The default run view is a persistent live execution timeline backed by engine
events rather than simulated activity. Completed rows remain visible above one
animated current-operation line. It shows:

- repository context size, cited/indexed files, compilation time, and whether
  model-budgeting omitted optional entries;
- model turn, latency, tool-call count, provider-reported tokens, and aggregate
  model time;
- the active typed tool, changed path, duration, and bounded output count;
- non-progress detection and the bounded read-only recovery turn when a weak
  model repeats an identical successful call;
- detected verification command, position, result, and duration;
- final turns, tool calls, tokens, elapsed/model time, and truncation count.

Every run opens with its durable short ID, model, and sanitized goal. Both
successful and failed runs close the timeline with aggregate turns, tools,
tokens, model time, wall time, and bounded-output count. Untrusted provider,
model, tool, path, goal, and summary text is terminal-control sanitized before
it reaches either the timeline or spinner.

The renderer reads the active terminal width. Framed dashboards, command help,
status fields, tool contracts, receipts, run history, and trace continuations
wrap deliberately in narrow terminals; long paths and URLs are hard-wrapped
instead of overflowing or disappearing. Diffs remain byte-faithful and are the
only view intentionally allowed to use the terminal's native wrapping.

`/trace` renders the complete durable timeline after a run. Its header shows the
terminal state, duration, event/action/evidence counts, and verified hash-chain
status. Every event has an explicit sequence number. Context, model, tool,
verification, policy, evidence, checkpoint, note, and lifecycle events have
distinct markers and colors. Tool-effect preparation and completion are
separate rows, so an interrupted effect is visible rather than inferred from a
missing summary. Action attributes and observed effects are shown
without persisting raw prompts, keys, or raw tool arguments.

Failure does not erase observability: Pactrail reports the run ID, exports the
portable trace, keeps that run focused for `/trace`, and lists it in `/runs`
even when no receipt could be issued.

If the process or machine stops during a safe model boundary, restart Pactrail
and continue the same durable trajectory:

```text
/runs
/resume 019f...
```

`/resume` without an ID selects the newest non-terminal executing run. It
rechecks the checkpoint, candidate, contract, provider/tool/runtime identities,
and remaining budgets before any model request. Two live Pactrail processes
cannot own one run. If the trace ends inside an effect fence, Pactrail names the
uncertain tool/risk and refuses automatic replay; use `/trace` to inspect it.

Informational prompts are first-class runs. They terminate as `ANSWERED`, issue
an integrity-checked receipt with no candidate changes, and never ask for
`/apply`. Broad workspace overviews begin with a deterministic profile derived
from root manifests and conventional entrypoints, followed by a separately
labelled model explanation. This keeps tiny-model degradation useful without
presenting model prose as kernel evidence.

Internal logs stay out of the normal transcript even when another tool exports
`RUST_LOG`. Set `PACTRAIL_LOG` for interactive diagnostics; non-interactive
commands continue to honor `RUST_LOG`.

## First session

Pactrail uses local Ollama by default. If no model is configured, startup tries
model discovery and selects the first result:

```text
/models
/model 2
/status
```

Connect llama.cpp, vLLM, LM Studio, SGLang, LocalAI, or another compatible API:

```text
/connect http://127.0.0.1:8080/v1 model-id
/context 4096
/output-tokens 512
/turns 8
```

Some compatible APIs omit `GET /models`. `/models` then reports discovery as
unavailable without clearing the configured model; select a known ID directly.

`/connect` validates and atomically persists only the provider kind, URL, and
model. Remote endpoints require HTTPS, URLs containing credentials are rejected,
redirects are not followed, and keys are read from the environment variable
selected by `/key-env`. `/status` reports only whether that variable is present.

## Task, trace, and review loop

```text
Fix the parser error conversion and add a regression test.
/trace
/diff
/apply
```

When the run stops, Pactrail prints the receipt outcome, evidence counts,
integrity status, changed paths, risks, model summary, and token usage. `/review`
combines receipt and diff. `/discard` idempotently rejects the candidate while
retaining the receipt, immutable diff, and trace; repeating it is safe. `/runs`
browses recent history.

For a repository question, use the same prompt directly:

```text
whats this directory about
/trace
```

The trace shows project-profile grounding, any model/tool activity, verification
availability, and the terminal `Completed` state.

Run IDs accept the dynamically unique prefix shown by `/runs`; Pactrail expands
time-adjacent UUIDv7 prefixes until they are unambiguous. Commands without an ID
focus the newest ready candidate, including after restart or after another
candidate is applied. The prompt's right side shows how many reviews are
waiting. Memory views show complete IDs so `/forget` never advertises an
ambiguous timestamp prefix.

## Workspace memory

Memory is explicit and provenance-aware:

```text
/remember convention Rust errors use thiserror and preserve source chains.
/remember decision Keep the public parser API synchronous.
/remember warning Do not edit generated/schema.rs directly.
/memory parser
/forget <memory-id-prefix>
```

`/remember` accepts `convention`, `decision`, or `warning`; omitting the kind
defaults to convention. Relevant entries are retrieved at task start under the
context budget and are also available through the model's read-only
`recall_memory` tool. Applied receipts create integrity-checked historical
records. The model cannot add or delete memory.

## Tool kernel inspector

`/tools` lists every model-visible tool with its capability, risk class, and
read-only/idempotent/parallel-safe annotations. The markers distinguish bounded
reads, isolated candidate mutations, and trusted host execution. This view uses
the same registry descriptors sent to the model.

Consecutive parallel-safe reads may overlap. Mutations remain serial and close
any read batch; the trace records whether each call was scheduled in parallel or
serially.

## Command palette

| Group | Command | Purpose |
|---|---|---|
| Work | `/resume [run]` | Continue an interrupted run from a proven safe checkpoint. |
| Work | `/review [run]` | Show receipt and immutable diff. |
| Work | `/diff [run]` | Show candidate changes. |
| Work | `/trace [run]` | Show the verified execution timeline. |
| Work | `/apply [run]` | Land a ready candidate after safety checks. |
| Work | `/discard [run]` | Reject a candidate and preserve evidence. |
| Work | `/runs` | Browse durable history. |
| Work | `/inspect [run]` | Show a receipt without its diff. |
| Memory | `/memory [query]` | Browse or search active workspace memory. |
| Memory | `/remember [kind] <text>` | Save a human-authored memory. |
| Memory | `/forget <id>` | Soft-delete a memory by full/unique ID prefix. |
| Model | `/models` | Discover models from the endpoint. |
| Model | `/model <name\|number>` | Select and persist a model. |
| Model | `/connect <url> <model>` | Configure a compatible endpoint and model. |
| Model | `/provider <kind> [url]` | Switch provider adapter. |
| Model | `/endpoint <url>` | Change only the endpoint. |
| Model | `/key-env <name>` | Select the key environment variable. |
| Model | `/stream on\|off` | Select bounded streaming or complete buffered responses. |
| Model | `/capability <name> <auto\|on\|off>` | Inspect or override one effective model capability. |
| Model | `/probe` | Spend one bounded turn on a no-execution capability probe. |
| Kernel | `/tools` | Inspect typed tools, capabilities, and risk. |
| Safety | `/process off` | Disable all process execution. |
| Safety | `/process native` | Select trusted, unsandboxed host execution. |
| Safety | `/process sandbox <image> [docker\|podman]` | Select restricted OCI execution with a local image. |
| Safety | `/context <tokens>` | Set declared context capacity. |
| Safety | `/output-tokens <tokens>` | Set per-turn output limit. |
| Safety | `/turns <count>` | Set the model-turn safety bound. |
| Session | `/status` | Show model, limits, policy, memory, and review state. |
| Session | `/doctor` | Inspect runtimes and isolation boundaries. |
| Session | `/help [command]` | Browse grouped or focused help. |
| Session | `/clear` | Clear the terminal. |
| Session | `/quit` | End the session. |

Tab completes commands, arrow keys browse persistent history, and Ctrl-R
searches it. Ctrl-C clears idle input or cancels an active run all the way
through provider I/O, tools, verification, and process cleanup. Safe candidate
changes remain available for review after cancellation. Ctrl-D exits. Prefix a
task with `//` when the task text itself begins with `/`. Unknown commands
provide a bounded typo suggestion when the match is unambiguous.

## Process execution and approvals

`/process off` is the default. `/process sandbox <image> [docker|podman]` uses a
locally available image through the restricted OCI profile. Pactrail pins the
resolved image identity, never pulls during a run, mounts only the candidate,
disables networking and privilege gain, clears ambient environment, and enforces
resource ceilings. If the runtime, local image, or required controls are not
available, the run fails before durable state is created; it never falls back to
native execution.

`/process native` runs registered commands directly on the host. The child is
not confined by an OS or container boundary and may reach host files, network,
operational environment, or external services. Use it only for trusted
repositories. `/process on` remains a deprecated alias for this mode.

Selecting a backend does not approve a command. When the model first requests a
process, Pactrail shows its exact program, arguments, environment-variable names,
backend identity, resource profile, and scope. Choose a one-call approval, an
exact run-scoped approval, or deny it. The decision and preceding policy
evaluation are separate hash-linked trace events. Non-interactive runs deny by
default; automation must opt in with `--process-approval allow-run`.

## Automation

No-subcommand mode requires an interactive terminal. Scripts, redirected input,
and CI should use stable subcommands:

```console
pactrail run "Fix the parser" --model qwen3-coder --output json
pactrail trace <RUN_ID> --json
pactrail inspect <RUN_ID> --json
pactrail apply <RUN_ID> --json
```

Generate native completion with `pactrail completion <shell>`. Supported shells
are Bash, Elvish, Fish, PowerShell (`powershell` or `pwsh`), and Zsh.
