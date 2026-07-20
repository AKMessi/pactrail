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
  Pactrail accepts only an explicit normalized reference snapshot in 0.8; it
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
