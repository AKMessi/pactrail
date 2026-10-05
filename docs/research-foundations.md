# Research foundations

Pactrail adopts mechanisms from primary research only when they can be made
deterministic, bounded, inspectable, and compatible with its transaction trust
boundary. A paper result is evidence for testing a mechanism, not permission to
copy its benchmark claim into Pactrail.

## Agent-quality program: falsifiable mechanisms

The [October 2026 program](design/0020-agent-quality-program.md) begins with an
expanded lab and retained current-main baseline. The following are experimental
hypotheses, not shipped improvements. No paper's pass rate is a Pactrail result.

| Primary research | Problem/mechanism | Already present | Cheapest falsification and promotion criterion |
| --- | --- | --- | --- |
| [SWE-agent](https://arxiv.org/abs/2405.15793) | Interface shape and concise action feedback influence repository navigation and editing | Typed bounded tools and post-edit source windows | Exact vs anchored editing under identical ceilings; reject if stale edits can pass or correctness declines |
| [Agentless](https://arxiv.org/abs/2407.01489) | Simpler localization/repair/validation and issue-quality audit can challenge complex agents | Structural navigation and independent verification | Primary vs fixed pipeline vs bounded scout, same total budget; retain delegates only for measured outcome value |
| [AutoCodeRover](https://arxiv.org/abs/2404.05427) | Structural search and test-guided localization narrow relevant context | Parsed definitions and lexical reference/impact tools | Retrieval-only file/symbol recall at equal bytes, then task outcome ablation |
| [RepoCoder](https://arxiv.org/abs/2303.12570) | Iterative retrieval updates context after task information changes | Current-candidate graph queries | Fixed initial retrieval vs bounded updated selection; require reduced missed context or inference waste without stale source |
| [RepoGraph](https://arxiv.org/abs/2410.14684) | Repository relationships augment navigation | Bounded definition/reference graph | Lexical vs graph vs hybrid with frozen gold labels; do not treat lexical edges as runtime calls |
| [Context as a Tool](https://arxiv.org/abs/2512.22087) | Explicit context maintenance and trained compression address long-horizon drift | Deterministic digest-bound compaction and recoverable observations | Measure repeated reads, omitted facts and context bytes before adding a new controller; trained-paper compressor gains do not transfer automatically |
| [What Context Does a Coding Agent Actually Need to Act?](https://arxiv.org/abs/2607.09691) | Separates localization from edit-site representation; reports null results for some surrounding context | Source re-read requirement, advisory navigation evidence | Hold localization fixed for context ablation; repeated trials and exact current source, never prose-as-source |
| [Agent Retrieval Bench](https://arxiv.org/abs/2607.24882) | Next-step workflow relevance differs from query similarity; selective retrieval has calibration gaps | No broad retrieval-only task label suite | Recall@K, budgeted yield and no-relevant-context cases; accept a broker only if recall/noise/latency tradeoff improves |
| [SWE-Gym](https://arxiv.org/abs/2412.21139) | Executable repository environments support agent and verifier training/evaluation | Isolated external graders and candidate checks | Validate base failure and reference passes locally; verifier selection later requires independent candidate budgets and regression checks |
| [AsynCodeBench](https://arxiv.org/abs/2609.32662) | Final task success can conceal failed dependency coordination | Durable addressed messages; no useful-handoff metric | Count actual handoff consumption and grounded downstream findings; do not infer useful collaboration from message counts |
| [OpenCollab](https://arxiv.org/abs/2609.38345) | Declared topology may differ from realized execution; shared controls and fine-grained events improve attribution | Deterministic lifecycle/sequence/accounting | Verify actual delegate routing against protocol and measure outcome under total resource constraints; no adoption of paper superiority claims |

This initial mechanism review uses primary paper descriptions and existing
Pactrail code/evidence. Full experimental methods and dataset caveats require
deeper review before implementing each proposed mechanism. In particular,
learned compressors/verifiers and asynchronous collaboration are not being
implemented merely because their papers report gains. The next scored baseline
must use validated historical issues, not the old six synthetic cases.

## Repository navigation

- [SWE-agent: Agent-Computer Interfaces Enable Automated Software Engineering](https://arxiv.org/abs/2405.15793)
  shows that simple, compact actions, concise environment feedback, and
  guardrails can materially change agent performance without changing model
  weights. Pactrail therefore keeps graph navigation as one typed read-only
  action rather than exposing a collection of parser-specific commands, and
  mutation tools return bounded current-source feedback instead of only an
  acknowledgement.
- [RepoCoder: Repository-Level Code Completion Through Iterative Retrieval and Generation](https://arxiv.org/abs/2303.12570)
  demonstrates the value of using new model/task information to retrieve again
  instead of treating initial retrieval as final. Pactrail exposes graph search
  during the run and rebuilds it from the current candidate on every query.
- [AutoCodeRover: Autonomous Program Improvement](https://arxiv.org/abs/2404.05427)
  uses program-structure search APIs and stratified retrieval to move from issue
  terms to classes and methods. Pactrail adopts the model-facing structural
  navigation pattern while retaining a language-portable deterministic
  fallback and explicit evidence labels.
- [RepoGraph: Enhancing AI Software Engineering with Repository-level Code Graph](https://arxiv.org/abs/2410.14684)
  reports gains from definition/reference ego-graphs across both procedural and
  agent systems. Pactrail's first graph is deliberately narrower: declarations
  and bounded lexical references, with no claim of type resolution or runtime
  call-flow accuracy.
- [Agentless: Demystifying LLM-based Software Engineering Agents](https://arxiv.org/abs/2407.01489)
  provides evidence for hierarchical localization and patch validation instead
  of assuming longer autonomous loops are always better. Pactrail uses graph
  evidence to improve localization and preserves deterministic verification as
  a separate authority.
- [Tree-sitter: Using Parsers](https://tree-sitter.github.io/tree-sitter/using-parsers/)
  documents the official incremental concrete-syntax-tree runtime and its Rust
  binding. Pactrail embeds a bounded subset of official grammars for structural
  declarations while preserving a feature-disabled lexical fallback.
- [Language Server Protocol](https://microsoft.github.io/language-server-protocol/)
  standardizes editor/server features including definitions and references.
  Pactrail accepts only an explicit normalized reference snapshot in v1; it
  does not infer that protocol support grants permission to start a server.

## Context and validation

- [A Case Study of LLM for Automated Vulnerability Repair: Assessing Impact of Reasoning and Patch Validation Feedback](https://arxiv.org/abs/2405.15690)
  supports feeding external compiler, test, and sanitizer evidence back into
  repair. Pactrail exposes capability-gated process results to the model and
  now permits one bounded repair cycle after deterministic validation rejects a
  candidate, followed by an independent final run in a fresh snapshot.
- [Context as a Tool: Context Management for Long-Horizon SWE-Agents](https://arxiv.org/abs/2512.22087)
  motivates separating stable task semantics, condensed long-term trajectory
  state, and high-fidelity recent interactions. Pactrail uses this separation
  for deterministic, provenance-preserving context compaction rather than
  append-only history or model-authored summaries.
- [What Context Does a Coding Agent Actually Need to Act?](https://arxiv.org/abs/2607.09691)
  reports that source at the edit site carries more useful behavioral signal
  than natural-language summaries and that carefully compressed context can
  match whole-file context at lower token cost. Pactrail therefore treats the
  evidence graph as navigation only and instructs the model to read current
  source before editing.

## Shipped evidence graph invariants

1. The default graph is derived without a model, network, compiler, or language
   server. Parser-backed structure is in-process and bounded.
2. Definition and reference locations are workspace-relative and deterministic.
3. References are labelled lexical, language-server, or corroborated; none can
   become verification evidence.
4. Construction has global and per-symbol limits with visible truncation.
5. Current source is read and hashed once per build; graph structure comes from
   that exact retained analysis rather than a second filesystem pass.
6. Tool queries rebuild from the isolated candidate, so preceding edits are
   visible and the source workspace remains untouched.
7. The model must read cited source before editing; graph results never replace
   current code.
8. Optional LSP data is canonical, bounded, integrity-checked, exact-repository
   bound, and validated completely before graph mutation. Pactrail does not
   start a language server during indexing.

## Shipped trajectory compaction invariants

1. Stable system, repository, and task messages are never summarized or
   removed.
2. Assistant tool calls and tool-result order remain intact for provider
   protocol validity.
3. Recent tool evidence stays exact unless its size alone threatens the model
   window.
4. A compacted result retains its call ID, tool name, error status, original
   byte count and BLAKE3 digest, bounded anchors, and a small exact JSON preview.
5. The model receives explicit re-read guidance; a compacted envelope is
   navigation evidence, not a replacement for source at the edit site.
6. Compaction thresholds come from declared context and output limits and every
   event records before/after request digests and byte counts.
7. No model-generated summary can become durable trajectory state.

## Shipped mutation-feedback invariants

1. Feedback is generated from the isolated candidate after the write succeeds,
   never from a model claim about the intended edit.
2. Final bytes and BLAKE3 digest identify the exact current file version.
3. Changed-line bounds come from a deterministic UTF-8-safe comparison of the
   prior and current source for exact edits.
4. Current source is line-numbered and bounded by both line and byte ceilings;
   distant change regions receive previews at both edges.
5. The result says whether all changed lines are visible and provides an exact
   narrow-read recovery path when they are not.
6. Exact no-op edits are rejected because they produce no new candidate state
   or evidence.

## Shipped validation-repair invariants

1. Repair is available only for a changed candidate, an authorized discovered
   check, a real non-zero process exit, and a remaining model-turn budget.
2. At most one automatic repair cycle occurs per run; final verification never
   recursively requests another repair.
3. Diagnostics are bounded from declared model context/output limits and carry
   a digest and original byte count.
4. Process output is delimited and labelled as untrusted repository data;
   infrastructure, policy, spawn, and timeout failures are not treated as
   repairable source failures.
5. The probe runs in a disposable candidate snapshot. The repaired candidate is
   verified again in a fresh snapshot, and only that final result becomes
   receipt evidence.
6. Probe/final phases, candidate digest, diagnostics digest, and controller
   decision are hash-linked and visible in the trace.

## Deliberately deferred

- A built-in LSP process adapter remains deferred until executable identity,
  initialization, synchronization, timeout, cancellation, and adversarial
  protocol fixtures can share the same governed runtime boundary.
- Learned embeddings are not a mandatory dependency; local and air-gapped use
  must retain deterministic retrieval.
- Multiple candidate sampling and patch ranking require isolated child budgets
  and receipts; they will not share mutable tool state.

## Methods review for the first quality experiments

The versioned primary-paper methods below inform experiments; they do not qualify
Pactrail changes. Captured source bytes and SHA-256 manifests are retained under
`benchmark-results/agent-quality-program-20261005/paper-methods-v1/`.

- [SWE-agent v1, ACI design and experimental setup](https://arxiv.org/html/2405.15793v1)
  studies tool documentation, feedback, edit guardrails and observation management.
  Its interface ablations use a smaller Python issue subset than its main results.
  For Pactrail, change the edit interface while keeping mutation feedback, context
  policy, model and total ceilings fixed. Otherwise improvements cannot be
  attributed to anchors. Measure rejected edits and redundant post-edit reads;
  safety rejection alone is not an edit failure. Do not add implicit compiler
  execution when process capability is denied.
- [Agentless v2, localization/repair/validation ablations](https://arxiv.org/html/2407.01489v2)
  varies components separately and measures retained gold locations alongside
  context size. Patch sampling and validation are additional work, not a free
  comparison with a single candidate. Pactrail's retrieval lab should record
  whether selected context contains reference-touched files/symbols, while noting
  that alternate valid fixes can touch different locations. Candidate sampling
  needs independent budgets and external grading. A delegate that localizes well
  can still hurt final repair; localization and final success must remain separate.
- [RepoGraph v1, construction and integration](https://arxiv.org/html/2410.14684v1)
  constructs repository relationships from parsed code and exposes graph retrieval
  to existing workflows. Pactrail already has parsed definitions and bounded
  lexical relationships. The useful experiment is selection under equal bytes,
  not another graph implementation. Compare lexical-only, graph-only and hybrid
  retrieval with the same labels and candidate revision. Name and lexical matches
  cannot be promoted to type-resolved call edges. Measure warm-index reuse and
  truncation as well as recall, since extra navigation can spend more inference.

Engineering deductions above are hypotheses to falsify in the Harness Lab. They
are not a reproduction of paper results or evidence of benefits for either
configured OpenRouter model. Remaining mechanisms require their own method-level
review before implementation.
