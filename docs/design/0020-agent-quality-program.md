# Design 0020: evidence-led agent quality program

Status: audit and baseline qualification in progress. No behavior experiment has
been promoted. Research branch: `research/v2.2-agent-quality-lab`.

## Baseline identity and constraints

The starting main commit is `64da001c565a7cf8d662a872c5f9d6c564c289b9`
(Pactrail 2.1.2). This program does not change release tags or modify main directly.
Raw qualification evidence lives in the ignored
`benchmark-results/agent-quality-program-20261005/` directory. A successful
baseline is not qualification of subsequent changes.

The model proposes; Pactrail owns authority. Candidate isolation, policy, safe
paths, effect fences, evidence grading, recovery, receipts, and explicit apply
remain non-pluggable. No benchmark improvement permits weakening these constraints.
Hosted text is not latent state. Real latent inference is outside this program.

## Existing outcome evidence

The immutable agent-runtime-v1 Space Bunny protocol v2 contains 18 declared
trials on three public historical issues: single agent passed 6/9, fixed text
specialists passed 1/9. Eight specialist trials exhausted role budgets. Lower
resource usage from these early failures is not improved efficiency. These are
diagnostics, not independent held-out generalization evidence.

The completion-audit pilot contains 48 trials on six synthetic Python tasks.
Both baseline and audited Pactrail passed 12/12 functional trials; audit used
70.5% more reported tokens and left a temporary file in one candidate. Its
historical cleanliness detector missed new empty files; preserve recorded
results and use the corrected runner only with a new protocol.

Sources: [agent qualification](0019-v2.1-release-qualification.md),
[completion pilot](../../benchmarks/results/2026-10-01-space-bunny-completion-audit/README.md).

## Capability and experiment matrix

| Capability | Current implementation | Limitation and evidence | Proposed experiment | Initial decision |
| --- | --- | --- | --- | --- |
| Harness Lab | Historical issue replay; six-case completion pilot; three-case agent diagnostic | Too few distinct issues; synthetic tasks; incomparable strict assurance checks | 20–50 pinned real issues, separate development/confirmation, externally executed gold checks | Build first |
| Editing | Exact replace, atomic ordered edit_file, strict apply_patch with optional digest, bounded post-edit windows | Exact edit requires reproduced old text; frequency of mismatches not yet measured | Exact vs revision-bound anchor ranges, same task/model/budget | Keep current; prototype only after baseline |
| Delegation | Independent conversations and durable fixed serial localizer/solver/critic/implementer | 8/9 specialist budget failures in frozen diagnostic | Primary + bounded read-only scout/reviewer vs fixed profile | Keep existing experimental; evaluate adaptive |
| Handoff | Content-addressed bounded text messages and delivery state | Prose overhead; information-loss rate not measured | Free text vs typed observation references | Experimental only |
| Budgets | Global usage/attempt/cost/time checks, durable reservations, role turn caps | Fixed role caps terminate despite unused useful global capacity | Deterministic delegate leases with primary reserve | Requires crash/accounting tests |
| Retrieval | Exact-byte cached index, Tree-sitter, lexical retrieval, bounded graph/impact, memory and Git tools | No broad gold-relevance recall baseline | Lexical, graph, hybrid, permitted Git with equal bytes | Reuse compiler; no mandatory vectors |
| LSP | Strict integrity-bound reference snapshot; no server launch | Live navigation unavailable | Approved pinned read-only server vs off | Optional; after retrieval baseline |
| Extensions | ModelDriver, tool registry, governed MCP, SDK | Lifecycle across optional strategies not unified | Narrow static registration and failure cleanup | Defer until concrete adapter need |
| Skills/workflows | Scoped AGENTS instructions; no qualified package lifecycle | Benefits unmeasured; malicious instructions cannot grant authority | Relevant advisory skill vs none | After bounded extension surface |
| Shared runtime | Engine reused; CLI orchestration and browser subprocess jobs remain separate | Design 0016 proposed, not implemented | Workspace lease, durable command admission, shared snapshots | Later dependency for ACP/steering |
| ACP/steering | No shared durable client command lifecycle | Idempotency/ownership not established | Duplicate commands, competing clients, safe-boundary steering | Defer until runtime |
| DAP | Governed process backend; no debugger adapter | Navigation benefit unknown | Observational adapter on deterministic fixtures | Later experimental |
| Tool surface | Stable full typed catalog; declared policy and dispatch validation | Schema overhead and wrong selection need measurements | Full vs deterministic lazy discovery | Preserve current default |
| Completion | Controller, acceptance checks, isolated verification, revision-bound optional audit | Pilot gave no benefit; strict vs functional failures need classification | Requested-behavior and regression evidence independently reported | Audit stays opt-in |
| Obligation tracking | Contract obligations and recorded evidence; audit snapshot | Suggested status alone is not proof | Evidence-derived unresolved ledger/context selection | Reuse domain types |
| CLI | Reedline composition, width-aware blocks, progress, recovery advice, PTY fixtures | Human task-flow baseline incomplete | Readability, narrow/wide terminals, recovery/review interaction checks | Include UX; no authority changes |
| Latent | Experimental transport/descriptor/artifact/intervention tests | No bundled real neural hidden-state backend | Preserve mechanics tests; future design only | Defer research backend |

