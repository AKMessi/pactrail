# Pactrail v2 Ledger implementation plan

The user's Ledger specification governs the visual direction. Corrections and
verified existing behavior are in [v2-audit.md](v2-audit.md). The browser UI and
private benchmark repository are outside this change.

## Design contract

Normal terminal scrollback is the session. Completed facts are appended and never
redrawn. A compact status/composer/current-operation dock holds current state.
No alternate-screen framework, background fills, decorative emoji, progress
percentages, inferred ETA, or fabricated reasoning. Existing explicit pagers
provide complete trace, diff, receipt and code detail.

Use native foreground for prose; ANSI cyan for interaction/active sections,
yellow for unresolved/stopped/warning states, red for failure, green only for
passing verification or diff additions (with explicit words/signs). Applied and
Answered remain neutral. Metadata uses terminal faint instead of assuming that
bright black contrasts with the user's background. NO_COLOR remains unstyled.

Prose measure is at most 88 cells with a two-cell state gutter. At 80+ columns
elapsed metadata can share a row; below 80 it is omitted from live rows but kept
in trace. At 40 and 32, labels and values stack. Long code/diff stays exact in the
pager; live excerpts must identify truncation. ASCII mode changes app-generated
symbols only, never Unicode paths or model text.

Status always shows process mode. Model identity and focused run are shown when
known; absent context/cost/usage does not become zero. Context gauges require
actual context measurements, not total token consumption. Selected tasks stay
session-local; saved focus is a restart default, not remote control by another
terminal.

### Reference flow

```text
pactrail · Ledger                         workspace /work/app
commands: none                           model fixture-read

◇ contract   validated task · edits isolated
● model      turn 1 · 2 actions
  read_file  src/parser.rs
  edit_file  src/parser.rs
▲ verify     inconclusive · process permission unavailable

▲ Awaiting review
  candidate  1 file · +42 / -7 bytes
  evidence   0 passed · 1 inconclusive
  /diff review · /evidence inspect · /apply · /discard

Composer · commands: none · run 019f...
pactrail ❯
```

This example uses bytes, not invented line counts. No action name or evidence is
shown unless reported by the engine.

## Failure policy

The UI classifier is explanatory. The engine remains the only authority for
retries, leases, mutations, budgets and resume eligibility. Always keep the exact
engine error with a bounded readable display and access to trace. Never replace
an uncertain outcome with an optimistic generic retry button.

| Class | Required behavior | Recovery boundary |
| --- | --- | --- |
| Connect/reset/5xx | Visible endpoint failure after bounded adapter policy | Guarded checkpoint; uncertain billing preserved |
| Rate limit | Report actual 429 and retry guidance when available | No invented sleep duration or zero cost |
| Authentication/configuration | Explain configuration/access correction | No blind retry loop |
| Buffered/streamed truncation | Discard unaccepted partial response | Never admit partial tool effects |
| Invalid calls/protocol | Show validation/controller error | Repair is a model turn, not replay of effects |
| Output limit | Incomplete answer; at most two accounted escalation turns | Preserved allowance, limits and candidate |
| Context exhaustion | Deterministic compaction or bounded refusal | Never drop authoritative contract |
| Repetition/controller deadlock | Explicit bounded steering and failure | Do not conceal repeated rejected tools |
| Tool timeout/cancel | Outcome recorded or uncertainty surfaced | Never retry uncertain mutations/processes |
| Verification/stale evidence | Show actual grade/status/candidate binding | Explicit review; no invented pass |
| Crash/storage/corruption | Preserve state; identify failed durable boundary | Reject broken chain/unknown schema |
| Missing receipt/source drift | Explain unavailable decision/refusal | No Apply bypass or manual candidate overwrite |
| Symlink/permission/setup | Exact path and ignore/configuration guidance | No automatic broad copying or scope omission |
| Concurrent run/unknown effect | Lease/ownership refusal and inspectability | No guessed owner PID, force takeover or replay |

Stopping messages name only known state. No fixed “30 seconds” promise unless a
real tool deadline exists. Ctrl-C handling must await cooperative engine cleanup;
a second interrupt cannot be advertised as daemon detachment without a separate
execution lifecycle implementation.

## Memory policy

- Contract/permissions: immutable engine authority.
- Checkpoint: exact validated execution, conversation, candidate and accounting.
- Events/receipt: durable facts and evidence.
- Working context: bounded model input, preserving valid call/result structure.
- Task summary: bounded advisory root goal/answer and provenance; never permission.
- Repository observations: separate source-bound advisory memory.
- UI state: local preferences, drafts and restart focus; never proof of completion.

Existing schema-1 task records remain readable. Do not introduce redundant SQL
stores where receipts/checkpoints already supply the facts. Any future schema
must define legacy loading, unknown-field handling, atomic persistence, migration
failure and historical fixtures before it is written by default.

