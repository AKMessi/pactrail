# Pactrail fuzzing

These `cargo-fuzz` targets exercise the smallest, highest-risk parsers and
state boundaries with libFuzzer:

- `workspace_path`: containment and portable path normalization;
- `event_envelope`: event JSON, hash verification, and deterministic replay;
- `mcp_schema`: untrusted MCP input/output schemas and arguments.

Install `cargo-fuzz` and run a target from the repository root:

```console
cargo install cargo-fuzz --locked
cargo fuzz run workspace_path
cargo fuzz run event_envelope
cargo fuzz run mcp_schema
```

Crash artifacts and local corpora stay below `fuzz/` and are ignored. Minimize
and commit a regression test before fixing any discovered defect.
