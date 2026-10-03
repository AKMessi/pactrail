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
receipt, candidate contents, file modes, and source baseline. Interactive Apply
first displays the candidate and evidence cautions, then asks you to type `apply`.
Enter, Ctrl-C, Ctrl-D, and any other response cancel. Discard similarly requires
`discard`. Scriptable `pactrail apply`/`discard` retain their existing semantics.

## Ledger presentation

The interactive session uses normal terminal scrollback and an open, compact
layout. Startup and approval fields stack on narrow terminals. Live activity
puts elapsed time at the right at 80 columns and wider; narrow terminals omit
that visual timestamp. Event timestamps remain in the durable trace.

The composer shows process mode, workspace and explicit continuation focus.
`?` opens help. `PACTRAIL_ASCII=1` replaces the state markers and composer
indicators with ASCII; repository paths and model prose keep their original
Unicode. `PACTRAIL_REDUCED_MOTION=1` disables the spinner's timed animation, as
`PACTRAIL_NO_ANIMATION=1` does. `NO_COLOR` disables styling. Metadata uses the
terminal's faint style rather than assuming a dark background.

Failure reports keep the original engine error and add guidance for its class.
Guidance never grants permissions, resets budgets or promises that a checkpoint
is resumable. Local task memory is advisory; the engine validates all recovery.

During execution, the running composer accepts the next draft. Enter retains it
without dispatching; completion restores the draft and caret. Ctrl-C requests a
cooperative stop and retains the draft. Ctrl-D requests stop and exits only after
cleanup. Drafts are saved on explicit submit and execution boundaries, not on
every keystroke. Closing the process is not daemon detachment.

Process approval temporarily takes input ownership. The draft is restored after
the decision. Type the freshly displayed `once <challenge>` or `run <challenge>`
to approve the exact request; Enter denies. Buffered task text is not consent.

`TERM=dumb` or `PACTRAIL_PLAIN=1` selects cursor-free line input. This mode
requires a terminal, preserves scrollback, and has no live type-ahead editor.
`/draft`, `/retry`, `/task` and `/editor` load a draft; `/dispatch` explicitly runs
it. Process requests are denied in plain mode; use cursor mode for interruptible
per-request approvals or explicitly configure a one-shot process policy. Use
`pactrail run` for noninteractive automation.

## What the UI reports

The default run view is a compact, persistent live execution timeline backed by engine
events rather than simulated activity. Completed rows remain visible above one
current-operation line. `/detail full` includes the lower-level context and
controller diagnostics; warnings remain visible in compact mode. The underlying
durable trace is unchanged. Across these views it reports:

- repository context size, cited/indexed files, warm/cold index reuse,
  kernel-derived citation coverage, graph evidence, compilation time, and
  whether model-budgeting omitted optional entries;
- sealed image count, decoded bytes, conservative input-token reservation, and
  digest prefixes without host paths or base64;
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

The renderer reads the active terminal width. Ledger blocks, command help,
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

Informational prompts are first-class runs. They terminate as `Answered`, issue
an integrity-checked receipt with no candidate changes, and never ask for
`/apply`. Broad workspace overviews begin with a deterministic profile derived
from root manifests and conventional entrypoints, followed by a separately
labelled model explanation. This keeps tiny-model degradation useful without
presenting model prose as kernel evidence.

Internal logs stay out of the normal transcript even when another tool exports
`RUST_LOG`. Set `PACTRAIL_LOG` for interactive diagnostics; non-interactive
commands continue to honor `RUST_LOG`.

## Compose without losing your place

- **Enter** dispatches a task. **Alt+Enter** or **Ctrl+J** inserts a newline.
  Shift+Enter also works when the terminal reports it as a distinct key.
- **Ctrl+P** opens described command completion; **Tab** opens/cycles it and
  Shift+Tab goes back. Escape closes the menu without executing an action.
  Run arguments complete known IDs, and `/model` completes discovered models.
- **Ctrl+G** sends the current draft to `VISUAL` or `EDITOR` and restores it when
  you exit. `/editor [text]` opens a fresh draft. Neither dispatches automatically.
- `/task "path with spaces.md"` loads a UTF-8 task file up to 64 KiB into the
  composer. Review it before pressing Enter. `/retry` restores the last submitted
  task for editing; it does not spend another model turn by itself.
- Arrow keys and **Ctrl+R** retain persistent input-history navigation.
  Ctrl+P is reserved for commands; Up remains available for history.

Editor and pager settings are trusted local executable/argument configurations,
parsed without shell evaluation. Quote a program path that contains spaces.
Editor invocation uses the selected workspace as its working directory and a
private temporary draft file. Task files must be regular files. Draft controls
are neutralized before display. These actions do not alter the agent's process
permissions.

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
retaining the receipt, immutable diff, and trace; repeating it is safe. `/runs [text]`
browses or filters history. `/focus <run-id-or-prefix>` explicitly selects the
run used by `/review`, `/trace`, `/evidence`, `/inspect`, and decisions.

Long run lists, traces, diffs, model lists, and evidence views use `PAGER`, defaulting
to `less -FRX`, when they exceed the terminal height. With less, `/` searches,
Space advances, and `q` returns to the composer. Set `PAGER` to an empty value or
use `/pager off` for ordinary scrollback. A missing/failing pager explains the
failure and displays the full output. Less runs without shell escapes or external
preprocessors. `/pager auto|off` and `/detail compact|full` affect this session only.

`/evidence` displays each obligation's grade, status, summary, reproduction
command, and artifact digest. Deterministic-passed evidence is the only green
verdict; completion, application, integrity, and configured permissions do not
use pass coloring. Apply notices include missing deterministic checks, failed
checks, and evidence preceding the latest write, derived from trace order.