Task selection is explicit id > current session focus > unambiguous single run.
Startup may restore saved focus after validation. Local corruption is explained;
no arbitrary run is selected to conceal it. Forget affects advisory memory/focus,
not the event chain or candidate. Follow-ups preserve scope and limits but use a
new accounting ledger and fresh evidence.

## Milestones and gates

1. **M0 audit/baseline:** resolve actual code behavior and specification conflicts;
   capture existing real-binary flows at 32/40/60/80/100/120/160 columns.
2. **M1 security/continuity:** visible bidi sanitization; bounded private local
   memory; no cross-session focus hijack; cancellation cleanup. Gate: negative
   tests, Unicode/prompt-injection tests and two-session PTY regression.
3. **M2 palette/glyphs:** terminal-native roles, no background assumption, ASCII
   app glyphs, NO_COLOR/dumb behavior. Gate: colored/plain captures, exact decoded
   JSON snapshots, no unsanitized model output.
4. **M3 blocks/review:** replace startup, activity, history, answer, receipt,
   evidence, error and confirmation presentation through pure block components.
   Gate: widths, long paths/code/diffs, explicit Apply/Discard, no stale UI facts.
5. **M4 dock spike:** single input owner during execution, type-ahead, resize and
   process approval arbitration. Gate: no lost paste/input, no duplicate submit,
   no stdin race, renderer failure leaves durable engine state inspectable. Do not
   ship unverified type-ahead or replace reedline before measuring the spike.
6. **M5 recovery report:** typed user guidance plus verified saved/not-saved
   facts. Gate: missing receipt, output limit, invalid JSON, unsafe checkpoint and
   unknown billing fixtures; no unsupported “resumable” label.
7. **M6–M9 engine/memory refinements:** reuse effect fences, leases, validated
   checkpoints and deterministic compaction. Change schemas or policies only for
   an observed remaining failure with a regression and compatibility plan.
8. **M10 release evidence:** strict fmt/Clippy/workspace tests/rustdoc/release,
   historical fixtures, cross-platform CI, hostile-repository Docker gate and
   dependency policy. Existing ignored tests do not become passed tests.

A milestone is complete only after its gate passes. Research/audit completion is
not completion of the UI rewrite, dock or a reliability benchmark.

## Acceptance and rejection

Reject a build if blank/unknown numbers become zero; completion implies verified
correctness; partial streams execute tools; event corruption is silently trimmed;
resume resets limits; another session silently changes the selected task; Apply
or Discard executes on Enter alone; errors hide their original cause; a proposed
feature appears usable without backend support; or renderer failure destroys
recoverable state.

Use deterministic loopback providers for protocol faults and PTY flows; crash at
intent/outcome/checkpoint boundaries; assert zero duplicate completed mutations.
Test local memory corruption, oversized UTF-8, directory/file symlinks, missing
files, restart and two simultaneous sessions. Preserve private benchmark data.

Rendering targets (not measurements): responsive composer under resize/paste;
no lost user input under high-rate synthetic events; coalesced operation updates,
append-only transcript and bounded dock. Record CPU, RAM, terminal, OS, binary
commit, event workload and latency distribution when measuring. Real models and
benchmarks are a separate evaluation, not implied by local fixture passes.

## Implementation status

- M0/M1 foundation shipped on the v2 feature branch: source audit, seven-width
  baseline, bounded local memory, visible bidi controls and session-local focus.
- M2/M3 initial Ledger presentation: shared section/field/note components, faint
  metadata, adaptive activity rows, neutral applied/answered states, state glyph
  fallbacks, workspace/focus composer status, and open-layout process approval.
- M5 initial typed recovery guidance preserves the engine error and distinguishes
  access/configuration, transient provider, budget, protocol, context, storage,
  workspace and unsafe-checkpoint failures. It does not claim saved byte counts
  or checkpoint eligibility that have not been checked.
- M4 uses reedline external printing with a bounded engine-to-terminal channel.
  One owner handles running input and approval. PTY regressions cover multiline
  Unicode paste, caret retention, explicit-only dispatch, cooperative stop,
  retained drafts and fresh approval challenges. Plain mode is cursor-free and
  denies process prompts; it does not pretend to offer an interruptible editor.
- M6–M9 retain the audited durable architecture instead of replacing its schemas.
  Existing production tests cover interrupted/failed model boundaries, effect
  fences, candidate drift, cost reservations, immutable budgets, compaction
  checkpoint replay and historical memory/state readers. No new SQL store or
  conversation authority tier is introduced.
- M10 is the final release gate: exact-commit platform CI, dependency policy,
  Docker containment, terminal workflows, historical readers, release build and
  deterministic soak. Publication remains blocked until all required gates pass.
