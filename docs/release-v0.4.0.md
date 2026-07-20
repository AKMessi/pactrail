# Pactrail v0.4.0

Pactrail v0.4.0 adds an enforceable process boundary without weakening its
transaction model. Users can keep execution disabled, explicitly trust native
host processes, or run approved commands through a restricted local OCI
backend. Backend selection, policy evaluation, approval, execution, cleanup,
and evidence remain independently visible and durable.

## Highlights

- Restricted Docker and Podman execution pins a locally resolved immutable
  image and never pulls during a run. The command receives only the isolated
  candidate workspace, a read-only root, bounded private temporary storage, no
  network, no Linux capabilities, no ambient host environment, and explicit
  CPU, memory, PID, output, and wall-time ceilings.
- Runtime executables are resolved outside the workspace and fingerprinted.
  Selected daemon endpoints are probed and non-local transports are rejected
  without echoing endpoint credentials. Missing runtimes, images, or required
  controls fail before run state is created; restricted execution never
  silently falls back to native.
- Process requests cross an exact approval boundary bound to the run, command,
  arguments, environment-variable names, executable actor, backend identity,
  and profile digest. One-call, exact run-scoped, and deny decisions are
  retained in receipts.
- Policy evaluation and approval decisions are separate hash-linked events, so
  traces distinguish what policy required from what a user or automation chose.
- Ctrl-C now propagates through provider I/O, tools, processes, verification,
  repair, and UI. Native children are terminated and OCI containers are
  force-removed with bounded cleanup. Safe candidate changes remain reviewable.
- Settings schema 1 migrates atomically to explicit process modes. The old
  `--allow-process` and `/process on` forms remain deprecated aliases for
  trusted native execution.
- CI runs a pinned hostile-repository fixture that attempts host file access,
  root mutation, ambient-secret discovery, network egress, and daemon-socket
  access from the restricted container.

## Install

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/AKMessi/pactrail/main/install.ps1 | iex
```

Linux x86_64 or Apple Silicon macOS:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://raw.githubusercontent.com/AKMessi/pactrail/main/install.sh | sh
```

Both installers verify the selected archive against the release's
`SHA256SUMS`. GitHub also publishes build-provenance attestations for every
release artifact.

Source installation:

```console
cargo install --git https://github.com/AKMessi/pactrail.git --tag v0.4.0 --locked pactrail
```

## Upgrade and security notes

Process execution remains disabled by default. `/process sandbox <image>` uses
a prebuilt local image; Pactrail intentionally does not install packages or pull
images on an agent's behalf. The restricted backend is labelled
`oci_restricted`, not "fully sandboxed": Docker or Podman, its daemon, the host
kernel or desktop VM, and the user account remain trusted computing base.

`/process native` is still unsandboxed host execution. It should be used only
for trusted repositories even though its environment is scrubbed and its exact
authority is recorded.

Existing settings migrate atomically. Contracts, receipts, and events remain
backward-readable; newly recorded approval and backend data is omitted when
reading old artifacts rather than inventing authority.

See the complete user-visible history in
[`CHANGELOG.md`](https://github.com/AKMessi/pactrail/blob/v0.4.0/CHANGELOG.md),
the [threat model](https://github.com/AKMessi/pactrail/blob/v0.4.0/docs/threat-model.md),
and the [containment design](https://github.com/AKMessi/pactrail/blob/v0.4.0/docs/design/0001-process-containment.md).
