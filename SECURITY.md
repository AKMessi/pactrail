# Security Policy

## Reporting a vulnerability

Do not open a public issue for a suspected vulnerability. Use GitHub's private vulnerability reporting for this repository and include affected versions, reproduction steps, impact, and any proposed mitigation.

Maintainers will acknowledge a complete report within five business days. Disclosure timing will be coordinated with the reporter after a fix is available.

## Security model

Pactrail treats model output, repository contents, tool descriptions, tool output, and plugin output as untrusted. A workspace transaction protects the user's working tree; a configured sandbox protects the host. These are separate guarantees, and Pactrail reports the effective sandbox strength instead of implying unavailable isolation.

Never include real credentials or private source code in a vulnerability reproduction.

