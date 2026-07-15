# Development guide

## Prerequisites

- Rust 1.95 (pinned by `rust-toolchain.toml`).
- Git for normal repository fixtures.
- No provider key or local model is required for the test suite.

SQLite is bundled through `rusqlite`, so contributors do not need a system
SQLite development package.

## Quality gate

Run the same checks as CI from the repository root:

```console
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo doc --workspace --all-features --no-deps --locked
cargo build --workspace --release --locked
```

Dependency and license policy is defined in `deny.toml` and run in CI with
`cargo-deny`.

The workspace forbids unsafe Rust, `unwrap`, `expect`, `panic`, direct stdout or
stderr printing, and warning-bearing builds. Tests use explicit unreachable
messages only after fixture construction where failure cannot be recovered.

## Architecture rules

- Keep probabilistic/provider behavior outside `pactrail-core`,
  `pactrail-workspace`, and policy enforcement.
- A new mutation path must use workspace-safe paths, declare a capability,
  produce observed effects, and preserve the apply boundary.
- A new durable format needs a schema version, fail-closed future-version test,
  crash/atomicity behavior, and migration policy.
- A new tool needs bounded typed input/output, annotations, policy enforcement,
  model-safe errors, trace metadata, and negative tests.
- Never persist raw credentials. New telemetry must document and test its data
  classification.
- Do not claim verification from model text. Evidence is created by a trusted
  kernel subsystem.

## Testing layers

- Crate unit tests cover reducers, path edge cases, schemas, bounds, parsers,
  transaction recovery, memory provenance, tool behavior, and provider fixtures.
- Engine tests use scripted `ModelDriver` implementations and real tool/workspace
  code. The parallel scheduler is tested with a two-party async barrier.
- CLI integration tests launch the real binary against a local mock HTTP
  provider, verify source isolation, inspect portable traces, and apply from a
  second process.
- CI repeats the complete suite on current Linux, macOS, and Windows runners.

Tests must not contact public providers or rely on a locally installed model.

## Manual local-provider smoke test

Start an OpenAI-compatible server on loopback, then use a disposable workspace:

```console
cargo new pactrail-smoke --lib
cd pactrail-smoke
pactrail
```

Inside the session:

```text
/connect http://127.0.0.1:8080/v1 model-id
/context 4096
/output-tokens 512
/turns 8
/process off
Create SMOKE_TEST.md containing exactly: Pactrail local model test passed.
/trace
/diff
/apply
```

Confirm that no source file exists before apply, the diff contains only the
requested change, the trace hash chain verifies, and the applied file matches
the receipt.

## Commit and pull-request discipline

Keep commits focused and reviewable. Use imperative conventional-style subjects
where practical, add DCO sign-off (`git commit -s`), and update the changelog for
user-visible changes. A pull request should explain the invariant it changes,
how failure behaves, and how it was verified—not only list edited files.

Do not commit `.pactrail`, provider keys, model files, private repositories,
terminal transcripts containing secrets, or generated target artifacts.
