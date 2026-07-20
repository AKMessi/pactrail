# Design 0004: governed MCP and embedding ecosystem

Status: accepted for Pactrail 0.7

## Problem

Pactrail needs an open tool ecosystem without turning an MCP server, plugin, or
repository-controlled configuration into a privileged side channel. Remote tool
descriptions, schemas, annotations, results, resources, and prompts are untrusted
inputs. A local MCP command is also arbitrary host code, while an HTTP MCP server
introduces network, credential, redirect, and external-side-effect authority.

The integration must preserve Pactrail's existing invariant: no model-selected
operation crosses a trust boundary unless its exact descriptor, policy decision,
approval, effect fence, result bound, cancellation behavior, and trace are known.

## Decision

### Adapter boundary

A new `pactrail-mcp` crate owns the MCP protocol adapter. It uses the official
Rust SDK internally but exposes only Pactrail-owned manifest, snapshot, health,
and registration types. This keeps the public embedding API independent from
the SDK's release cadence.

Pactrail 0.7 supports the stable MCP 2025-11-25 initialize lifecycle over:

- stdio, for explicitly trusted local commands; and
- Streamable HTTP, for explicit HTTPS endpoints and opt-in loopback HTTP.

There is no legacy SSE transport, implicit transport fallback, OAuth discovery,
dynamic client registration, or server-requested sampling in 0.7. HTTP redirects
are disabled. Each additional protocol surface requires a separate threat review.

### Explicit configuration and immutable discovery

Workspace MCP configuration lives in `.pactrail/mcp.toml`. It is never loaded
from a parent directory and never starts a process or contacts a network endpoint
on its own. A server entry declares:

- a unique stable name and enabled state;
- one exact command plus arguments, or one canonical URL;
- an allowlist of environment-variable names whose values may be brokered at
  execution time (values are never persisted);
- request, startup, output, and catalog limits;
- the capabilities locally assigned to each exposed tool; and
- explicit resource URIs and prompt invocations eligible for context snapshots.

`pactrail mcp snapshot` is an explicit administrative operation. It displays the
authority being exercised, connects once, bounds and validates the complete
catalog, and writes `.pactrail/mcp/<server>.snapshot.json`. The snapshot contains
the negotiated protocol revision, server implementation identity, canonical
transport digest, namespaced descriptors, local capability declarations,
selected context, and a digest over the whole document. No secret value is
stored. Atomic replacement prevents a partial snapshot from becoming runnable.

A normal run does not perform discovery. It registers tools only from an
integrity-checked snapshot whose transport digest still matches the manifest.
On first tool use, the adapter lazily connects after the tool has crossed the
same policy and effect-fence boundary as a built-in. The live server identity is
checked against the snapshot before the call is sent. Catalog-change
notifications never mutate the active tool set; they mark the server stale and
require a new explicit snapshot.

### Authority model

MCP annotations and descriptions are hints, never grants. Every remote tool has
a local policy profile in the manifest. A dedicated `mcp_invoke` capability
keeps MCP authority independent from broad native-process authority. Its
descriptor uses the strongest declared effect, and the adapter additionally
authorizes every declared capability before connection or invocation:

- stdio requires `process_spawn` plus the declared semantic effects;
- HTTP requires `network` for the exact canonical origin plus the declared
  semantic effects;
- named credential forwarding additionally requires `secret_use`; and
- any tool capable of modifying a remote service requires `external_write`.

`mcp_invoke` remains request-scoped even under `--mcp-approval allow-run`, so
every grant is bound to the run, snapshot, public tool, argument digest,
transport identity, and profile digest. Process and MCP approval modes are
routed independently. A complete task contract must declare `mcp_invoke` and
every underlying effect explicitly.

If a profile is absent, the tool is not registered. Pactrail never infers
read-only or idempotent behavior from a server annotation. Parallel safety is
off unless the local profile opts in and the tool is locally declared read-only
and idempotent.

For stdio, the child receives an empty environment plus the minimum platform
variables needed to start the exact allowlisted executable and the explicitly
brokered names. Shell interpretation is never used. The full command and all
arguments are shown before snapshotting. Native stdio servers are labelled host
execution; OCI-contained MCP is a later additive transport.

For HTTP, the manifest URL must be canonical. Production endpoints require
HTTPS. Plain HTTP is limited to literal loopback hosts when explicitly enabled.
User information, query parameters, fragments, redirects, implicit endpoint
discovery, and server-selected authorization endpoints are rejected. Bearer
credentials may come only from a named environment variable and are redacted
from errors and traces.

