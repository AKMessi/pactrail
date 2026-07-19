# Research foundations

Pactrail adopts mechanisms from primary research only when they can be made
deterministic, bounded, inspectable, and compatible with its transaction trust
boundary. A paper result is evidence for testing a mechanism, not permission to
copy its benchmark claim into Pactrail.

## Repository navigation

- [SWE-agent: Agent-Computer Interfaces Enable Automated Software Engineering](https://arxiv.org/abs/2405.15793)
  shows that simple, compact actions, concise environment feedback, and
  guardrails can materially change agent performance without changing model
  weights. Pactrail therefore keeps graph navigation as one typed read-only
  action rather than exposing a collection of parser-specific commands.
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

## Context and validation

- [A Case Study of LLM for Automated Vulnerability Repair: Assessing Impact of Reasoning and Patch Validation Feedback](https://arxiv.org/abs/2405.15690)
  supports feeding external compiler, test, and sanitizer evidence back into
  repair. Pactrail already exposes capability-gated process results to the model
  and independently reruns discovered verification in a disposable snapshot.
- [Context as a Tool: Context Management for Long-Horizon SWE-Agents](https://arxiv.org/abs/2512.22087)
  motivates separating stable task semantics, condensed long-term trajectory
  state, and high-fidelity recent interactions. Pactrail will use this as the
  basis for deterministic, provenance-preserving context compaction rather than
  append-only history.
- [What Context Does a Coding Agent Actually Need to Act?](https://arxiv.org/abs/2607.09691)
  reports that source at the edit site carries more useful behavioral signal
  than natural-language summaries and that carefully compressed context can
  match whole-file context at lower token cost. Pactrail therefore treats the
  evidence graph as navigation only and instructs the model to read current
  source before editing.

## Shipped evidence graph invariants

1. The graph is derived without a model, network, compiler, or language server.
2. Definition and reference locations are workspace-relative and deterministic.
3. References are labelled lexical; they cannot become verification evidence.
4. Construction has global and per-symbol limits with visible truncation.
5. A file changing between graph passes aborts indexing instead of mixing
   incompatible repository states.
6. Tool queries rebuild from the isolated candidate, so preceding edits are
   visible and the source workspace remains untouched.
7. The model must read cited source before editing; graph results never replace
   current code.

## Deliberately deferred

- Tree-sitter and optional LSP/type-resolution enrichment require language and
  parser compatibility fixtures before they can strengthen lexical edges.
- Learned embeddings are not a mandatory dependency; local and air-gapped use
  must retain deterministic retrieval.
- Automatic history compaction must preserve tool-call protocol validity,
  provenance, reread paths, and trace observability before shipping.
- Multiple candidate sampling and patch ranking require isolated child budgets
  and receipts; they will not share mutable tool state.
