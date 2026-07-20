# Changelog

All notable user-visible changes to Pactrail are documented here. The format is
based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and releases
follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Governed MCP 2025-11-25 support over bounded stdio and Streamable HTTP, with
  explicit inspect/snapshot lifecycle, atomic integrity-checked catalogs,
  local capability profiles, deterministic namespacing, selected
  provenance-labelled context, lazy execution, and live identity/schema checks.
- `pactrail mcp init|check|list|inspect|snapshot|enable|disable`, interactive
  `/mcp`, MCP-aware `/tools` and `/status`, stable JSON diagnostics, and
  cancellation-aware administrative connections.
- A distinct request-scoped `mcp_invoke` capability and independent MCP approval
  routing so native process trust cannot implicitly authorize a remote tool.
- `pactrail-sdk`, a static Rust embedding facade with subsystem/prelude exports,
  a documented custom-provider/custom-tool path, and a compile-time external
  extension compatibility fixture using the real execution kernel.

### Security

- MCP discovery never occurs during a normal run; only locally pinned snapshots
  become model-visible and their digests are bound into resume identity.
- MCP subprocesses receive no ambient environment, HTTP redirects and implicit
  retries are disabled, remote HTTP is rejected, URL/query credentials are
  rejected, results and schemas are bounded, and uncertain calls are never
  replayed.

## [0.6.0] - 2026-07-20

### Added

- Bounded SSE streaming for OpenAI-compatible Chat Completions, native
  Anthropic Messages, and native Gemini GenerateContent, with transient live
  text/tool/usage/first-byte progress and complete-turn execution authority.
- Native Anthropic message/content-block/tool-result mapping with API version
  negotiation, partial-JSON tool assembly, cache-token accounting, and strict
  event-order validation.
- Native Gemini system/content/function mapping with function IDs, stable
  synthetic IDs for older responses, thought-signature round trips, safety
  blocking, cache usage, and cumulative stream de-duplication.
- Explicit model capability overrides for native tools, parallel calls,
  structured output, vision, prompt caching, and reasoning controls. Effective
  profiles are visible in `/status` and bound into durable resume identity.
- Positive-only, no-execution capability probing through `/probe` and
  `pactrail probe`, including stable JSON output for automation.
- Fragmented local HTTP fixtures for every built-in transport plus malformed
  frame, partial argument, duplicate event, usage regression, retry, redirect,
  content-type, size, and disconnect coverage.

### Changed

- Streaming is enabled for new interactive settings while schema 1–3 settings
  migrate atomically without silently changing an existing buffered selection.
- Durable model trace actions now record the adapter, stream mode, bounded
  provider metadata, and time to first byte where available.
- Provider safety blocks are hard protocol failures and can never become an
  empty successful answer or executable tool turn.
- Tool descriptors and calls are removed from a run when native tools are
  explicitly disabled; contradictory profiles fail before provider access.

### Fixed

- Direct embedders calling buffered `ModelDriver::invoke` no longer receive an
  SSE response merely because the driver's interactive stream mode is enabled.
- Provider metadata is scalar-allowlisted, length-bounded, and terminal-safe
  before it enters the durable trace.

## [0.5.0] - 2026-07-20

### Added

- Provider-neutral, content-addressed session checkpoints bound to the exact
  hash-linked event head, task contract, isolated candidate, context,
  model/tool profiles, process-runtime attestation, token use, turn counters,
  and active wall-time budget.
- `pactrail resume <RUN_ID>` and interactive `/resume [run]`, backed by a
  bounded secret-free run manifest. Resume reconstructs the existing
  transaction and rejects contract, endpoint, model, tool, containment,
  candidate, checkpoint, or budget drift before appending another event.
- Write-ahead `effect_prepared` and reconciled `effect_completed` events for
  model-requested tools. Traces show call, risk, argument, candidate, result,
  and runtime digests; an interrupted effect is reported as uncertain and is
  never replayed automatically.
- Exclusive local execution leases combining kernel-released file locking with
  SQLite ownership metadata. A second live process is rejected, while an
  abruptly killed process can be resumed immediately.
- Real-binary crash recovery coverage that kills Pactrail during provider I/O,
  rejects a concurrent owner, resumes the same run, preserves one contract and
  one event chain, and retains cumulative token accounting.

### Changed

- New event envelopes use schema 2 for effect-fence events while schema 1
  histories remain hash-verifiable and projectable.
- The event-store database migrates atomically to schema 2 for run-lease
  metadata. Existing event, receipt, transaction, memory, and settings data
  remain readable.
- CLI and interactive trace views render effect admission and completion as
  first-class rows instead of burying effect safety inside generic tool output.

## [0.4.0] - 2026-07-20

### Added

- Three explicit process backends: fail-closed disabled execution, trusted
  native host execution, and restricted OCI execution through a locally
  attested Docker or Podman runtime. OCI runs pin the locally resolved image
  identity, mount only the isolated candidate, use a read-only root and bounded
  private temporary space, disable network and capabilities, apply CPU/memory/PID
  ceilings, and never pull or silently downgrade to native execution.
- Versioned, exact-scope process approvals with one-call, run-scoped, and deny
  decisions bound to the request, run, actor, backend identity, and profile
  digest. Policy evaluation and approval decisions are separate hash-linked
  trace events and are retained in receipts.
