# Roadmap

Pactrail's roadmap is organized by trust boundary, not by feature count. A
feature is complete only when its policy, trace, recovery, tests, and user-facing
failure mode are complete.

This file distinguishes shipped behavior from planned work. It is not a release
promise or a security claim.

## Shipped foundation — 0.1

- Contract-first run lifecycle and capability overgrant detection.
- Isolated workspace transactions, receipt-bound review/apply, drift detection,
  crash journal, rollback, and idempotent recovery.
- Provider-neutral model IR and bounded OpenAI-compatible transport.
- Tool Kernel v2 with schemas, risk/behavior annotations, batch read, atomic
  multi-edit, change inspection, recall, bounded results, and safe read batching.
- Model-aware context budgets and correctly scoped `AGENTS.md` instructions.
- Grounded broad-question context with anchor previews, deterministic project
  profiles, first-class answered runs, and bounded weak-model loop recovery.
- Provenance-aware workspace memory with human and applied-receipt write paths.
- Hash-linked detailed execution traces and portable JSONL.
- Bounded repository evidence graph with typed current-candidate symbol and
  lexical-reference search.
- Interactive review-focused CLI plus scriptable JSON interface.
- Cross-platform CI, dependency policy, and attested GitHub release workflow.

## 0.2 — containment and approvals

Highest priority is making untrusted-code execution a real enforceable boundary.

- A `ProcessBackend` abstraction with an OCI runner on Linux/macOS/Windows hosts.
- Explicit filesystem, network, environment, CPU, memory, PID, and wall-time
  profiles surfaced in contract, status, trace, and receipt.
- Fail-closed runtime detection and a visible sandbox-strength vocabulary; never
  silently fall back from containerized to native execution.
- Interactive approval objects with exact capability/resource/expiry scope,
  durable decisions, and non-interactive denial by default.
- Cancellation that propagates through model requests, tools, processes,
  verification, and UI without corrupting the event lifecycle.

Exit criterion: hostile-repository fixtures demonstrate that forbidden host
reads, writes, network, and process escape are blocked by the configured backend.

## 0.3 — open tool ecosystem

- MCP client support behind the same `ToolDescriptor`, capability, risk,
  output-bound, effect, and trace boundary as built-ins.
- Server identity pinning, command/URL allowlists, environment redaction,
  schema snapshots, timeouts, health state, and per-server enable/disable.
- A stable Rust tool SDK and manifest format for out-of-tree tools.
- Namespaced tool discovery and collision handling.
- Resource and prompt ingestion as provenance-labelled context, never implicit
  system authority.

Exit criterion: disconnects, malformed schemas, oversized results, name
collisions, poisoned descriptions, and unauthorized effects have deterministic
tests and legible CLI diagnostics.

## 0.4 — model and context intelligence

- Streaming provider responses and live token/latency display with bounded
  transcript retention.
- Native Anthropic and Gemini adapters where their protocols offer capabilities
  that cannot be represented faithfully by Chat Completions.
- Capability probing and user-overridable model profiles for tools, parallelism,
  context, output, vision, caching, and structured output.
- Incremental repository index invalidation; tree-sitter structure and optional
  LSP references without making an LSP a hard dependency.
- Tree-sitter/type-aware graph enrichment, incremental index invalidation,
  context usefulness telemetry, and deterministic compaction summaries with
  provenance. The shipped lexical evidence graph remains the bounded fallback.
- Image input as an explicit artifact capability.

Exit criterion: context-overflow and retrieval-relevance suites cover small
local models through large hosted models, with no hidden prompt truncation.

## 0.5 — long-running work and collaboration

- Durable resumable sessions built from event replay rather than serialized
  model internals.
- Checkpoints, branches, and task decomposition with isolated child contracts,
  budgets, workspaces, and receipts.
- Patch-stack and multi-candidate comparison without shared mutable tool state.
- Agent Client Protocol support for editor/IDE surfaces.
- Background verification with explicit lifecycle states and cancellation.

Exit criterion: crash/restart, branch conflict, partial completion, and nested
budget cases replay identically on all supported platforms.

## 1.0 bar

Pactrail will not label itself 1.0 solely because the CLI is polished. The bar is:

- documented and compatibility-tested contracts/events/receipts/memory formats;
- a stable public tool/provider embedding API;
- at least one production sandbox backend with adversarial fixtures;
- deterministic recovery from interruption at every source-mutation boundary;
- a public eval harness measuring task success, regression rate, tool
  efficiency, context use, trace completeness, and human review burden;
- signed/attested reproducible release artifacts and a published support policy;
- independent security review of path handling, apply, process, provider, MCP,
  and memory boundaries.

## Non-goals

- Hiding uncertainty behind autonomous-agent theater.
- Treating a model's confidence or prose as evidence.
- A universal unrestricted shell masquerading as a safe tool.
- Silent network, secret, deployment, or repository-hosting side effects.
- Provider-specific logic in the deterministic core.
- Multi-agent concurrency before single-run state and containment are trustworthy.

## Proposing work

Material changes should start as a design issue containing the user problem,
threat-boundary impact, contract/API shape, durable-state impact, failure and
recovery behavior, observability, test plan, and compatibility strategy. See
[CONTRIBUTING.md](../CONTRIBUTING.md).
