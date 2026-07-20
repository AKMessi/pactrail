# Roadmap

Pactrail's roadmap is organized by trust boundary, not feature count. A feature
is complete only when its policy, trace, recovery, tests, compatibility story,
and user-facing failure mode are complete.

This file distinguishes shipped behavior from planned work. It is not a release
promise or a security claim.

## Shipped through 0.6

- Contract-first runs with explicit capability policy and overgrant detection.
- Isolated workspace transactions, receipt-bound review/apply, source-drift
  detection, synchronized crash journal, rollback, and idempotent recovery.
- Provider-neutral model IR and bounded OpenAI-compatible transport.
- Typed Tool Kernel with schema validation, behavior/risk annotations, bounded
  results, deterministic parallel-safe reads, atomic edits, and candidate-aware
  inspection.
- Model-budgeted context, scoped repository instructions, provenance memory,
  deterministic evidence-graph retrieval, and long-window compaction.
- Hash-linked detailed events, portable trace JSONL, deterministic verification,
  bounded validation repair, and integrity-checked answered/change receipts.
- Interactive and scriptable CLI, model/provider configuration, run history,
  review/apply/discard, trace inspection, doctor, completion, and JSON output.
- Three explicit process modes: disabled, trusted native, and restricted OCI.
- Locally pinned Docker/Podman execution with candidate-only bind, read-only
  root, private bounded temporary storage, no network/capabilities/ambient
  environment, numeric Unix identity, and CPU/memory/PID/time/output ceilings.
- Exact request-bound approvals, distinct policy/decision trace events,
  fail-closed non-interactive behavior, and end-to-end cancellation/cleanup.
- Hostile-repository Docker CI, cross-platform gates, dependency policy, and an
  attested GitHub release workflow.
- Head-bound provider-neutral checkpoints, secret-free runtime manifests,
  exclusive local run ownership, `pactrail resume`/`/resume`, and real-process
  crash/restart coverage at safe model boundaries.
- Write-ahead and completed tool-effect fences with explicit uncertain-effect
  refusal. Candidate mutations and process effects are never replayed from an
  ambiguous crash boundary.
- Native Anthropic Messages and Gemini GenerateContent adapters, plus bounded
  OpenAI-compatible, Anthropic, and Gemini streaming with transient live output
  and complete-turn execution authority.
- Explicit, resume-bound capability profiles and a positive-only, no-execution
  model probe available interactively and non-interactively.
- Fragmented HTTP/SSE conformance fixtures covering native headers, framing,
  tool assembly, usage, retries, contradictions, and incomplete streams.

## Shipped in 0.6 — streaming and provider intelligence

- Bounded provider event streaming with live text/tool/token/latency updates and
  cancellation-safe transcript assembly.
- Native Anthropic and Gemini adapters where their protocols cannot be represented
  faithfully by Chat Completions.
- Capability probing plus explicit user-overridable profiles for tools,
  parallel calls, context, output, reasoning, caching, and structured output.
- Provider conformance fixtures for malformed streams, retry headers, partial
  tool arguments, duplicated events, usage disagreement, and disconnects.
- Credential-safe endpoint diagnostics and deterministic fallback rules that
  never silently change a model or weaken tool semantics.

Exit criterion: local and hosted adapters pass the same conversation/tool
contract suite, with bounded memory and identical durable semantics.

## 0.7 — open tool and embedding ecosystem

- MCP client support behind the same descriptor, schema, capability, approval,
  risk, output-bound, effect, trace, and cancellation boundary as built-ins.
- Server identity pinning, command/URL allowlists, environment redaction, schema
  snapshots, timeouts, health state, and per-server enable/disable.
- Stable Rust provider/tool embedding APIs, a manifest format, examples, and
  compatibility tests for out-of-tree integrations.
- Namespaced discovery and deterministic collision handling.
- MCP resources and prompts as provenance-labelled context, never implicit
  system authority.

Exit criterion: disconnects, poisoned descriptions, malformed schemas,
oversized results, collisions, and unauthorized effects have deterministic
tests and legible CLI diagnostics.

## 0.8 — repository-scale intelligence and performance

- Incremental repository-index invalidation with content-addressed cache entries.
- Tree-sitter structure and optional LSP references without making an LSP a hard
  dependency; the bounded lexical graph remains the deterministic fallback.
- Change-impact retrieval, context usefulness telemetry, and explicit citation
  coverage without model-authored relevance claims becoming authority.
- First-class Git status/diff/history tools that are read-only by default;
  commits, branches, remotes, and hosting actions require separate capabilities.
- Large-monorepo latency, memory, descriptor-count, and context-budget suites.
- Image input as an explicit artifact capability.

Exit criterion: cold/warm performance and retrieval-relevance suites cover tiny
local models through hosted models without hidden prompt truncation.

## 0.9 — stabilization and public evaluation

- Compatibility fixtures and migrations for every durable contract, event,
  receipt, memory, settings, transaction, checkpoint, tool, and provider format.
- Fuzzing and property tests for path handling, schemas, event replay, apply,
  provider framing, MCP framing, and terminal rendering.
- Fault injection for storage exhaustion, permission loss, concurrent source
  changes, abrupt process death, network loss, and runtime cleanup failure.
- Public matched-harness evaluation measuring task correctness, regression rate,
  tokens/cost, tool efficiency, context use, trace completeness, containment,
  recovery, and human review burden—with raw artifacts and preregistration.
- Performance budgets, release-candidate soak runs, security audit closure, and
  deprecation/migration tooling.

Exit criterion: no open release-blocking correctness or security defect, all
compatibility fixtures pass on supported platforms, and evaluation claims are
reproducible from public artifacts.

## 1.0 — stable public contract

- Stable documented CLI, task/receipt/trace formats, and Rust embedding APIs.
- Published platform/provider support matrix, compatibility policy, support
  windows, security response process, and upgrade guide.
- At least one production sandbox backend with adversarial fixtures.
- Deterministic interruption recovery at every source-mutation boundary.
- Signed and attested reproducible artifacts plus checksum-verifying installers.
- Independent review of path handling, apply, process, provider, MCP, memory,
  persistence, and release boundaries, with all critical/high findings resolved.
- A final v1 evaluation report that makes only protocol-bounded, reproducible
  comparisons; benchmark execution begins only after maintainer approval.

Pactrail will not label itself 1.0 solely because the CLI is polished. The
version means downstream users can rely on the public contracts and migration
policy as well as the runtime behavior.

## Non-goals

- Hiding uncertainty behind autonomous-agent theater.
- Treating model confidence or prose as evidence.
- A universal unrestricted shell masquerading as a safe tool.
- Silent network, secret, deployment, or repository-hosting side effects.
- Provider-specific logic in the deterministic core.
- Multi-agent concurrency before single-run replay and containment are proven.

## Proposing work

Material changes should start as a design issue containing the user problem,
threat-boundary impact, contract/API shape, durable-state impact, failure and
recovery behavior, observability, test plan, and compatibility strategy. See
[CONTRIBUTING.md](../CONTRIBUTING.md).
