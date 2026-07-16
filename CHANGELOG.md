# Changelog

All notable user-visible changes to Pactrail are documented here. The format is
based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and releases
follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Provenance-aware workspace memory with explicit user commands, model read-only
  recall, and integrity-checked applied-receipt history.
- Tool Kernel v2 batch read, atomic multi-edit, candidate-change inspection, and
  tool behavior/risk annotations.
- Concurrent scheduling for consecutive parallel-safe reads with deterministic
  result journaling.
- Model-aware context-pack budgets and explicit directory scope for nested
  `AGENTS.md` instructions.
- Integrity-checked model/tool/verification execution timelines, interactive
  `/trace`, scriptable `pactrail trace`, and portable run-local JSONL.
- Failed-run trace export, run-ID diagnostics, and receipt-independent run
  history so model/protocol failures remain discoverable after restart.
- Interactive `/tools` kernel inspector and expanded live latency/token/context
  activity reporting.
- First-class `Answered` receipts and terminal `Completed` runs for
  informational prompts, with no fake candidate or apply step.
- Conventional project-anchor fallback, bounded current anchor previews, and
  deterministic ecosystem/entrypoint profiles for broad workspace questions.
- Bounded weak-model recovery that steers repeated successful read-only calls,
  then permits one budgeted tool-free synthesis turn for informational goals.
- Distinct live and trace presentation for recovery and deterministic answer
  grounding.
- Persistent live execution timelines with durable run identity, sanitized
  context/model/tool/state/recovery/verification rows, and success/failure
  metric footers.
- A unified framed terminal dashboard, integrity receipt cards, clearer
  answer/review handoffs, and sequence-numbered `/trace` inspection with a
  verified run summary and legend.
- Width-aware layouts for dashboards, help, tools, status, receipts, run
  history, and complete trace continuations, with narrow-terminal ConPTY
  validation and resolvable durable identifiers.

### Changed

- Deterministic verification now runs from a disposable candidate snapshot so
  build products and test-runner caches cannot pollute review receipts.
- Tool results shown to models and retained verification output are bounded to
  256 KiB with explicit narrowing metadata.
- Model-limit validation now rejects output limits equal to the full context
  window.
- Tool errors provide local models with virtual workspace-path recovery guidance.
- `doctor` now distinguishes Pactrail's shipped transaction isolation from an
  externally managed container boundary for hostile repositories.
- Model discovery now bounds entry count and identifier size and rejects
  control-character identifiers before they reach selection or settings.

### Fixed

- Repeated `discard` calls and interrupted discard receipt writes now recover
  from the durable event head after the candidate workspace has been removed.
- Memory lookup accepts canonical or compact UUID prefixes beyond the first
  hyphen, so nearby UUIDv7 entries remain individually addressable.
- Bounded run-goal and memory previews now end with a visible ellipsis instead
  of silently dropping continuation text.

## [0.1.0] - 2026-07-15

### Added

- Initial verification-native Rust harness, versioned task contracts, capability
  policy, isolated workspace transactions, evidence grading, receipts, apply and
  discard.
- Hash-linked SQLite event journal and compressed content-addressed storage
  primitive.
- OpenAI-compatible model transport supporting Ollama and local/hosted compatible
  APIs.
- Typed read/search/write/replace/remove/process tools and automatic repository
  verification discovery.
- Interactive persistent CLI, immutable review diffs, run history, model
  discovery/configuration, native completion, doctor, and JSON automation.

[Unreleased]: https://github.com/AKMessi/pactrail/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/AKMessi/pactrail/releases/tag/v0.1.0
