# Pactrail v0.7.0

Pactrail v0.7.0 opens the tool ecosystem without opening an authority bypass.
It adds governed MCP support and a Rust embedding facade while preserving the
same contract, approval, effect-fence, trace, transaction, and resume semantics
used by built-in tools.

## Highlights

- Stable MCP 2025-11-25 support over bounded stdio and Streamable HTTP.
- Explicit `mcp inspect` and `mcp snapshot` operations; normal agent runs never
  discover tools, start an unapproved server, or mutate their active catalog.
- Integrity-checked snapshots pin server/protocol identity, executable or
  endpoint identity, selected schemas, local profiles, and selected advisory
  resources/prompts.
- Per-server enable/disable, deterministic `mcp__server__tool` namespacing,
  strict JSON Schema and result bounds, shared run-local health, and stable JSON
  diagnostics.
- A dedicated request-scoped `mcp_invoke` capability keeps remote-tool approval
  independent from process approval. Each invocation also authorizes its exact
  process/network/secret/external-write effects before connecting.
- `pactrail-sdk` reexports the real model, tool, engine, MCP, transaction,
  persistence, memory, and context contracts for statically linked Rust hosts.

## Security boundary

Server descriptions, annotations, schemas, resources, prompts, and results are
untrusted data. Missing local tool profiles are not model-visible. Stdio starts
the exact configured executable without a shell and with an empty environment
except explicitly brokered names. Remote connections require HTTPS except for
opt-in literal loopback HTTP. Redirects, URL/query credentials, implicit retry,
session reinitialization, OAuth discovery, and automatic tool replay are
disabled.

A live call rechecks the executable/endpoint digest, initialization identity,
tool presence, and input/output schema before sending the single tool request.
Identity drift marks the shared health state stale. Every successful call
retains server, transport, snapshot, schema, and health-transition effects in
the durable action record.

## Operator workflow

```console
pactrail mcp init
# Add a disabled server and local tool profiles to .pactrail/mcp.toml.
pactrail mcp inspect <server>
pactrail mcp snapshot <server>
pactrail mcp check
pactrail mcp enable <server>
```

Interactive runs prompt for exact MCP approvals. Non-interactive runs deny by
default and require `--mcp-approval allow-run`. Complete task contracts must
declare `mcp_invoke` and every underlying capability explicitly.

## Scope and compatibility

This release intentionally does not include legacy MCP SSE, OAuth discovery,
server-requested sampling, dynamic native plugin loading, or OCI-contained MCP
subprocesses. Rust extensions are statically linked; `pactrail-sdk` remains a
pre-1.0 Git dependency until the v1 SemVer support window begins. Existing 0.6
runs without MCP preserve their previous runtime-identity calculation and remain
resumable.

The release gate runs formatting, warning-free Clippy, full workspace tests,
documentation, locked dependency policy, cross-platform CI, real fragmented
loopback MCP fixtures, hostile schema/description/size cases, and a compile-time
custom-provider/custom-tool compatibility fixture. Tests require no public
server, provider key, or local model.

Install from source:

```console
cargo install --git https://github.com/AKMessi/pactrail.git --tag v0.7.0 --locked pactrail
```

See the [MCP design](design/0004-mcp-ecosystem.md), [embedding guide](embedding.md),
[threat model](threat-model.md), and [changelog](../CHANGELOG.md).
