# Governance

Pactrail is currently a maintainer-led open-source project.

## Roles

- **Contributors** participate through issues, discussions, documentation,
  tests, reviews, and code.
- **Reviewers** are trusted contributors who consistently provide accurate,
  constructive review in an area of the project.
- **Maintainers** can merge changes, manage releases and security reports, and
  make compatibility or governance decisions.

Roles are earned through sustained public contribution and sound judgment,
especially around security boundaries and user data. They are not purchased or
granted automatically by employment or sponsorship.

## Decisions

Routine changes use pull-request review. Material changes to contracts, durable
formats, sandboxing, provider/tool APIs, or governance should begin with a public
design issue and seek rough consensus. Maintainers make the final decision when
consensus is not possible, documenting the rationale and meaningful objections.

Security-sensitive details may be discussed privately until coordinated
disclosure is safe. No governance rule requires publishing an active exploit.

## Releases

Maintainers create version tags and GitHub releases after CI passes. Release
artifacts receive checksums and GitHub build-provenance attestations. User-visible
changes are recorded in `CHANGELOG.md`; breaking changes during `0.x` must be
called out explicitly.

## Conflicts of interest

Reviewers should disclose material personal or commercial conflicts and avoid
being the sole approver of affected decisions. Project safety, accurate claims,
and user control take priority over provider or sponsor preferences.

## Changes to governance

Governance changes follow the same public design and review process as material
architecture changes.
