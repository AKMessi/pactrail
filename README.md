# Pactrail

**Every change carries its evidence.**

Pactrail is a verification-native coding agent harness written in Rust. It turns a software task into an explicit contract, works in an isolated transaction, and produces an evidence-backed change receipt before touching your working tree.

Pactrail is under active development. The first usable vertical slice is being built in public; interfaces may change before the 1.0 release.

## Principles

- Model-neutral: API and local models use the same capability-driven interface.
- Transactional: agent writes happen outside the user's working tree until explicitly applied.
- Evidence-first: tests, diagnostics, observations, and model opinions are graded honestly.
- Durable: runs are event-sourced, resumable, inspectable, and forkable.
- Local-first: no account, hosted service, or telemetry is required.
- Secure by default: network, secrets, and external side effects require scoped policy decisions.

Remote providers must use HTTPS. Pactrail uses the operating system TLS backend so
production trust policy and certificate management remain under host control.

## Building

```console
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

The minimum supported Rust version is 1.95.

## License

Licensed under either of Apache License 2.0 or MIT at your option.
