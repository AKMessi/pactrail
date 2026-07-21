# Upgrade guide

Pactrail separates binary installation, read-only readiness checks, and durable
state migration. An upgrade never silently rewrites a source workspace.

## Preflight

Run this before and after installing a newer binary:

```console
pactrail upgrade
pactrail upgrade --json
```

`upgrade` is strictly read-only. It runs the same schema and integrity audit as
`pactrail migrate`, validates event chains, receipts, checkpoints, memory,
settings, transactions, and offline MCP snapshots, then reports:

- whether local state is readable by the current binary;
- how many known migrations are pending;
- every active deprecated CLI surface, its exact replacement, and planned
  removal version; and
- ordered next steps.

The JSON report has its own schema number and always includes
`changes_applied: false`. Automation should fail the upgrade when the command
returns non-zero, not infer readiness from partial output.

## Apply known state migrations

Review the preflight, stop active Pactrail runs, then apply only the supported
local migrations:

```console
pactrail migrate --apply
pactrail upgrade
```

Apply mode completes the entire compatibility preflight before changing any
component, refuses active run locks, migrates SQLite inside database
transactions, persists settings through its backup/rename recovery path, and
revalidates the result. Unknown versions or integrity failures stop before a
known component is migrated.

Do not open migrated state with an older binary unless that release explicitly
declares the resulting schemas readable. Source files and unapplied candidate
workspaces are not rewritten by state migration.

## Deprecated process aliases

The following aliases remain supported throughout Pactrail 1.x and are planned
for removal in 2.0:

| Deprecated | Replacement |
|---|---|
| `pactrail run --allow-process` | `--process-backend native --process-approval allow-run` |
| `/process on` | `/process native` |

The explicit forms distinguish the host-execution trust boundary from approval
authority. Both deprecated forms emit a warning when used; `pactrail upgrade
--json` is the stable inventory for tooling.

## Binary installation

The checksum-verifying installers replace only the Pactrail executable. They do
not run state migrations:

```powershell
# Windows
irm https://raw.githubusercontent.com/AKMessi/pactrail/main/install.ps1 | iex
```

```sh
# Linux or macOS
curl --proto '=https' --tlsv1.2 -LsSf \
  https://raw.githubusercontent.com/AKMessi/pactrail/main/install.sh | sh
```

For reproducible deployments, set an explicit release version as documented in
the installer help, verify `pactrail --version`, and retain the release checksum
and provenance attestation with your deployment record.
