# Pactrail

[![CI](https://github.com/AKMessi/pactrail/actions/workflows/ci.yml/badge.svg)](https://github.com/AKMessi/pactrail/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/AKMessi/pactrail)](https://github.com/AKMessi/pactrail/releases/latest)
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
  ╭─ P A C T R A I L  v1.0.0
  │  verification-native coding · every change carries evidence
  ├─
  │ workspace  C:\work\project
  │ runtime    local-coder  · open-ai-compatible
  │ safety     isolated edits · native processes blocked
  │ trace      live timeline · durable hash chain · /trace
  │ review     no candidates waiting
  │ memory     3 memories · /memory
  ╰─ Describe a task · /help commands · // escapes a leading slash

pactrail ❯ Fix the parser regression and add a test.

  ╭─ RUN 019f7a31  compatible/local-coder
  │ Fix the parser regression and add a test.
  │     0ms  ◇ state     contracting · validating task contract
  │     4ms  ◆ context   186 indexed · 181 warm · 5 cold · 140 parsed · 46 lexical · 8 cited · 100.00% coverage · 4ms
  │   1.82s  ● model     turn 1 · 2 actions · 3,412 tokens · 1.8s
  │   1.84s  ● tool      read_many_files · 18.1 KiB · 12ms
  │   1.86s  ◆ tool      edit_file · changed src/parser.rs · 9ms
  │   3.21s  ✓ verify    Rust workspace tests · passed · 1.3s
  ╰─ ✓ complete  2 turns · 4 tools · 6,104 tokens · 2.9s model · 3.2s
```

## Why Pactrail is different

- **The model proposes; the kernel disposes.** Models never receive a raw host
  filesystem or an untyped shell. JSON Schema tools, capability policy, path
  confinement, budgets, and transaction state remain deterministic Rust code.
- **Review is a hard boundary.** Candidate edits live under `.pactrail/runs`.
  `/diff` reads an immutable review artifact; `/apply` rechecks the receipt,
  candidate bytes, file modes, and source baseline before landing anything.
- **Crashes are explicit state, not lost work.** Provider-neutral checkpoints
  bind the conversation and controller to the event head, candidate, runtime,
  tools, and budgets. `pactrail resume` continues only at a proven safe point;
  an uncertain tool effect is named in the trace and never guessed or replayed.
- **Memory has provenance.** Explicit conventions, decisions, and warnings live
  in a durable SQLite store. Applied receipts can create integrity-checked
  historical records. The model can recall memory but cannot write it.
- **Context adapts to the model.** Repository topology, scoped `AGENTS.md`
  instructions, relevant memories, and a bounded repository evidence graph are
  compiled under a model-derived byte budget. Project-defined symbols link to
  lexical reference locations for navigation without pretending to be a runtime
  call graph. Authoritative context fails closed; optional entries are omitted
  whole and the omission is visible.
- **Repository intelligence is incremental, not stale.** Every run hashes
  current files, while unchanged bounded structure is reused by content digest.
  The live timeline and durable trace report warm/cold reuse, rejected entries,
  graph evidence, and kernel-derived citation coverage.
- **Structure has a real parser and a real fallback.** Bounded embedded
  Tree-sitter grammars cover Rust, Python, JavaScript, and TypeScript/TSX;
  unsupported or exhausted parses use the lexical index. Optional SDK-provided
  LSP snapshots preserve lexical, language-server, and corroborated provenance
  without Pactrail silently starting an external process.
- **Images are artifacts, not ambient URLs.** Explicit PNG, JPEG, and WebP
  inputs are read once, header-validated, dimension/byte bounded, BLAKE3-sealed,
  stripped of host paths, and carried by the provider-neutral conversation.
  Their estimated visual-token cost is reserved before repository context is
  built, and checkpoint resume restores the exact sealed bytes.
- **Git is evidence, not ambient shell authority.** Built-in status, diff, and
  history tools read the exact source repository in process with hard bounds.
  HEAD/index/raw-worktree evidence and Pactrail's isolated candidate stay
  visibly separate; hooks, filters, credentials, remotes, and commands are not
  part of the boundary.
- **Long runs stay evidence-dense.** Pactrail measures the serialized
  conversation and tool schemas against a model-derived high-water mark. Old
  tool results become deterministic, digest-bound navigation envelopes while
  the newest evidence stays lossless whenever possible. Tool-call/result pairs
  remain valid and every compaction is visible in the live and durable trace.
- **Traces describe reality.** Model latency and token deltas, tool duration,
  risk, argument digests, output bounds, observed effects, verification, policy,
  evidence, and state transitions are hash-linked and available as portable
  JSONL.
- **Open tools do not become open authority.** MCP servers are explicit,
  locally profiled, and snapshot-pinned. Discovery is an operator action, never
  hidden run behavior. Every invocation crosses a dedicated `mcp_invoke` gate
  plus its network/process/secret/write capabilities, exact approval, effect
  fence, output bound, cancellation path, and durable trace.
- **Weak models degrade gracefully.** Broad questions receive bounded current
  anchor previews and a deterministic ecosystem/entrypoint profile. Repeated
  successful read-only loops get one tool-free synthesis turn. Change tasks get
  a fixed discovery allowance, then a kernel-enforced implementation tool set;
  varied calls returning equivalent evidence still count as stalled progress.
  Invalid loops fail closed, while coherent candidate edits remain reviewable.
- **Verification interrupts mistakes, not just completion.** With authorized
  process isolation, the controller checks a changed candidate before the next
  model turn. One repairable failure returns bounded, untrusted diagnostics to
  the model; a passing digest-bound gate is reused as final evidence if no later
  edit invalidates it.

## Shipped foundation

### Interactive experience

- Start from any repository with `pactrail`; optionally pass the first task.
- Persistent history, completion, typo suggestions, review-aware prompt, and
  a persistent live execution timeline instead of simulated or disappearing
  activity. Completed context, model, tool, recovery, state, and verification
  rows stay visible above one current-operation spinner.
- Width-aware rendering keeps dashboards, receipts, tools, help, history, and
  complete trace continuations legible in narrow or wide terminals.
- `/tools` risk/capability inspector, `/trace` execution timeline, `/memory`
  browser, `/runs`, `/resume`, `/review`, immutable `/diff`, explicit `/apply` and
  `/discard`, `/doctor`, model discovery, and persistent provider settings.
- Human-readable output by default and stable JSON for scripts.
- Informational prompts finish as `ANSWERED` with no fake apply step. Kernel
  facts and model explanation remain visibly distinct for broad workspace
  overviews.

### Tool Kernel v2

- Bounded file listing, single and batch reads, lexical search, exact replace,
  repository-wide symbol/reference graph and one-hop change-impact search,
  read-only Git status/diff/history, atomic multi-edit, write, remove,
  candidate-change inspection, memory recall, and trusted native verification.
- Per-tool read-only, idempotency, parallel-safety, capability, and risk metadata.
- Consecutive parallel-safe reads overlap; mutations stay serial and durable
  results retain the model's call order.
- Model-visible results are bounded to 256 KiB with explicit narrowing guidance.
- Successful writes and exact edits return digest-bound, line-numbered source
  previews from the isolated candidate around the changed region. Distant or
  oversized changes carry explicit re-read guidance instead of implying that a
  partial preview is complete; no-op replacements are rejected.
- Conversation growth is bounded independently: older observations are
  compacted locally with BLAKE3 provenance, high-signal anchors, an exact JSON
  preview, and re-read guidance. No model-generated history summary becomes an
  authority.

### Governed MCP ecosystem

- Stable MCP 2025-11-25 client support over bounded stdio and Streamable HTTP,
  using the official Rust protocol types behind a Pactrail-owned adapter.
- `.pactrail/mcp.toml` assigns local authority. Server descriptions, schemas,
  annotations, resources, prompts, and results remain untrusted data.
- `pactrail mcp inspect <server>` performs explicit, read-only discovery.
  `pactrail mcp snapshot <server>` atomically pins the negotiated server
  identity, executable/endpoint identity, selected schemas, local profiles, and
  selected advisory context under an integrity digest.
- Normal runs never discover tools. They load only enabled, valid snapshots,
  namespace every tool as `mcp__<server>__<tool>`, reject collisions, and bind
  the snapshot set into durable resume identity.
- Local subprocess servers start without a shell or ambient environment. Remote
  servers require HTTPS except for explicit literal loopback HTTP; redirects,
  URL credentials/query secrets, OAuth discovery, implicit retries, and hidden
  replay are disabled.
- A shared run-local health handle moves through ready, connecting, healthy,
  stale, and failed states. Success traces retain the health transition;
  identity/schema drift becomes stale and other failures remain explicit.
- `/mcp`, `/tools`, and `/status` expose configured state and pinned tools in the
  interactive CLI. All lifecycle operations also have stable JSON-capable
  subcommands for automation.

### Durable safety and state

- Git-aware or plain-directory copy-on-run transactions.
- Workspace-relative path enforcement, write-scope enforcement, symlink and
  special-file rejection, and concurrent source-drift protection.
- Idempotent apply with synchronized atomic source replacement, a crash journal,
  deterministic permission/storage fault coverage, and rollback.
- SQLite WAL event and memory stores with full synchronization and fail-closed
  schema versions.
- BLAKE3 hash-linked events, integrity-protected receipts, and a tested
  compressed content-addressed artifact-store primitive.
- Content-addressed session checkpoints, secret-free runtime manifests,
  immediate crash recovery, exclusive OS/SQLite run leases, and write-ahead
  effect fences visible in both human and JSON traces.
- Automatic Rust, Go, Python, and JavaScript verification discovery.
- Up to two proactive candidate checks, with one bounded validation-repair
  cycle when an authorized deterministic check fails. Diagnostics are
  byte-budgeted to the configured context window, labelled as untrusted process
  output, and bound to the exact candidate digest; later edits invalidate them.

### Model portability

- Ollama, OpenAI, llama.cpp, vLLM, SGLang, LM Studio, LocalAI, and compatible
  hosted gateways through a bounded OpenAI Chat Completions adapter.
- Native Anthropic Messages and Gemini GenerateContent adapters preserve typed
  content blocks, function-call IDs, cached-token accounting, Gemini thought
  signatures, and provider finish semantics without compatibility shims.
- Explicit image input maps the same sealed artifact to OpenAI-compatible data
  URLs, Anthropic base64 image blocks, and Gemini inline data. Four images,
  4 MiB each and 12 MiB total are accepted; PNG, JPEG, and WebP form the
  deliberately portable format intersection.
- Bounded SSE streaming across all built-in transports provides live sanitized
  text, tool, usage, and first-byte progress. Partial calls are never executable;
  malformed framing and disconnects fail without silent buffered fallback.
- A provider-neutral Rust `ModelDriver`, ordered conversation IR, typed tool
  calls/results, finish reasons, usage, request IDs, and extension metadata.
- Explicit capability profiles and positive-only `/probe`/`pactrail probe`
  diagnostics make model assumptions visible and resume-bound.
- Remote endpoints require HTTPS. Plain HTTP is restricted to exact loopback
  hosts, redirects are disabled, and credentials are read from environment
  variables rather than CLI values or settings files.

### Rust embedding

`pactrail-sdk` is a static facade for applications embedding the real Pactrail
kernel. It reexports the provider-neutral `ModelDriver`, typed `Tool`, policy,
engine, MCP, transaction, store, checkpoint, memory, and context contracts. An
out-of-tree-style compatibility fixture implements a custom provider and tool
and composes them with `RunEngine`. See the [embedding guide](docs/embedding.md).
The v1 SDK is consumed from an immutable Git tag and follows the documented 1.x
SemVer contract. Workspace implementation crates remain internal; crates.io
publication is not part of the 1.0 distribution contract.

## Install

Install the latest checksum-verified release without Rust.

Windows PowerShell 5.1 or newer:

```powershell
irm https://raw.githubusercontent.com/AKMessi/pactrail/main/install.ps1 | iex
```

Linux x86_64 or Apple Silicon macOS:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://raw.githubusercontent.com/AKMessi/pactrail/main/install.sh | sh
```

The Windows installer uses
`%LOCALAPPDATA%\Programs\Pactrail\bin` and adds it to the user `PATH`. The Unix
installer uses `~/.local/bin` by default and reports when that directory is not
on `PATH`. Both download the release checksum manifest and reject an archive
whose SHA-256 digest does not match. Set `PACTRAIL_INSTALL_DIR` to choose a Unix
destination or pass `-InstallDir` when running a downloaded PowerShell script.

Prebuilt binaries and checksums are also available on the
[latest release](https://github.com/AKMessi/pactrail/releases/latest). Current
release targets are Windows x86_64, Linux x86_64, and Apple Silicon macOS.

To build the current source with Rust 1.95 or newer:

```console
cargo install --git https://github.com/AKMessi/pactrail.git --tag v1.0.0 --locked pactrail
```

Or from a local checkout:

```console
cargo install --path crates/pactrail-cli --locked --force
```

Cargo normally installs the executable in `~/.cargo/bin`; put that directory on
`PATH` and `pactrail` will start from any terminal.

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

Attach screenshots or visual references without exposing a host path:

```text
/capability vision on
/image add "C:\work\references\failure.png"
Explain this failure and fix the responsible code.
```

For a one-shot run, repeat `--image` as needed:

```console
pactrail run --vision on --image failure.png --image expected.webp "Match the expected UI"
```

Only enable `vision` for a model that actually accepts image input. The image
filename, sealed bytes, media type, dimensions, and digest become run-local
checkpoint state; the original path does not. The bytes are sent to the chosen
model provider on every turn, so do not attach secrets you would not send to
that provider.

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

Processes are disabled by default. For untrusted repositories, select the
restricted OCI backend with a prebuilt local image:

```text
/process sandbox pactrail-rust:local docker
```

Pactrail resolves the local image to an immutable digest and never pulls during
a run. Each command gets only the candidate workspace, a read-only image root,
private temporary storage, no network, no capabilities, no ambient environment,
and bounded CPU, memory, PIDs, output, and wall time. Docker or Podman and the
host kernel/desktop VM remain trusted infrastructure; this boundary is labelled
`oci_restricted`, not "fully sandboxed."

`/process native` is available for trusted repositories. It runs directly on
the host with broad filesystem, network, secret, and external-service authority.
Every process request still requires an exact scoped approval recorded in the
trace and receipt. `/process off` restores the fail-closed default.

### Add an MCP server

Initialize a workspace-local manifest:

```console
pactrail mcp init
```

Add a disabled server to `.pactrail/mcp.toml`. This example deliberately keeps
credentials in an environment variable and grants one advertised read-only tool
only network authority:

```toml
schema = 1

[[servers]]
name = "issues"
enabled = false
environment = ["ISSUES_MCP_TOKEN"]
startup_timeout_seconds = 30
request_timeout_seconds = 30
max_output_bytes = 262144
resources = []
prompts = []

[servers.transport]
kind = "streamable-http"
url = "https://mcp.example.com/mcp"
bearer_token_env = "ISSUES_MCP_TOKEN"

[servers.tools.search_issues]
capabilities = ["network"]
read_only = true
idempotent = true
parallel_safe = true
```

Then inspect, pin, validate, and enable it:

```console
pactrail mcp inspect issues
pactrail mcp snapshot issues
pactrail mcp check
pactrail mcp enable issues
```

The interactive session asks for each exact MCP request by default. Automation
fails closed unless the run explicitly uses `--mcp-approval allow-run`; complete
task-contract files must declare `mcp_invoke` and every underlying capability
in `permissions.allow` or `permissions.ask`. Disable a server instantly with
`pactrail mcp disable issues`; its snapshot is retained for inspection.

## Automation and CI

No-subcommand mode intentionally requires a terminal. Use subcommands in scripts:

```console
pactrail run "Fix the parser" --model qwen3-coder --output json
pactrail run "Fix the parser" --model qwen3-coder --output json --process-backend oci --sandbox-image pactrail-rust:local --process-approval allow-run
pactrail resume <RUN_ID> --output json
pactrail trace <RUN_ID> --json
pactrail inspect <RUN_ID> --json
pactrail diff <RUN_ID> --json
pactrail runs --json
pactrail apply <RUN_ID> --json
```

Repeatable work can use a complete versioned contract:

```console
pactrail task-template "Refactor the cache without changing behavior" > pactrail.task.toml
pactrail run --task pactrail.task.toml --model qwen3-coder --output json
```

Other discovery commands include `pactrail tools --json`, `pactrail schema`,
`pactrail compatibility --json`, the read-only `pactrail upgrade` preflight,
the explicit `pactrail migrate` state migration,
`pactrail mcp list --json`, `pactrail memory list`, `pactrail runs` (an alias
for `list`), `pactrail doctor`, and `pactrail completion <shell>`.

## Reproducible evaluation

The model-free [repository-scale suite](benchmarks/repository-scale/README.md)
separately gates cold/warm/incremental cache behavior, targeted context bytes,
tool descriptor count/weight/depth, release-mode latency, Linux peak RSS, and
fresh-run repository/context identity stability. Its schema-versioned raw
reports are CI artifacts; every release also ships a checksum-covered,
provenance-attested three-iteration soak report. Shared-runner timing is used as
a regression ceiling, not a marketing benchmark.

The public [Pactrail MVB v1](benchmarks/mvb-v1/README.md) runner performs seven
one-shot, model-in-the-loop tasks with exact artifact grading—no LLM judge and
no favorable-sample selection. Its first Windows ARM64 local baseline recorded:

- Qwopus3.5 9B Coder Q3_K_M: **6/7 strict task passes**;
- LFM2.5 230M Fable-5 F16: **1/7 strict task passes**;
- source isolation before apply: **14/14**;
- integrity-accepted portable traces: **14/14**.

Read the [methodology, environment, limitations, per-case results, model hashes,
and raw evidence](benchmarks/results/2026-07-17-windows-arm64/README.md). This is
an integration baseline, not a SWE-bench result or a claim of superiority over
another harness.

A separate [OpenRouter free-tier report](benchmarks/results/2026-07-17-openrouter-free/README.md)
records a quota-constrained Laguna M.1 run: **1/4 strict completions**, **4/4
exact isolated candidates**, **4/4 source isolation**, and **4/4
integrity-accepted traces**. It retains the strict failures and the Qwen
rate-limit diagnostic that exposed and led to a provider-backoff fix.

An earlier [DeepSeek V4 matched-harness evaluation](benchmarks/results/2026-07-18-deepseek-v4/README.md)
ran seven cases three times on both V4 Flash and V4 Pro: **Pactrail passed
42/42**, while OpenCode 1.2.27 passed **36/42** under the same no-shell policy.
Pactrail used **59.0% fewer reported model tokens**, preserved pre-apply source
isolation in **42/42** trials, and produced **42/42 integrity-accepted traces**.
The report retains every raw run and explicitly limits the claim to this matrix.

A harder [real-issue replay](benchmarks/results/2026-07-18-real-issue-replay/README.md)
uses pinned historical defects and hidden behavior tests. On its preregistered
three-task held-out set, Pactrail and OpenCode tied **1/3 on functional
correctness**; OpenCode led **1/3 to 0/3 on strict completion**. Pactrail used
**6.7% fewer reported tokens**, **55.7% less agent time**, and **67.1% less
estimated API cost**, with source isolation and verified traces in **3/3**
runs. This result is deliberately published despite not establishing broad
superiority: it identifies completion reliability as Pactrail's main remaining
gap and documents the fixes made after scoring without rerunning the tasks.

The newest [Pactrail v1 confirmation](benchmarks/results/2026-07-22-v1-confirmation/README.md)
is stricter and intentionally unflattering. Across 12 pass@1 trials on three
new real Rust defects and both DeepSeek V4 Flash and Pro, Pactrail passed
**0/6** functional trials while OpenCode passed **2/6**. Pactrail preserved
source isolation and verified trace integrity in **6/6** trials and used
**12.8% fewer reported tokens**, **65.0% less agent time**, and **65.0% lower
estimated API cost**, but its investigation loop never progressed to a
production edit. Every failure is retained. This result rejects a current
capability-superiority claim and records the loop behavior as a release
blocker.

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

The twelve crates keep the core domain, storage, memory, context, Git evidence,
models, MCP, tools, workspace transactions, engine, SDK, and CLI independently testable. See
[Architecture](docs/architecture.md), [Threat model](docs/threat-model.md),
[Interactive CLI](docs/interactive-cli.md), [Providers](docs/providers.md),
[Support matrix](docs/support.md), [Compatibility contracts](docs/compatibility.md),
[Upgrade guide](docs/upgrading.md), [Release runbook](docs/releasing.md), and
[Roadmap](docs/roadmap.md). The primary-paper mechanisms behind recent
architecture decisions are recorded in [Research foundations](docs/research-foundations.md).

## Durable local layout

Pactrail keeps its state in `WORKSPACE/.pactrail` by default:

```text
.pactrail/
├── events.sqlite3        # authoritative hash-linked event journal
├── memory.sqlite3        # provenance-aware workspace memory
├── artifacts/checkpoints # content-addressed resumable session state
├── artifacts/repository-index # content-addressed derived file analysis
└── runs/<run-id>/
    ├── run.json          # bounded, secret-free runtime manifest
    ├── execution.lock    # kernel-released exclusive local owner lock
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

CI runs those gates plus release builds on Linux, macOS, and Windows, applies
the dependency/license policy in `deny.toml`, and runs a hostile-repository
containment fixture against Docker. Generated-input property suites run inside
the normal test gate, while bounded libFuzzer jobs exercise paths, event replay,
and MCP schemas weekly. Start with
[CONTRIBUTING.md](CONTRIBUTING.md) and [Development](docs/development.md).

## Project status

Pactrail 1.0 is the stable public contract for the CLI, versioned JSON output,
durable state and migration behavior, transaction/apply guarantees, provider
and tool boundaries, and the `pactrail-sdk` facade. Stable does not mean
infallible: human UI presentation may evolve, model quality remains external,
and every security guarantee is limited by the documented threat model.

The release is supported for public use, evaluation, integrations, and
contributions on the Tier 1 platforms in the [support matrix](docs/support.md).
The restricted OCI backend has an adversarial Docker CI fixture; it is not
protection from a compromised runtime, daemon, kernel, desktop VM, or user
account. Native process execution remains explicitly trusted and unsandboxed.
The v1 maintainer audit is public; independent third-party review remains
welcome and is not implied by the version number.

## License

Licensed under either [Apache License 2.0](LICENSE-APACHE) or
[MIT](LICENSE-MIT), at your option.
