# V1 security audit closure

This document records Pactrail's maintainer security review for the stable v1
boundary. It is not an independent third-party audit and does not claim that
models, native processes, containers, MCP servers, providers, or the host are
intrinsically safe.

## Scope and threat model

The review traced untrusted repository, model, provider, MCP, process, durable
state, and terminal data through these security-sensitive sinks:

- workspace reads, candidate mutation, receipt construction, apply, rollback,
  and interruption recovery;
- native and restricted-OCI process creation, environment transfer, runtime and
  image identity, cancellation, and cleanup;
- OpenAI-compatible, Anthropic, Gemini, and MCP HTTP/stdio transports;
- event, memory, checkpoint, artifact, repository-cache, settings, and MCP
  persistence;
- approval and capability policy, portable traces, human/JSON rendering, CI,
  installers, and release provenance.

The attacker may control a checked-out repository, filenames and links, model
or provider responses, MCP protocol data, and process output. The configured
provider, explicitly approved native executables, local OCI runtime, host
kernel/filesystem, and current user account remain trusted computing base as
described in [the threat model](threat-model.md).

## Findings resolved

1. **[Medium] Durable-state directory links could redirect writes**
   - Confidence: High
   - Location: `crates/pactrail-cli/src/commands.rs:2580`,
     `crates/pactrail-store/src/artifact.rs:30`,
     `crates/pactrail-context/src/lib.rs:1373`
   - Attack path: repository-controlled `.pactrail`, `runs`, `artifacts`, MCP,
     artifact-prefix, or repository-cache directory link -> path APIs followed
     the link -> Pactrail created durable state below an unintended directory.
   - Impact: constrained writes outside the selected workspace/state root under
     fixed Pactrail names; artifact/cache data could also be redirected.
   - Fix: state roots and known children now require real local directories;
     artifact roots, digest prefixes, targets, and cache component chains reject
     links and special files before reads or writes.
   - Verification: negative unit tests cover root, child, digest-prefix, and
     repository-cache redirection; cross-platform code uses
     `symlink_metadata`, with Unix link creation exercised in CI.

2. **[Medium] MCP state reads followed file links**
   - Confidence: High
   - Location: `crates/pactrail-cli/src/mcp.rs:600`,
     `crates/pactrail-cli/src/mcp.rs:633`,
     `crates/pactrail-cli/src/mcp.rs:672`
   - Attack path: repository-controlled MCP manifest/snapshot link -> metadata
     and open followed the target -> external bytes entered MCP configuration or
     advisory context.
   - Impact: bounded local-file disclosure when an external target also formed
     valid Pactrail MCP state, plus unsafe state-update redirection.
   - Fix: MCP manifests, snapshots, lock files, backups, targets, and parent
     directories now reject symlinks and non-regular entries.
   - Verification: a linked-manifest regression test fails before parsing or
     connecting; normal init/read/update tests still pass.

3. **[Low] OCI environment values appeared in runtime process arguments**
   - Confidence: High
   - Location: `crates/pactrail-tools/src/process_backend.rs:531`
   - Attack path: approved process request environment -> `--env NAME=value`
     runtime argument -> value visible to local process inspection.
   - Impact: a value already supplied to the process tool could be exposed to
     other local processes or diagnostics.
   - Fix: values are written to a permission-restricted transient env file;
     runtime arguments contain only its path. The file is held for the child
     lifetime and removed on drop. Newline/NUL injection and aggregate overflow
     fail closed, and environment values never enter the runtime's own
     environment.
   - Verification: command-plan tests assert sentinel values are absent from all
     runtime arguments and line injection is rejected.

4. **[Low] Buffered OpenAI-compatible tool calls had weaker bounds than streams**
   - Confidence: High
   - Location: `crates/pactrail-models/src/openai_compatible.rs:783`,
     `crates/pactrail-models/src/openai_compatible.rs:852`
   - Attack path: malicious/defective provider response -> bounded HTTP body but
     unbounded per-field normalization -> oversized or control-bearing tool
     metadata reached engine state.
   - Impact: avoidable memory/durable-state pressure and a terminal-control
     primitive when combined with diagnostic logging.
   - Fix: buffered responses now require exactly one choice, bounded text, at
     most 128 calls, 512-byte control-free IDs/names, object arguments, and the
     same 1 MiB per-call argument ceiling as streaming.
   - Verification: regressions cover oversized arguments, excess calls,
     multiple choices, non-object arguments, and control-bearing identifiers.

5. **[Low] Explicit tracing fields could render untrusted controls**
   - Confidence: High
   - Location: `crates/pactrail-engine/src/engine.rs:3091`
   - Attack path: provider/tool/path error -> structured warning field -> tracing
     subscriber wrote the field directly to a terminal.
   - Impact: terminal display manipulation when verbose diagnostics were
     enabled.
   - Fix: every dynamic engine warning field is bounded and renders control
     characters as escaped text before it reaches tracing. Normal CLI error,
     human, JSON, timeline, and theme output retain their existing sanitizers.
   - Verification: the existing generated terminal-safety suite plus engine and
     CLI tests cover the rendering boundary.

No critical or high-severity finding remained open in the reviewed scope.

## Verification record

The closure is gated by formatting, warnings-as-errors Clippy, the complete
workspace test suite, documentation, release build, compatibility fixtures,
permission/storage failure injection, deterministic repository soak, workflow
syntax parsing, and the repository's cargo-deny advisory/license/source policy.
The release workflow additionally runs hostile-repository OCI containment and
attests every published artifact.

No credential-like production file was found among tracked project source. The
`secret.txt` files under benchmark result trees contain synthetic containment
markers, not credentials. Local `cargo-audit`/`cargo-deny` subcommands were not
installed during this review; the pinned CI cargo-deny action remains the
authoritative dependency-advisory gate.

## Residual risk

- Native execution is deliberately unsandboxed and must be treated as full host
  authority.
- Restricted OCI depends on the local runtime/daemon, image, kernel or desktop
  VM, and user account; it is not a VM-grade or formally verified sandbox.
- Same-user filesystem replacement races, sudden power loss beyond documented
  process-interruption guarantees, compromised providers/runtimes, and semantic
  correctness of generated code remain outside the defended boundary.
- Checksums fetched from the same GitHub release protect transport integrity;
  GitHub artifact attestations provide the separately verifiable provenance
  record.
- Independent external review remains welcome. Reports should use GitHub's
  private vulnerability-reporting channel described in `SECURITY.md`.
