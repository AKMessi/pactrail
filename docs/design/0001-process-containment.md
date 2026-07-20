# Design 0001: process containment, approvals, and cancellation

Status: accepted for implementation on `feat/v04-containment`  
Target: v0.4

## Problem

Pactrail isolates source edits in a transaction, but its current process tool
executes directly on the host. A process grant therefore implies the child can
attempt to read the host filesystem, use the host network, discover ambient
credentials, and mutate external services. Clearing most environment variables
does not create an operating-system security boundary.

The v0.4 boundary must let trusted users retain native execution while making
restricted execution explicit, enforceable, observable, cancellable, and
impossible to downgrade silently.

## Threat model

The repository, model, command, arguments, process output, container image, and
tool descriptions are untrusted data. The configured container runtime and host
kernel are trusted computing base. A compromised runtime, daemon, kernel, or
user account is outside this boundary.

Protected assets are host files outside the candidate, ambient credentials,
host network access, external services, the source workspace, the durable event
journal, and availability of the host.

The primary attack paths are path or mount injection, execution through a
workspace-shadowed runtime binary, mutable image substitution, network egress,
privilege gain, resource exhaustion, orphaned containers, terminal-control
injection, and a requested restricted backend falling back to native execution.

## Runtime contract

Process execution has three explicit modes:

- `disabled`: no process can start;
- `native_trusted`: direct host execution with its full effective authority
  recorded in the task contract;
- `oci_restricted`: execution through a locally available Docker- or
  Podman-compatible CLI with a policy-derived container invocation.

There is no `auto` mode that can select native execution. Runtime discovery may
choose Docker or Podman only after `oci_restricted` has already been selected.
If no conforming runtime or local image is available, the request fails closed.

Every backend exposes a stable descriptor containing its kind, sandbox strength,
runtime identity, resolved image identity, filesystem policy, network policy,
environment policy, and resource ceilings. This descriptor is recorded in
status output, action traces, verification evidence, and receipts.

## Restricted OCI profile

The initial backend runs one short-lived container per command with:

- only the isolated candidate mounted read-write at `/workspace`;
- the image root filesystem read-only;
- an explicit writable `tmpfs` with a size ceiling;
- working directory `/workspace`;
- network namespace disabled by default;
- all Linux capabilities dropped;
- `no-new-privileges` enabled;
- bounded memory, CPU, PID count, output, and wall time;
- no host devices, daemon socket, host namespaces, or additional mounts;
- no implicit host environment forwarding;
- an immutable locally resolved image identity and `--pull=never`;
- automatic container removal plus explicit forced cleanup after cancellation,
  timeout, client failure, or abnormal exit.

The backend must pass arguments directly and must never interpolate a shell.
Mount sources are generated only from Pactrail's canonical transaction root.
Image and runtime values come from validated configuration, never model input.

Docker documents `--network none`, bind mounts, read-only root filesystems,
capability controls, PID limits, and memory/CPU constraints in its
[container run documentation](https://docs.docker.com/reference/cli/docker/container/run/)
and [resource constraints](https://docs.docker.com/engine/containers/resource_constraints/).
Podman documents the corresponding controls in
[`podman-run`](https://docs.podman.io/en/latest/markdown/podman-run.1.html).

The strength label is `oci_restricted`, not `fully_sandboxed`: containers share
the host kernel or a desktop VM boundary, and enforcement depends on runtime and
host configuration. `pactrail doctor` reports detected enforcement support and
never upgrades the label based only on runtime presence.

## Approval model

An approval is a versioned durable object, not a boolean. It binds:

- capability;
- canonical resource selector;
- actor or executable fingerprint;
- backend identity and sandbox profile digest;
- decision (`allow_once`, `allow_run`, or `deny`);
- creation time and optional expiry;
- run identity when run-scoped.

Denials in the task contract always win. Non-interactive execution denies an
unresolved approval. Interactive execution presents the exact command,
workspace, backend, network policy, environment names, resource limits, and
expiry before accepting a decision. Secrets are named but never displayed.

Policy evaluation, the approval decision, and the resulting effect remain
separate hash-linked events. An approval cannot authorize a broader capability
than the task contract permits and cannot be reused after its bound scope or
profile digest changes.

## Cancellation and cleanup

One run-scoped cancellation token propagates through model transport, tool
execution, process backends, verification, repair, and the CLI. Cancellation is
a first-class lifecycle result rather than an arbitrary engine error.

On cancellation Pactrail must:

1. stop accepting new model or tool work;
2. terminate the active child process or force-remove the active container;
3. drain or abort bounded output readers;
4. record the cancellation request and completed cleanup;
5. retain coherent candidate changes for review when safe;
6. leave the event chain replayable and the transaction recoverable.

Dropping a future remains a final safety net, not the primary cleanup protocol.

## Compatibility

The existing `--allow-process` flag remains temporarily supported as an explicit
alias for `native_trusted` and emits a deprecation warning. Existing settings
schema 1 migrates `allow_process = false` to `disabled` and `true` to
`native_trusted`; migration is atomic and never chooses `oci_restricted`
implicitly.

Task contracts, events, receipts, and settings receive explicit schema changes
only where the new durable information is required. Readers either migrate a
known older version or fail closed on an unknown version.

## Required tests

- Argument vectors cannot become shell syntax.
- Runtime binaries inside the candidate are rejected.
- Image references are resolved locally and run with `--pull=never`.
- The only bind source is the canonical candidate root.
- Network, capabilities, root filesystem, PID, CPU, memory, and temporary-space
  restrictions are present in the generated invocation.
- Missing runtimes, missing images, unsupported controls, and cleanup failures
  fail closed with actionable diagnostics.
- Timeout and cancellation forcibly remove the container and preserve a valid
  event lifecycle.
- Contract denial overrides every stored or interactive approval.
- Approval reuse fails after resource, actor, backend, or profile changes.
- Settings migration preserves the old native-trust meaning exactly.
- Hostile fixtures attempt host reads/writes, network egress, environment-secret
  reads, process bombs, symlink escapes, and daemon-socket access.
- Linux, macOS, and Windows retain deterministic behavior when no runtime is
  installed; runtime-backed fixtures run where CI provides a supported daemon.

## Deliberately excluded from v0.4

Remote container daemons, Kubernetes executors, privileged containers, arbitrary
host mounts, host networking, device forwarding, automatic image pulls, and
model-selected images are not supported. Package installation belongs in a
prebuilt, pinned sandbox image rather than an agent command with network access.