- End-to-end cancellation across provider requests, tools, native children, OCI
  containers, verification, repair, and interactive Ctrl-C, with bounded cleanup
  and integrity-checked candidate preservation.
- Atomic settings-schema migration for the new process modes, detailed backend
  attestation in status/doctor/traces, and a pinned hostile-repository Docker CI
  fixture covering host reads/writes, ambient secrets, network egress, root
  mutation, and daemon-socket exposure.
- Local Docker/Podman endpoint validation with credential-safe diagnostics;
  remote daemon transports fail closed before image inspection or run creation.

### Changed

- Unresolved capabilities now fail closed or require an explicit scoped
  approval instead of being treated as implicitly denied without an auditable
  decision. Non-interactive process approvals still deny by default.
- `--allow-process` and `/process on` remain deprecated compatibility aliases
  for trusted native execution; new code should use `--process-backend` or
  `/process off|native|sandbox` explicitly.

## [0.3.0] - 2026-07-19

### Added

- A deterministic repository evidence graph that connects bounded
  project-defined symbol locations with lexical reference locations, expands
  initial retrieval one hop, and is available to models through the typed
  `search_code_graph` tool. Tool queries rebuild from the isolated candidate so
  navigation cannot silently use a pre-edit index.
- Deterministic long-horizon context management that preserves recent
  observations, compacts older tool results into BLAKE3-bound navigation
  envelopes, keeps tool protocol topology intact, and records before/after
  request digests and reclaimed bytes in live and durable traces.
- Post-mutation current-source feedback for writes and exact edits, including
  content digests, changed-line bounds, bounded line-numbered previews, and
  explicit narrow re-read guidance when the changed region is not fully shown.
  Exact no-op replacements are now rejected as non-evidence-producing actions.
- A single budget-respecting validation-repair cycle: repairable deterministic
  check failures are returned as bounded, digest-bound, explicitly untrusted
  diagnostics, then the repaired candidate is independently verified again.
  Completion-gate/final phases and the controller decision are visible in live and
  durable traces.

### Fixed

- Native verification now retains an explicit non-secret Windows toolchain and
  SDK discovery environment, preventing repaired Rust candidates from failing
  spuriously with `link.exe not found` after the process environment is cleared.
  Secret variables, external Cargo target directories, and compiler wrappers
  remain excluded.

## [0.2.0] - 2026-07-18

### Added

- Public Pactrail MVB v1 model-in-the-loop evaluation runner with seven exact,
  deterministic cases, source-isolation and trace-integrity assertions,
  machine-readable results, raw receipts/traces, and a reproducible Windows
  ARM64 baseline for Qwopus3.5 9B Coder and LFM2.5 230M Fable-5.
- Explicit request-deadline and non-thinking controls for hosted and local
  OpenAI-compatible models, with bounded handling of provider retry windows.
- Repeat-run local, OpenRouter, and DeepSeek evaluation evidence with separately
  graded isolated candidates and cryptographic artifact manifests.
- A matched OpenCode comparator plus preregistered public and held-out
  real-issue replay suites for testing repository-scale behavior without
  changing the protocol after seeing model output.

### Fixed

- Generated CLI contracts now align their aggregate token and model-attempt
  ceilings with the configured context, output, and turn limits, so a valid
  `--max-turns` setting is no longer preempted by hidden default budgets.
- Exact replace and atomic multi-edit tools now accept newline-equivalent model
  text while preserving each file's existing LF or CRLF convention.
- Rust verification no longer compiles every bench and example by default, and
  a Rust `tests/` directory no longer incorrectly triggers `pytest` unless it
  contains Python tests or the repository declares Python test configuration.
- Tool-loop resilience now accepts a file path in `search`, paginates omitted
  `read_file` ranges at 300 lines, and reports continuation metadata instead of
  flooding the model conversation with an entire large source file.
- Runs that exhaust their model-turn budget after making candidate changes now
  continue through deterministic verification and explicit review with a
  completeness risk, rather than failing before the candidate can be assessed.
- OpenAI-compatible requests now coalesce all system instructions into one
  leading message. This preserves instruction priority while supporting strict
  Qwen-style llama.cpp chat templates that reject adjacent or late system
  messages.

## [0.1.0] - 2026-07-16

### Added

- Checksum-verifying one-command installers for Windows x86_64, Linux x86_64,
  and Apple Silicon macOS, backed by attested GitHub release artifacts.
- Initial verification-native Rust harness, versioned task contracts,
  capability policy, isolated workspace transactions, evidence grading,
  receipts, apply and discard.
- Hash-linked SQLite event journal and compressed content-addressed storage
  primitive.
- OpenAI-compatible model transport supporting Ollama and local/hosted
  compatible APIs.
- Typed read/search/write/replace/remove/process tools and automatic repository
  verification discovery.
- Interactive persistent CLI, immutable review diffs, run history, model
  discovery/configuration, native completion, doctor, and JSON automation.
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

[Unreleased]: https://github.com/AKMessi/pactrail/compare/v0.6.0...HEAD
[0.6.0]: https://github.com/AKMessi/pactrail/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/AKMessi/pactrail/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/AKMessi/pactrail/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/AKMessi/pactrail/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/AKMessi/pactrail/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/AKMessi/pactrail/releases/tag/v0.1.0
