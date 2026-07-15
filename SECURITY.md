# Security policy

## Reporting a vulnerability

Do not open a public issue for a suspected vulnerability. Use GitHub's private
vulnerability reporting for this repository and include:

- affected commit/version and platform;
- required configuration and capability grants;
- minimal reproduction steps or a private test fixture;
- impact on workspace, host, credentials, integrity, or remote systems;
- any known workaround or proposed mitigation.

Never include real credentials, private source, or third-party personal data.
Maintainers will acknowledge a complete report within five business days and
coordinate validation, remediation, release, and disclosure with the reporter.

## Supported versions

Before the first stable release, security fixes target the latest `main` and the
latest GitHub release, when one exists. Older developer-preview commits are not
maintained as separate security branches.

## Security model

Pactrail treats model output, repository content, memory, provider/tool output,
and terminal text as untrusted. Its workspace transaction protects the normal
review/apply path; it does **not** sandbox native child processes. Process
execution is disabled by default and should remain disabled for untrusted code
until a production sandbox backend ships.

See [docs/threat-model.md](docs/threat-model.md) for defended properties,
limitations, and out-of-scope risks.
