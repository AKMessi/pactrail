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
- Interactive `/tools` kernel inspector and expanded live latency/token/context
  activity reporting.

### Changed

- Tool results shown to models and retained verification output are bounded to
  256 KiB with explicit narrowing metadata.
- Model-limit validation now rejects output limits equal to the full context
  window.
- Tool errors provide local models with virtual workspace-path recovery guidance.

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
