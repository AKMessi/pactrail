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
- An interactive, persistent terminal session with model discovery, history,
  completion, review, diff, apply, and discard commands.
- Scriptable human-readable and JSON output, task files, receipts, inspection,
  and run listing.

## Install

Until signed release binaries are published, install from the repository:

```console
cargo install --git https://github.com/AKMessi/pactrail.git --locked pactrail
```

Pactrail requires Rust 1.95 or newer when building from source.

For a local checkout, install the current source with:

```console
cargo install --path crates/pactrail-cli --locked --force
```

Cargo places the executable in its binary directory (normally `~/.cargo/bin`).
Once that directory is on `PATH`, `pactrail` works from any terminal and any
repository.

## Quick start

Open a terminal in the repository you want to work on and start Pactrail:

```console
pactrail
```

The interactive session discovers models from local Ollama on first launch.
Type a software task directly, or use `/models` and `/model` to choose a model:

```text
/models
/model 1
Fix the failing parser tests and add a regression test
/diff
/apply
```

Pactrail runs every model edit in an isolated transaction. `/diff` reviews the
immutable run artifact; `/apply` performs integrity and source-drift checks
before touching the working tree. `/discard` removes the candidate while
preserving its receipt and diff.

Connect a llama.cpp, vLLM, LM Studio, SGLang, or other OpenAI-compatible server
inside the session:

```text
/connect http://127.0.0.1:8080/v1 model-id
```

Use `/help` for the command palette and `/status` for the active endpoint,
model, token limits, and safety policy. Settings and input history persist
across sessions.

The non-interactive interface remains available for scripts and CI. Run with a
local Ollama model:

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
[Interactive CLI](docs/interactive-cli.md), and
[Provider compatibility](docs/providers.md).

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
native/OCI sandboxing, native Anthropic/Gemini adapters, and richer terminal
visualization.

## License

Licensed under either Apache License 2.0 or MIT at your option.
