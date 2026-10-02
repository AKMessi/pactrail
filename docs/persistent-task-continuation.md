# Persistent task continuation

In the interactive CLI, enter `continue` (case insensitive) or `/continue` to
continue the selected task. `/continue <id-or-prefix>` selects an explicit run.
A normal prompt containing other words remains a new task.

## What happens

| Durable state | Action |
| --- | --- |
| Executing after interruption | Ask the engine to validate and resume the checkpoint under the same run ID. |
| Failed | Attempt the existing guarded checkpoint recovery. Unsafe checkpoints, unknown capped billing, tampered state and exhausted budgets still fail closed. |
| Awaiting review | Open the existing candidate review. No new run, Apply or Discard happens. |
| Answered or Applied | Start a new isolated follow-up, preserving the previous contract's write scope, exclusions, obligations, checks, capability permissions and budget ceilings. Usage starts at zero for this new run; limits are not unlimited. |
| Stopped, Discarded, or another unsupported phase | Explain why automatic continuation is unavailable. Do not silently recreate or apply partial/discarded files. |

A completed-task follow-up starts with a completion assessment. It asks the model
to inspect current files, finish remaining
work, and gather fresh evidence. If the task is already complete, it should explain
that rather than repeat edits. Current model/process configuration is subject to
the copied contract and the ordinary engine permission checks; per-run approvals
are not copied. Apply still requires the existing explicit confirmation.

## Selecting the right task

The CLI persists focus when a run starts, when results arrive, and when `/focus`,
Apply or Discard selects a run. It restores that focus after restart and checks
that the selected run exists in this workspace's state directory. An explicitly
focused run stays selected when other review candidates exist.

Without saved focus, exactly one historical run can be selected automatically.
With several runs, the CLI asks for `/continue <id>` or `/focus <id>` rather than
guessing. Missing/corrupt focus produces a visible error. The existing engine
execution lease prevents a second process from concurrently executing the same
workspace/run.

## What memory is retained

Engine checkpoints remain the full recovery mechanism: conversation, tool
observations, candidate identity, usage, budgets and durable effect fences.
Continuation never replays completed effects itself.

For completed-task follow-ups, private local task records retain the original
root goal, root run ID and a bounded last answer summary. Records live next to
composer drafts, outside source/candidate copies, with the same atomic/private
file handling. Final records bind to the run ID and verified contract digest.
Early records bind to the started run ID and goal digest, so a crash does not
lose task ancestry; finalization upgrades the binding to the contract. Unknown
schemas, unknown fields, changed bindings and oversized records are rejected.

Historical paths, risks, verification counts and the previous answer are supplied
through the **existing advisory context compiler**, not inserted as authoritative
instructions in the user goal. Memory is explicitly labelled historical and the
model's summary untrusted. The compiler's normal context budget can omit optional
history. Old evidence is never converted into fresh passing evidence.

The root goal is capped at 16,000 bytes; larger originals require explicit editing
with `/retry` instead of silent truncation. Answer summaries are capped at 8,000
bytes with a visible truncation marker. File/risk history is bounded to 30 paths
and 12 risks. Repeated continuation carries the original root goal, not recursively
nested previous prompts. This is task context, not unlimited chat history or a
claim that a model remembers everything.

CLI-started historical runs without these local answer records still use their
verified contract and receipt. The absence of saved answer text is stated
explicitly. Persistence errors do not turn a completed engine run into a failure;
the CLI reports that local context could not be saved.

`/continue forget` removes the selected run's local answer record and saved focus.
It does not erase engine receipts, checkpoints, traces, other runs' local records,
or the separate workspace memory database managed by `/memory` and `/forget`.
Task context is local plaintext, not a credentials store.

## Verification

Unit tests cover state routing, UTF-8 bounds, root ancestry across repeated
follow-ups, provisional records, restart loading, contract/run/schema binding,
receipt integrity, decision transitions and rejection of oversized root goals.
The real-engine PTY suite covers no-history continuation, pending review without
Apply/dispatch, a completed follow-up preserving the original contract, a provider
failure followed by restart and same-ID checkpoint recovery, local forgetting,
and ambiguous-selection refusal. The deterministic fixture makes its first
`fixture-recover` request fail and its next one succeed; no live provider is needed.
