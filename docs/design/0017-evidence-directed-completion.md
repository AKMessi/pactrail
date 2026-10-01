# Design 0017: evidence-directed completion

Status: implementation and evaluation in progress. October 1, 2026.

## Decision

Prioritize a bounded, revision-bound completion audit over a daemon rewrite for
improving task outcomes. Design 0016 remains the coordination roadmap; a daemon
alone cannot determine whether code satisfies a task. This upgrade extends V2's
existing controller and deterministic verification rather than replacing them.

The concrete evidence is in the September 26 Space Bunny runs: three cases
produced no edits; a later calibration fixed one path but missed another path
in the same public API. Neither faster execution nor a passing generic test
suite establishes a complete fix.

## Paper review

These are scoped research findings, not promises that the same gains transfer
to Pactrail or Space Bunny. Links are to primary papers.

| Paper | Finding relevant here | Design consequence |
| --- | --- | --- |
| [SWE-agent](https://arxiv.org/html/2405.15793v3) | Agent-facing actions, concise feedback, and guardrails affect behavior. Repeated navigation/edit actions are observed failure modes. | Preserve typed tools, exact mutation feedback and bounded context; do not introduce a second privileged shell path. |
| [Agentless](https://arxiv.org/html/2407.01489v1) | Hierarchical localization and controlled repair/filtering can compete with complicated autonomous pipelines. Its issue audit finds misleading or incomplete descriptions. | Prefer a deterministic control plane and inspect related locations. Audit benchmark tasks and graders instead of assuming their labels are correct. |
| [SWE-Search](https://arxiv.org/html/2410.20285v1) | Search and iterative refinement explore alternative trajectories with explicit selection mechanisms. | Consider candidate search later, after selection signals and total budgets are reliable. Unbounded branching is not the first efficient upgrade. |
| [SWE-Review](https://arxiv.org/html/2607.06065v1) | Repository-grounded review with actionable diagnosis can improve revision; review quality and reviewer strength matter. | Add an explicit opportunity to inspect and repair before accepting a final summary. Review remains fallible; require independent execution for correctness claims. |
| [Intrinsic self-correction study](https://arxiv.org/html/2310.01798v2) | The studied reasoning models often fail to improve without external feedback. The result is not a theorem about all coding models. | Ask for falsifying source/tool evidence, not generic “think again.” Never promote model approval to deterministic evidence. |
| [SWE-Bench Pro](https://arxiv.org/html/2509.16941v1) | Longer tasks require coordinated repository changes and multi-step evaluation. | Include sibling paths, boundary cases, and regressions; small local tests cannot establish broad superiority. |
| [Developer productivity RCT](https://arxiv.org/abs/2507.09089) | Early-2025 tools slowed the experienced developers studied despite optimistic expectations. Its population and tools are specific. | Measure accepted outcomes and human review effort, not token speed alone. |

## Implemented mechanism

The opt-in `--completion-audit` policy adds at most two audit requests over a
change run. Each consumes the ordinary model-turn, token, cost and time budget.
The engine binds requests to the exact candidate digest, including every changed
file. A later revision can request another audit; the global allowance cannot
reset when a candidate changes or the process resumes.

A bounded controller snapshot lists obligations, caller-declared check counts,
changed paths and digests, with explicit total counts when lists are truncated.
The model must inspect current source/related implementations, identify checks
that could falsify the fix, repair concrete defects, and separate observed
checks from uncovered obligations and blockers. Tools and permissions stay
unchanged. No process permission is silently granted.

Audit admission is recorded as a normal hash-linked controller action. The
ledger is reconstructed from the verified event store, independent of context
compaction. The request and assistant proposal are checkpointed before the next
model turn. Enabled policy participates in checkpoint profile identity; disabled
policy preserves historical profile encoding. Historical run manifests default
the new option to false.

This is an opportunity for evidence-guided review, not proof that review occurred
or was correct. A review request does not add passing evidence. Final verification
still uses the existing isolated, candidate-bound process and acceptance-check
mechanism. A final revision without an audit allowance produces a persistent
receipt risk. Read-only tasks skip this policy.

## Rollout and limits

Keep the policy opt-in until the matched ablation justifies its cost. The same
model may repeat its earlier mistake; this first implementation does not claim
an independent reviewer. The next stage should separate reviewer context,
provide independently supplied acceptance plans, and select diagnostic actions
using observed failures. Those stages require separate compatibility and
budget/recovery gates, not unconditional extra model calls.

## Required checks

- Revision change invalidates the prior request; at most two requests globally.
- Zero remaining turns never adds a request or exceeds the turn cap.
- Verified event replay restores the allowance after context compaction/resume.
- A second-path repair can occur before finalization without writing source.
- Review cannot fabricate a passing evidence record or widen permissions.
- Enabled/disabled policy cannot be exchanged during checkpoint resume.
- Freeze tasks, gold grader validation, versions, model parameters, order and
  failure retention before scored calls; compare both pre-upgrade and audited
  Pactrail with external harnesses. Report exploratory small-suite limits.
