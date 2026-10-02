# V2 Ledger: local repository audit

Audited 2026-10-02, based on main `1dc279a` and the user's 19-page Ledger
specification. The specification explicitly lacked source access; this document
supersedes its assumptions about implementation. This is an implementation audit,
not an independent security review or a model benchmark.

## Source of truth

| Area | Actual implementation | Consequence for v2 |
| --- | --- | --- |
| License | `LICENSE`, workspace Cargo metadata: PolyForm Noncommercial 1.0.0; bundled notices preserved | Resolved. No third-party code or assets are copied for this redesign. |
| Composer | `interactive.rs`: reedline, bracketed paste, multiline editing, private drafts, external editor, command/run completion | Retain editing and keyboard compatibility while replacing presentation. No need for a new full-screen framework. |
| Output | `output.rs` sanitizes C0/C1; `Theme` sanitizes before adding trusted styles; JSON escaping preserves decoded values | Add visible bidi-control handling. Raw terminal control writes must remain app-generated; JSON and paths must not be rewritten semantically. |
| Live activity | `RunActivity` receives typed `RunProgress`; indicatif owns only the current-operation line | Events remain append-only. Running-phase type-ahead needs a separate input-owner spike; never race stdin against process approval. |
| Paging | `terminal.rs` invokes an explicitly configured pager as argv, not shell code | Keep complete diff/code in the pager. Do not invent a built-in interactive reviewer. |
| Decisions | Interactive Apply/Discard require exact typed acknowledgments; blank or incorrect input cancels | Preserve this boundary. Applied is a neutral outcome, not proof of passing verification. |
| Task focus | `Session.last_run` plus a workspace-keyed `focus` file in local config | Previously, `continue` reread the shared file every time. Another session could redirect it. Restore once; keep live-session selection local. |
| Task memory | `continuation.rs`: bounded schema-1 JSON in `ComposerStore`; run/contract binding, provisional goal binding, root lineage and bounded answer | Advisory context only. Validate before use; original contracts/checkpoints stay authoritative. No new competing task database is required. |
| Local persistence | `composer.rs`: private temp file, file fsync, atomic replacement, 64 KiB cap | Add directory fsync on Unix, bounded key validation, real-directory checks on read/delete, and no-follow/nonblocking reads on Unix. This is not encryption or protection against every same-user filesystem race. |
| Continue | Executing/Failed -> guarded resume; AwaitingApply -> review; Answered/Applied -> bounded assessment follow-up; Cancelled/Discarded -> refusal | Keep the user's existing plain-continue behavior. Do not silently resurrect discarded candidates or stop-state work. |
| Resume | `checkpoint.rs::load_head`, `engine.rs::validate_resume_checkpoint`: event-bound artifact, candidate/contract/profile hashes, pending effects, remaining budgets | Failure labels alone never authorize resume. Do not scan backwards across effects or reset budgets. |
| Failed provider recovery | Only adjacent Executing -> Failed after an intact BeforeModel checkpoint; cost-capped failures refuse unknown billing | Preserved. A readable output-limit failure after a model action is not automatically eligible under this rule. |
| Model transport | Four adapter families; bounded buffered/streamed parsing; retry policy in each adapter; OpenAI-compatible EOF retries share a three-retry allowance | Do not replace this with arbitrary five-attempt defaults. Usage from an unreadable attempt is unknown. Streamed partial calls are not admitted. |
| Output exhaustion | `engine.rs`: reported usage first; at most two accounted recovery turns; adaptive allowance ceiling; allowance restored from trace | Existing implementation matches the core requirement. A truncated answer never becomes success. |
| Effects | Core effect events, event snapshots, transaction journals, checkpoint call IDs and pending-effect refusal | Already present. Do not create a parallel effects SQL table or promise arbitrary exactly-once process execution. |
| Concurrency | SQLite immediate-transaction run lease CAS; CLI workspace execution lock; TTL tied to execution limits | Existing protection, not the spec's proposed 15-second heartbeat protocol. No owner-PID/live-session count in the UI unless actually recorded. |
| Receipt missing | `commands.rs::apply_run` already explains the missing completed receipt and guarded recovery | No generic file-not-found Apply message for this path. Other receipt-dependent views need truthful fallbacks. |
| Dependency symlinks | `manifest.rs` uses ignore traversal; excludes `.git` and `.pactrail`; remaining symlinks are conservatively rejected | Generated dependency trees must be ignored explicitly. Blanket omission by basename could silently omit intended source and is not adopted. |
| Verification | Digest-bound candidate checks and obligation-linked evidence; process permission required | Disabled commands mean build/test evidence may be inconclusive. A completed edit is not a passing build. |
| Compaction | `context_window.rs`: deterministic observation dedupe/artifact references; high water 80%, target 65%; schema preserved | Do not substitute untested 60/75/90 percentages or a fixed three-turn window. Preserve call/result pairing and checkpoint conversations. |
| Repository memory | `pactrail-memory`: distinct durable repository observations/provenance; context compiler supplies advisory fragments | Keep separate from task summaries and UI preferences. No made-up confidence score is shown. |
| Historical compatibility | Versioned checkpoint/request/receipt/state fixtures; future schemas rejected | Never auto-truncate a corrupt event chain or pretend an unsupported schema is safe to migrate. |

