# Pactrail

[![CI](https://github.com/AKMessi/pactrail/actions/workflows/ci.yml/badge.svg)](https://github.com/AKMessi/pactrail/actions/workflows/ci.yml)
[![Rust 1.95+](https://img.shields.io/badge/rust-1.95%2B-orange.svg)](rust-toolchain.toml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

**Every change carries its evidence.**

Pactrail is a model-agnostic coding-agent harness written in Rust. It combines
the direct terminal flow of a coding assistant with a stricter execution model:
tasks become contracts, edits happen in isolated transactions, actions become
tamper-evident traces, and the source workspace changes only after explicit,
receipt-bound apply.

The central abstraction is not a chat wrapper or an agent persona. It is a
durable, inspectable software change transaction.

```text
  P A C T R A I L  0.1.0
  verification-native coding · every change carries evidence

  workspace  C:\work\project
  model      local-coder
  safety     isolated edits · native processes blocked
  review     no candidates waiting
  memory     3 memories · /memory

pactrail ❯ Fix the parser regression and add a test.
```

## Why Pactrail is different

- **The model proposes; the kernel disposes.** Models never receive a raw host
  filesystem or an untyped shell. JSON Schema tools, capability policy, path
  confinement, budgets, and transaction state remain deterministic Rust code.
- **Review is a hard boundary.** Candidate edits live under `.pactrail/runs`.
  `/diff` reads an immutable review artifact; `/apply` rechecks the receipt,
  candidate bytes, file modes, and source baseline before landing anything.
- **Memory has provenance.** Explicit conventions, decisions, and warnings live
  in a durable SQLite store. Applied receipts can create integrity-checked
  historical records. The model can recall memory but cannot write it.
- **Context adapts to the model.** Repository topology, scoped `AGENTS.md`
  instructions, and relevant memories are compiled under a model-derived byte
  budget. Authoritative context fails closed; optional entries are omitted whole
  and the omission is visible.
- **Traces describe reality.** Model latency and token deltas, tool duration,
  risk, argument digests, output bounds, observed effects, verification, policy,
  evidence, and state transitions are hash-linked and available as portable
  JSONL.
- **Weak models degrade gracefully.** Broad questions receive bounded current
  anchor previews and a deterministic ecosystem/entrypoint profile. Repeated
  successful read-only loops get one tool-free synthesis turn; invalid loops
  still fail closed, while coherent candidate edits remain reviewable.

## Shipped in 0.1

### Interactive experience

- Start from any repository with `pactrail`; optionally pass the first task.
- Persistent history, completion, typo suggestions, review-aware prompt, and
  truthful live phases instead of simulated activity.
- `/tools` risk/capability inspector, `/trace` execution timeline, `/memory`
  browser, `/runs`, `/review`, immutable `/diff`, explicit `/apply` and
  `/discard`, `/doctor`, model discovery, and persistent provider settings.
- Human-readable output by default and stable JSON for scripts.
- Informational prompts finish as `ANSWERED` with no fake apply step. Kernel
  facts and model explanation remain visibly distinct for broad workspace
  overviews.

### Tool Kernel v2

- Bounded file listing, single and batch reads, lexical search, exact replace,
  atomic multi-edit, write, remove, candidate-change inspection, memory recall,
  and trusted native verification.
- Per-tool read-only, idempotency, parallel-safety, capability, and risk metadata.
- Consecutive parallel-safe reads overlap; mutations stay serial and durable
  results retain the model's call order.
- Model-visible results are bounded to 256 KiB with explicit narrowing guidance.

### Durable safety and state

- Git-aware or plain-directory copy-on-run transactions.
- Workspace-relative path enforcement, write-scope enforcement, symlink and
  special-file rejection, and concurrent source-drift protection.
- Idempotent apply with a synchronized crash journal and rollback.
- SQLite WAL event and memory stores with full synchronization and fail-closed
  schema versions.
- BLAKE3 hash-linked events, integrity-protected receipts, and a tested
  compressed content-addressed artifact-store primitive.
- Automatic Rust, Go, Python, and JavaScript verification discovery.

### Model portability

- Ollama, OpenAI, llama.cpp, vLLM, SGLang, LM Studio, LocalAI, and compatible
  hosted gateways through a bounded OpenAI Chat Completions adapter.
- A provider-neutral Rust `ModelDriver`, ordered conversation IR, typed tool
  calls/results, finish reasons, usage, request IDs, and extension metadata.
- Remote endpoints require HTTPS. Plain HTTP is restricted to exact loopback
  hosts, redirects are disabled, and credentials are read from environment
  variables rather than CLI values or settings files.

## Install

Install the current source from GitHub:

```console
cargo install --git https://github.com/AKMessi/pactrail.git --locked pactrail
```

Or from a local checkout:

```console
cargo install --path crates/pactrail-cli --locked --force
```

Building requires Rust 1.95 or newer. Cargo normally installs the executable in
`~/.cargo/bin`; put that directory on `PATH` and `pactrail` will start from any
terminal.

## Quick start

Open a terminal in a project and launch the session:

```console
pactrail
```

Pactrail discovers local Ollama models on first launch. A normal review loop is:

```text
/models
/model 1
Fix the failing parser tests and add a regression test.
/trace
/diff
/apply
```

You can also ask normal repository questions. They produce terminal `ANSWERED`
runs with integrity-checked receipts and traces, but no candidate or apply step:

```text
whats this directory about
/trace
```

Connect any local OpenAI-compatible server without restarting:

```text
/connect http://127.0.0.1:8080/v1 model-id
/context 4096
/output-tokens 512
/turns 8
```

For a local GGUF model, start its llama.cpp-compatible server separately, then
connect Pactrail to its `/v1` endpoint. A key variable may contain any non-empty
placeholder if that server requires an authorization header; Pactrail does not
need a real remote credential for a local endpoint.

Native processes are disabled by default. `/process on` permits detected tests
and other registered process calls, but those children have the host process's
filesystem, network, secret, and external-service authority. The edit
transaction is not an operating-system sandbox. Enable this only for trusted
repositories.

## Automation and CI

No-subcommand mode intentionally requires a terminal. Use subcommands in scripts:

```console
pactrail run "Fix the parser" --model qwen3-coder --output json
pactrail trace <RUN_ID> --json
pactrail inspect <RUN_ID> --json
pactrail apply <RUN_ID> --json
```

Repeatable work can use a complete versioned contract:

```console
pactrail task-template "Refactor the cache without changing behavior" > pactrail.task.toml
pactrail run --task pactrail.task.toml --model qwen3-coder --output json
```

Other discovery commands include `pactrail tools --json`, `pactrail schema`,
`pactrail memory list`, `pactrail list`, `pactrail doctor`, and
`pactrail completion <shell>`.

## Architecture at a glance

```text
TaskContract ──> model-budgeted ContextPack ──> ModelDriver
     │                       │                       │
     │                 provenance memory       typed calls
     │                                               │
     ├──> PolicyEngine ──> Tool Kernel ──> WorkspaceTransaction
     │                           │                  │
     └──> hash-linked EventStore <──── effects ────┘
                    │
              Evidence + ChangeReceipt ──> apply / discard
```

The nine crates keep the core domain, storage, memory, context, models, tools,
workspace transactions, engine, and CLI independently testable. See
[Architecture](docs/architecture.md), [Threat model](docs/threat-model.md),
[Interactive CLI](docs/interactive-cli.md), [Providers](docs/providers.md), and
[Roadmap](docs/roadmap.md).

## Durable local layout

Pactrail keeps its state in `WORKSPACE/.pactrail` by default:

```text
.pactrail/
├── events.sqlite3        # authoritative hash-linked event journal
├── memory.sqlite3        # provenance-aware workspace memory
└── runs/<run-id>/
    ├── transaction.json  # isolated workspace metadata
    ├── workspace/        # model-visible candidate tree
    ├── receipt.json      # integrity-protected change receipt
    ├── review.diff       # immutable review artifact
    └── trace.jsonl       # portable verified execution timeline
```

## Development

```console
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo doc --workspace --all-features --no-deps --locked
```

CI runs those gates plus release builds on Linux, macOS, and Windows, and applies
the dependency/license policy in `deny.toml`. Start with
[CONTRIBUTING.md](CONTRIBUTING.md) and [Development](docs/development.md).

## Project status

Pactrail 0.1 is a production-grade developer preview: its invariants and failure
modes are tested, while Rust APIs and versioned local formats may still evolve
before 1.0. OCI/OS sandbox backends, MCP, native provider adapters, streaming,
and richer retrieval are roadmap work—not current security claims.

## License

Licensed under either [Apache License 2.0](LICENSE-APACHE) or
[MIT](LICENSE-MIT), at your option.
