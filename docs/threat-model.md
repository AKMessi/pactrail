# Threat model

This document describes Pactrail 0.1 as shipped. It is not a claim that model
execution or native processes are intrinsically safe.

## Assets and trust boundaries

Pactrail protects the source workspace, durable run state, credentials handled
by the provider adapter, and the integrity of review/apply decisions.

The following inputs are untrusted:

- model text, tool names, IDs, and arguments;
- repository source, filenames, generated files, and instruction-like content;
- historical memory content at retrieval time;
- provider, tool, process, plugin, and future MCP output;
- terminal control sequences;
- concurrent changes in the source workspace;
- portable trace, receipt, or candidate files after external modification.

The task contract and correctly scoped `AGENTS.md` files are model instructions,
but neither can override capability policy, safe-path resolution, or receipt
validation.

## Defended properties

### Filesystem and apply

- Model tools see a virtual `.` root rather than a host path.
- Safe-path parsing rejects absolute paths, drive/UNC prefixes, parent traversal,
  symlinks, and special files.
- Writes additionally require an allowed workspace-relative prefix.
- The source tree is untouched before explicit apply unless a trusted native
  process escapes the candidate directory.
- Apply binds the receipt to the exact candidate change set and refuses a source
  path whose baseline bytes or mode changed concurrently.
- Landing uses a synchronized backup journal, rollback, and idempotent recovery.

### Durable integrity

- Event envelopes are sequence-checked and BLAKE3 hash-linked.
- Receipts bind contract, evidence, baseline, resulting digest, changes, risks,
  and final event hash.
- Transaction, settings, event, memory, contract, and receipt schemas reject
  unknown future versions.
- Portable trace JSONL is regenerated atomically from the verified SQLite event
  chain and is not treated as authoritative input.

### Model and tool boundary

- Tool inputs use JSON Schema contracts and deserialize into bounded Rust types.
- Every call is capability-gated; denial wins and runtime overgrant fails before
  execution.
- Tool and process results have retained/model-visible output ceilings.
- Deterministic verification runs in a disposable candidate snapshot, keeping
  ordinary build and test artifacts out of the receipt-bound candidate tree.
- Parallel scheduling is limited to tools explicitly annotated read-only and
  parallel-safe. Mutations and host execution remain serial.
- Repeated call IDs, impossible finish reasons, token/wall-time overruns, and
  persistent non-progress loops fail explicitly.

### Memory and prompt injection

- Only the user-facing CLI and integrity-checked applied receipts can write
  memory; the model has recall-only access.
- Memory carries source, kind, ID, timestamps, tags, and receipt provenance.
- Retrieved memory is labelled advisory and cannot replace current-file checks.
- Root instructions must fit the model-derived context budget in full. Nested
  instructions carry explicit directory scope and optional entries are never
  cut mid-document.

These controls reduce durable prompt poisoning. They cannot determine whether a
human-authored convention or a previously applied change contains bad advice;
the model and user must still compare memory with current code.

### Credentials, network, and terminal output

- Provider keys are accepted by environment-variable name, never as CLI values,
  endpoint credentials, or persisted secret values.
- Remote endpoints require HTTPS; plain HTTP is accepted only for exact loopback
  hosts. Redirects are disabled.
- Provider response bodies are bounded and malformed protocol data fails closed.
- Human and JSON output neutralize terminal control characters from untrusted
  fields.
- Traces store argument digests and bounded metadata, not raw prompts, raw tool
  arguments, API keys, or full provider responses.

Operational logs and provider error messages can still contain data produced by
third-party libraries or endpoints. Treat debug logs as potentially sensitive.

## Critical limitation: native processes

`--allow-process` or `/process on` is an explicit trust decision. A child starts
in either the candidate workspace or a disposable verification snapshot with a
scrubbed/rebuilt operational environment, but Pactrail 0.1 does not provide an
OS or container sandbox. The child can attempt to read other host files, find
secrets, use the network, modify the source tree directly, or affect external
services.

The transaction protects normal tool-based landing; it cannot contain hostile
native code. Pactrail therefore records process, network, secret-use, and
external-write authority together, and rejects task files that understate it.
Keep processes disabled for untrusted repositories. `pactrail doctor` reports
available runtimes but does not imply they are active sandboxes.

## Out of scope in 0.1

- containment of arbitrary native code without an external sandbox backend;
- protection from a compromised user account, kernel, filesystem, or provider;
- cryptographic identity/non-repudiation for local receipts;
- automatic secret brokering or least-privilege remote credentials;
- remote side effects such as pull requests, messages, or deployments;
- semantic proof that a model-produced change is correct;
- confidentiality of prompts sent to the configured provider.

## Reporting

Do not publish suspected vulnerabilities in a normal issue. Follow
[SECURITY.md](../SECURITY.md) and use GitHub private vulnerability reporting.