This is an initial matrix grounded in the named existing implementations and
retained results. Repository-wide audit and external source review are ongoing;
unmeasured limitations above are hypotheses, not established defects.

## Program order and promotion gates

1. Finish current-main audit, strict Rust/compatibility/security/terminal gates,
   repository-scale and process/startup measurements. Preserve baseline binary
   identity and raw output; never overwrite prior frozen benchmark binaries.
2. Review pinned primary harness sources and papers; record license/provenance.
   Conceptual reimplementation is preferred. No external code reuse by default.
3. Expand Harness Lab. Every eligible case must prove base targeted failure,
   gold targeted pass and gold regression pass. Isolate each grading phase to
   prevent overlays/build cache contaminating subsequent judgments.
4. Freeze task population, split, graders, source/binary hashes, model metadata,
   permission/budget policy, ordering, repetitions, stopping and analysis before
   the first scored request. Record all declared outcomes including unavailable.
5. Establish baseline, then Experiment 01 (anchors). No agent behavior change
   before the preceding gates. Sequential ablations follow the dependency order
   in the matrix; avoid Cartesian-product experiments and benchmark-specific fixes.
6. After each experiment: PROMOTE, KEEP EXPERIMENTAL, DEFER, or REMOVE. Promotion
   requires material measured benefit and zero authority/compatibility regression.
   Confirmation cases must not be used for iterative tuning. Task authorship and
   curator independence must be disclosed; partitioning alone does not make a
   task set independent.

## Measurement and analysis contract

Strict completion, targeted correctness, regressions and candidate production
are distinct fields. Pactrail-specific assurance checks are reported separately
when comparing other harnesses. Classify model, harness, policy, verification
configuration, tool interface, budget and provider failures.

Track coverage alongside tokens/cache/cost/latency/tool calls/context/edit
failures/handoffs/patch scope. Missing is null, never inferred zero. Free API
pricing does not prove monetary savings. Compare paired task-level outcomes and
task-clustered uncertainty; repetitions do not create independent tasks.
Retrieval relevance inferred from reference patches is an incomplete label,
requiring manual validation where feasible; it is not exhaustive causal context.

Baseline public OpenRouter metadata on October 5 exposes
`stealth/space-bunny-alpha` and `apodex/apodex-1.1-mini:free`. Metadata availability
is not proof of inference availability. Capture the endpoint/model capabilities
per experiment, do not substitute models, and keep real credentials out of agent
and grader environments and all retained artifacts.

## Security and reproducibility gates

Every new experiment retains the same Tool Kernel and isolated transaction.
Untrusted source/agent/LSP/DAP/skill/plugin output is advisory, not evidence or
authority. External graders are not in agent trees; gold patches and future
Git history/remotes are absent. Global reservations precede model I/O and cannot
be refunded by crashes. Mutations stay serialized and uncertain completion
fails closed. Historical hashes/codecs and ordinary single-agent defaults remain
readable and unchanged.

Each logical milestone requires formatting, strict Clippy, complete workspace
tests, docs, release build, compatibility/recovery/authority checks, infrastructure
tests and relevant finite fuzz/hostile-input campaigns. Record local unsupported
containment explicitly rather than converting a skip into a pass; platform CI
is required before qualification. Record binary size, startup/RSS, cold/warm
index/context, transaction and event/tool overhead before/after, with declared
noise-aware thresholds and explanations of intentional increases.

CLI work will improve observable progress, wrapped prose/code, results/errors,
next actions and task composition. It must retain normal scrollback, NO_COLOR,
plain/ASCII modes, narrow widths, cancellation, explicit /diff and /apply, and
no display of private reasoning. PTY checks supplement human-flow review.

## Deliverables and present status

External source ledger: `docs/research/harness-review-2026.md` (pending).
Final research report: `docs/research/agent-quality-final-report.md` (pending).
Harness Lab expansion and scored baseline: pending. No improvement or program
completion is claimed at this stage. Release/publication is outside this branch.
