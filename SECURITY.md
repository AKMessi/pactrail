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

Security fixes target the latest stable 1.x minor and current `main`. For 90
days after a newer 1.x minor is released, the previous minor remains eligible
for critical/high fixes. The 0.x developer-preview line is unsupported after
v1.0.0.

Maintainers target initial triage within ten business days of acknowledging a
complete report. After validation, the target remediation window is 7 calendar
days for critical severity and 30 calendar days for high severity. Medium and
low findings normally ship in the next planned release. Complex fixes or
coordinated disclosures may require a different private schedule; the reporter
will be told when that happens.

## Security model

Pactrail treats model output, repository content, memory, provider/tool output,
and terminal text as untrusted. Its workspace transaction protects the normal
review/apply path; it does **not** sandbox native child processes. Process
execution is disabled by default. For untrusted code, keep native execution
disabled and use the restricted OCI backend only after reviewing its documented
runtime, daemon, image, kernel, and host-account trust assumptions.

See [docs/threat-model.md](docs/threat-model.md) for defended properties,
limitations, and out-of-scope risks. The maintainer review and resolved findings
for the stable boundary are recorded in
[docs/security-audit-v1.md](docs/security-audit-v1.md).
