# Pactrail v2.0.0

Pactrail v2 brings the Ledger terminal experience together with durable task
continuation, bounded provider recovery, model-neutral execution and
receipt-bound review. Work remains isolated until an explicit Apply.

## Model setup

`pactrail setup` and `/setup` guide provider selection, private credential input
and searchable model discovery. First tasks survive setup cancellation. CLI and
browser share saved model defaults; permissions remain unchanged. See
[model setup](model-setup.md) for endpoint-bound key storage and platform credential storage.

## The terminal

- Open, width-aware scrollback with native foreground prose, faint metadata,
  distinct state markers, and compact activity rows.
- A running composer accepts the next draft without starting another task.
  Completion restores text and caret; Ctrl-C requests cooperative cleanup and
  retains the draft. Ctrl-D stops and exits after cleanup.
- Process approval owns input temporarily and requires the displayed challenge.
  Buffered draft text cannot approve a command. Enter denies; Apply and Discard
  also retain typed confirmations and cancellation defaults.
- Cursor-free line mode for `TERM=dumb` and `PACTRAIL_PLAIN=1`; `/dispatch` runs
  loaded drafts explicitly. Plain mode denies process prompts rather than
  blocking cancellation inside a cooked-input approval reader.
- ASCII app markers, NO_COLOR and reduced motion; Unicode paths and model prose
  are preserved. Invisible direction controls are exposed in human output.

## Recovery and memory

- Truncated buffered responses and interrupted response bodies use a shared,
  bounded adapter retry policy. Incomplete streamed tool calls are rejected.
- Output-limit recovery uses at most two accounted escalation turns; allowance
  and remaining budgets survive validated checkpoint resume.
- Failed runs can resume only at eligible durable boundaries. Event integrity,
  runtime identity, candidate contents, permissions and budgets remain binding.
  Unknown effects, corrupt chains and candidate drift are refused.
- `continue` uses explicit session focus. Completed tasks use an assessed
  follow-up with preserved authority; another terminal cannot silently redirect
  the current session's task.
- Local drafts and task context are bounded, scoped, advisory, atomically saved
  and validated. They cannot become permissions or verification evidence.
- Recovery reports retain the exact engine error and distinguish access,
  protocol, budget, context, workspace, storage and unsafe recovery failures.
  A stored checkpoint is not a guarantee of resumability or zero provider cost.

## Architecture

Provider-neutral adapters, stable tool catalogs, bounded context compaction,
content-addressed observations, provenance-aware memory, pre-request cost
reservation and hash-linked receipts remain the execution foundation. Only
candidate-bound deterministic evidence is presented as a deterministic pass;
ordinary repository checks do not prove an unrelated requested behavior.

## Upgrade and compatibility

Back up state, stop active runs, install v2, then run `pactrail upgrade`. This
release introduces no new durable event, receipt or checkpoint schema. The
historical compatibility inventory and production-reader fixtures remain binding.
Unknown versions fail closed; migration never increases authority.

The deprecated `/process on` and `--allow-process` inputs are removed. Use
`/process native`, or `--process-backend native --process-approval allow-run`
when explicitly trusting host execution. Historical stored configurations remain
readable. Human terminal output changed; scripts must use versioned JSON modes.
Restart existing sessions to load the new executable.

## Distribution and limitations

Release archives contain the binary, README, project license and third-party
notices, with checksums and provenance. Linux x86_64, Windows x86_64 and Apple
Silicon macOS are the prebuilt targets. Publishing is gated on successful CI for
the exact commit, platform packaging checks and the deterministic release soak.

Pactrail is source available under PolyForm Noncommercial 1.0.0. Previously
released license grants are unaffected; third-party terms remain in their notices.

No model is guaranteed to finish every task. Daemon detachment, implicit
permission escalation and automatic Apply are not provided. Local fixture and
architecture tests are not evidence that Pactrail outperforms another harness;
independent model/task evaluation remains a separate activity.