### Names, schemas, and results

External tools are exposed as `mcp__<server>__<tool>`. Both components use a
bounded ASCII identifier grammar. Normalization collisions, duplicate source
names, built-in collisions, and names over the model/provider limit fail the
whole snapshot deterministically.

Input schemas must be JSON Schema objects with an object root, bounded depth,
node count, property count, string size, and serialized size. Unsupported or
malformed schemas fail snapshot creation; they are not weakened to an untyped
object. The canonical schema digest is rechecked at execution time. Server
descriptions are control-character-free, byte-bounded, and prefixed as untrusted
server-provided text.

Arguments must be JSON objects and are validated by Pactrail before sending.
Results are collected under a strict byte and item budget. Text and structured
content are retained; binary, audio, image, embedded-resource, and link content
are represented by bounded metadata rather than silently injected into model
context. Oversized or malformed results fail closed with a legible diagnostic.
The engine's independent model-facing result ceiling remains a second bound.

### Cancellation, timeout, health, and recovery

Every initialization, discovery, call, resource read, and prompt read has a
deadline. Cancellation drops the active request and closes the session; stdio
children are terminated and reaped. A disconnected or timed-out server produces
one failed tool result and a health transition. Pactrail does not automatically
retry a tool call because its remote effect may be uncertain.

Health is run-local and diagnostic: `disabled`, `snapshot_missing`, `ready`,
`connecting`, `healthy`, `stale`, or `failed`. It never weakens policy or changes
the registered descriptor set. Durable action traces include the server name,
transport kind, snapshot digest, schema digest, duration, bounded output size,
truncation state, and health transition, but never endpoint credentials,
environment values, raw authorization headers, or unbounded server errors.

MCP effects use the existing write-ahead/completed effect fence. If Pactrail
dies after dispatch but before a completion record, resume refuses to replay the
call as an uncertain effect.

### Resources and prompts

Resources and prompts are never automatic system instructions. Only explicitly
selected items captured by `pactrail mcp snapshot` may become supplemental
`ContextFragment` values. Each fragment is labelled with server, primitive,
identifier, snapshot digest, and capture time. Content remains advisory and
cannot override the task contract, repository instructions, policy, or tool
contracts. Binary content is excluded in 0.7.

### Rust embedding API

A new `pactrail-sdk` facade provides the supported pre-1.0 integration surface:

- `ModelDriver`, model request/response IR, and stream observer types;
- `Tool`, descriptor, annotations, bounded output, context, and registry types;
- a builder that combines built-ins with out-of-tree tools deterministically;
- a versioned integration manifest and compatibility constants; and
- examples for one provider and one typed tool.

The facade re-exports owned Pactrail contracts, not CLI internals or storage
implementations. Compatibility tests compile the examples and deserialize fixed
manifests. Until 1.0, breaking API changes require a minor version, migration
notes, and fixtures for the previous minor.

## Alternatives rejected

- **Discover on every run.** This makes prompt construction execute code or use
  the network and lets catalog changes silently alter authority.
- **Trust MCP annotations.** The protocol defines them as hints, so they cannot
  safely determine capability, approval, idempotence, or concurrency.
- **Pass through raw MCP types.** This couples Pactrail's public API and durable
  formats to an independently evolving SDK.
- **Retry disconnected calls.** A remote mutation may have completed even when
  its response was lost.
- **Treat resources or prompts as system messages.** That would let a server
  rewrite the agent's authority and defeat provenance ordering.

## Test and release criteria

0.7 is complete only when automated tests cover:

- deterministic namespacing and all collision classes;
- malformed, recursive, deeply nested, and oversized schemas;
- poisoned descriptions and control characters;
- manifest traversal, duplicate names, invalid commands/URLs, secret redaction,
  and transport-digest drift;
- missing capability profiles and denied secondary capabilities;
- fragmented stdio and HTTP responses, disconnects, startup/call timeouts,
  cancellation, child cleanup, and no automatic replay;
- oversized/mixed-content results and exact output bounds;
- server identity/schema drift and list-change staleness;
- explicitly selected resource/prompt provenance and absence of implicit
  authority; and
- a legible human CLI plus stable JSON diagnostics.

The dependency policy, cross-platform suite, documentation, and hostile fixtures
must pass before the milestone branch is merged.