Token figures are labeled **engine-counted tokens**. The current normalized
backend counters do not preserve field-presence coverage for every provider;
zero cannot always prove that a provider reported zero. Cost without a report
is `—`, not an invented zero. Complete presence-aware accounting is proposed
in [design 0016](design/0016-durable-evidence-runtime.md), not claimed as shipped.

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

## Next-task image queue

Vision input is explicit and consumed by the next submitted task:

```text
/capability vision on
/image add "C:\work\reference\broken.png"
/image list
Fix the regression visible in the screenshot.
```

`/image clear` empties the queue. Adding validates the file immediately and
shows its filename, dimensions, decoded size, and digest prefix. The queue keeps
paths only in the live CLI process; a valid task submission seals bytes into the
provider-neutral user turn and clears the queue. A preflight failure keeps the
queue available for correction. The source path is never sent or stored. The
sealed image is stored in the local checkpoint and sent to the configured
provider, so attachment is a data-disclosure decision.

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
| Work | `/runs [query]` | Browse or filter durable history. |
| Work | `/focus <run>` | Select the run used by review and decisions. |
| Work | `/evidence [run]` | Inspect obligation support and reproduction commands. |
| Work | `/retry` | Restore the last task to the composer without dispatch. |
| Work | `/inspect [run]` | Show a receipt without its diff. |
| Work | `/image add <path>\|list\|clear` | Manage sealed image evidence for the next task. |
| Memory | `/memory [query]` | Browse or search active workspace memory. |
| Memory | `/remember [kind] <text>` | Save a human-authored memory. |
| Memory | `/forget <id>` | Soft-delete a memory by full/unique ID prefix. |
| Model | `/models [query]` | Discover or filter models from the endpoint. |
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
| Session | `/status` | Show model, limits, policy, queued images, memory, and review state. |
| Session | `/doctor` | Inspect runtimes and isolation boundaries. |
| Session | `/help [command]` | Browse grouped or focused help. |
| Session | `/editor [text]` | Compose in VISUAL/EDITOR and return for review. |
| Session | `/task <path>` | Load a bounded UTF-8 task file into the composer. |
| Session | `/pager auto\|off` | Choose paging for long review views in this session. |
| Session | `/detail compact\|full` | Choose live diagnostic detail in this session. |
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
repositories. `/process on` is removed in v2; use `/process native`.

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
pactrail diff <RUN_ID> --json
pactrail runs --json
pactrail apply <RUN_ID> --json
```

`diff --json` returns the receipt outcome, integrity result, typed change set,
and immutable unified diff in a schema-versioned report. Unknown run IDs fail
without creating state. The human trace renderer uses the same deliberate
hanging indentation as the interactive timeline, including for metadata,
effects, and long integrity digests.

Generate native completion with `pactrail completion <shell>`. Supported shells
are Bash, Elvish, Fish, PowerShell (`powershell` or `pwsh`), and Zsh.

## Development verification

`cargo test -p pactrail` covers presentation, Unicode cells, described completion,
review cautions, bounded task files, safe executable parsing, CLI JSON contracts,
and engine integration. The optional development check
`devtools/check_cli_experience.py` uses pexpect/pyte in an external Python
virtual environment. It runs the real binary and a local deterministic model
fixture in disposable workspaces, checks composition and decision workflows,
exercises pager/editor return and NO_COLOR/TERM=dumb, and saves terminal captures
under `/tmp/pactrail-cli-qa`. Those Python packages are not runtime dependencies
of Pactrail. The local fixture uses port 4190.

## Budgeted completion audit in automation

`pactrail run "Fix both call paths" --completion-audit` requests up to two
revision-bound reviews before accepting a change summary. Reviews consume the
existing turn/token/cost/time allowances and preserve all tool permissions.
The option is opt-in and stored with the run for resume; interactive sessions
retain their existing policy. Review requests are advisory, never deterministic
passes. Use caller-declared acceptance checks for independent behavioral evidence.

### Workspace drafts and quick navigation

- **Ctrl+S** saves the current composer text locally, keeping it editable.
- **/draft** restores it without dispatching; **/draft clear** deletes it.
- **/retry** restores the previous submitted task even after a restart, scoped to
  this workspace. It does not resume a run or restore image attachments.
- **Ctrl+O** opens run focus in the composer. Type a goal fragment or ID, then
  **Tab** to select a run; **Enter** changes focus only. A current draft is saved
  before switching. Use `/draft` to return to it.
- Command completion supports fuzzy abbreviations; prefix matches rank first.
  `/task` completes quoted file paths without reading their contents.
- Bracketed multiline paste waits for explicit Enter before dispatching.

Drafts and previous tasks are local plaintext in the config directory, outside
source/candidate copies. Ctrl+C clears only the current editor; saved drafts
remain until replaced or cleared. Saving is explicit, not automatic on every
keystroke. See [the CLI design](cli-workflow-upgrade.md) for research and limits.

### Continue a selected task

Type **continue** or **/continue**. Interrupted executing runs and recoverable
failed checkpoints resume under the same run ID. A pending candidate opens
review without applying. An Answered/Applied task starts a fresh isolated run,
with the original contract constraints and bounded historical context; current
files and evidence must be checked again. Stopped/discarded runs and unsafe
checkpoints are not silently restarted.

Focus and task context survive restart. Use `/continue <id>` when choosing an
older task; if no focus is saved and several runs exist, Pactrail asks you to
select one. `/continue forget` clears the selected local answer record and focus,
not engine history. See [continuation details](persistent-task-continuation.md).
