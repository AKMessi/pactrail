# Pactrail v0.5.0

Pactrail v0.5.0 makes an interrupted coding run a recoverable, integrity-bound
transaction instead of a prompt the user has to start again.

## Highlights

- `pactrail resume <RUN_ID>` and interactive `/resume [run]` reopen the same
  isolated candidate, provider-neutral conversation, controller state, and
  cumulative budgets.
- Content-addressed checkpoints are bound to the hash-linked event head,
  contract, candidate, repository context, model/tools, and exact secret-free
  runtime/process profile. Drift fails before another event or model request.
- Every model tool call now has visible `effect_prepared` and
  `effect_completed` fences. An incomplete effect is named and never replayed
  automatically.
- Kernel file locking plus SQLite lease metadata rejects a second live owner but
  releases immediately after an abrupt process death.
- A real-binary test kills Pactrail during provider I/O, proves concurrent
  resume is denied, restarts the same run, and verifies its single contract and
  intact event chain.

## Recovery boundary

Automatic resume is intentionally narrower than “try again.” It continues from
pre-model and pre-verification checkpoints only. A pre-tool checkpoint is kept
for diagnosis, while any prepared tool effect or other post-checkpoint event
requires inspection. This prevents duplicate file mutations and host/external
process effects when the system cannot prove what completed.

Use:

```console
pactrail list
pactrail trace <RUN_ID>
pactrail resume <RUN_ID>
```

Inside the interactive CLI:

```text
/runs
/trace <run-prefix>
/resume <run-prefix>
```

## Upgrade notes

- The event database migrates from schema 1 to schema 2 to add local run-lease
  metadata. Existing events remain unchanged and readable.
- New event envelopes use schema 2 for effect-fence events. Existing schema 1
  event chains remain hash-verifiable, inspectable, and projectable.
- New runs contain `run.json` (bounded configuration without key values) and
  `execution.lock` under their run directory, plus checkpoint artifacts under
  `.pactrail/artifacts/checkpoints`.
- Runs created before v0.5 remain inspectable and applyable but have no session
  checkpoint and therefore cannot enter the model loop through `resume`.
- API keys remain environment-only. The manifest records only the variable
  name; changing any manifest bytes invalidates the checkpointed runtime
  identity.

## Install

Use the checksum-verifying installers from the README, or install this tag with
Rust 1.95 or newer:

```console
cargo install --git https://github.com/AKMessi/pactrail.git --tag v0.5.0 --locked pactrail
```

See the [changelog](../CHANGELOG.md), [architecture](architecture.md), and
[durable-resume design](design/0002-durable-resume.md) for the complete
contract and failure semantics.
