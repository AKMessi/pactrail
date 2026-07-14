# Pactrail

**Every change carries its evidence.**

Pactrail is a verification-native coding agent harness written in Rust. A task
becomes an explicit contract, model writes happen in an isolated transaction,
and a tamper-evident change receipt is produced before the working tree can be
modified.

Pactrail is not a role-playing agent swarm. Its core abstraction is a durable,
evidence-backed software transaction.

## What works today

- Local Ollama and HTTPS OpenAI-compatible model endpoints.
- A provider-neutral Rust model and tool API.
- Typed, JSON Schema-described read, search, edit, delete, and process tools.
- Strict workspace-relative path and write-scope enforcement.
- Copy-on-run transactions for Git and plain directories.
- Receipt-bound landing, baseline content/mode drift detection, idempotent apply,
  crash journal, rollback, and discard.
- Append-only SQLite events with hash-chain integrity.
- BLAKE3-addressed compressed artifact storage.
- Repository topology, language, symbol, import, and hierarchical `AGENTS.md` indexing.
- Model turn, token, process, output, and filesystem limits.
- Automatic Rust, Go, Python, and JavaScript test discovery.
- Evidence grades that distinguish deterministic results from observations and model opinions.
- Human-readable and JSON CLI output, task files, receipts, inspection, and run listing.

## Install

Until signed release binaries are published, install from the repository:

```console
cargo install --git https://github.com/AKMessi/pactrail.git --locked pactrail
```

Pactrail requires Rust 1.95 or newer when building from source.

## Quick start

Run with a local Ollama model:

```console
pactrail run "Fix the failing parser tests" --model qwen3-coder
```

Run against an HTTPS OpenAI-compatible endpoint without putting a key in shell history:

```console
export OPENAI_API_KEY="..."
pactrail run "Add regression tests for issue 42" \
  --provider open-ai-compatible \
  --base-url https://models.example.com/v1 \
  --model coding-model
```

The source tree is still untouched when the run finishes. Inspect and land it explicitly:

```console
pactrail inspect <RUN_ID>
pactrail apply <RUN_ID>
```

Generate a complete TOML contract for CI or repeatable work:

```console
pactrail task-template "Refactor the cache without changing behavior" > pactrail.task.toml
pactrail run --task pactrail.task.toml --model qwen3-coder --output json
```

Process execution is disabled unless the contract records its full effective
authority or `--allow-process` is passed. Native processes can access the host
filesystem, network, secrets, and external services; the workspace transaction
protects receipt landing, but it is not a host sandbox. Use only trusted
repositories until the OCI runner is available.

## Core workflow

1. Validate the task contract and capability policy.
2. Snapshot the repository into an isolated transaction.
3. Compile deterministic repository context for the selected model.
4. Execute typed tools while recording declared and observed effects.
5. Run detected verification commands when process capability is granted.
6. Grade evidence and issue a hash-protected change receipt.
7. Apply only after receipt validation and baseline-drift checks.

See [Architecture](docs/architecture.md), [Threat model](docs/threat-model.md),
and [Provider compatibility](docs/providers.md).

## Development

```console
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo doc --workspace --all-features --no-deps --locked
```

The end-to-end suite runs a real Pactrail child process against a local mock
provider, confirms the source is untouched, and then lands the receipt in a
second process.

## Status and compatibility

The current `0.x` line is a production-quality developer preview. Receipt,
event, and contract formats are explicitly versioned, but public Rust APIs may
still change before 1.0. See the repository milestones for ACP, MCP, stronger
native/OCI sandboxing, native Anthropic/Gemini adapters, and the full TUI.

## License

Licensed under either Apache License 2.0 or MIT at your option.