## Required corrections to the supplied specification

1. Unknown billing stays unknown even when the provider returns HTTP 429; the
   harness cannot assert a zero provider charge without evidence.
2. No tool call is executed from an incomplete stream merely because its current
   argument prefix parses. Acceptance requires the transport's completed protocol.
3. Corrupt event history is preserved and rejected, not trimmed and rewritten.
4. A budget conflict refuses execution; a UI confirmation cannot amend an
   immutable run contract or grant more authority.
5. Source-baseline freshness is not asserted in the confirmation dialog unless
   a real preflight has checked it. Apply still performs the authoritative check.
6. A detached renderer is not an independent daemon. The current in-process
   engine cannot be promised to survive process exit or SIGKILL.
7. Complete tool output bodies are not all present in the trace. Checkpoints and
   content-addressed artifacts remain necessary; rebuilding every conversation
   from summary events would lose information.
8. Existing NO_COLOR behavior remains strictly unstyled. Optional faint metadata
   applies only when terminal styling is enabled.
9. The PDF ends at the beginning of section 12. Acceptance gates are completed
   in `v2-cli-plan.md`, rather than assumed to exist in the supplied document.

## Research checked against primary sources

- [OpenCode TUI](https://opencode.ai/docs/tui/): command discovery, details,
  external editor and compaction controls inform progressive disclosure.
- [Pi compaction](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/compaction.md):
  supports explicit context-management boundaries; not a benchmark of Pactrail.
- [Oh My Pi](https://github.com/can1357/oh-my-pi): terminal workflow reference;
  no source or artwork is copied.
- [The Complexity Trap](https://arxiv.org/abs/2508.21433): observation masking was
  competitive with model summarization in its evaluated setting. This motivates
  retaining deterministic compaction first; it does not establish a universal
  result across Pactrail models/tasks.
- [CoALA](https://arxiv.org/abs/2309.02427): modular memory distinctions support
  separating authority, history, working context and advisory memory.
- [Lost in the Middle](https://arxiv.org/abs/2307.03172): context position can
  affect retrieval performance; larger prompts alone are not evidence of quality.
- [Temporal activities](https://docs.temporal.io/activities): idempotency and
  durable boundaries inform recovery. Temporal's retry semantics are not assumed
  to transfer to arbitrary host commands or billable model requests.

Research is document/source inspection, not hands-on comparison or a claim that
Pactrail outperforms these tools. Performance and reliability targets require
measured fixture and real-task results.

## Foundation verification

Existing release startup was captured at all seven specified widths under
`/tmp/pactrail-v2-baseline` without model requests. The amended foundation passed
strict workspace Clippy, CLI unit tests, a debug build and the real-engine PTY
suite. The suite now includes two simultaneous sessions and all seven widths.
Cross-platform and Docker release gates remain CI work; these local checks do
not imply that the remaining Ledger milestones are implemented.
