# V2 launch candidate

## Candidate scope

The candidate is developed on `feat/v2-frictionless-setup`. All twelve workspace
packages and internal dependency requirements are 2.0.0. The launch description
is [release-v2.0.0.md](release-v2.0.0.md); the terminal and recovery contract is in
[interactive-cli.md](interactive-cli.md) and [v2-cli-plan.md](v2-cli-plan.md).
The private Pactrail Bench repository is outside this release.

## Required evidence

Before merging/tagging, the exact candidate must pass:

- Formatting, strict all-feature workspace Clippy, all workspace tests,
  warnings-as-errors rustdoc and optimized builds.
- Historical production-reader fixtures and permission/storage fault tests.
- Real-binary terminal scenarios, including running input, guarded decisions,
  approval ownership, stopped drafts, Unicode, seven widths, NO_COLOR and dumb mode.
- Guided setup: hidden entry, restart, cancellation, replacement, no-auth endpoints,
  endpoint binding, private Unix permissions, Windows DPAPI and secret-free web defaults.
- Browser projection/design tests and loopback security tests.
- Linux, macOS and Windows CI; dependency policy; hostile-repository Docker tests.
- Candidate installer checks: correct executable version and checksum rejection
  without changing the existing installation.
- Descriptor and repository-scale release soak with recorded budgets/RSS.

Evidence must distinguish a passed check from an ignored or unavailable check.
Local deterministic providers are interaction/protocol fixtures, not harness
quality benchmarks. No claim of universal task success or superiority is made.

## Publication procedure

1. Review and merge the tested candidate into `main`.
2. Wait for successful push CI on that exact main commit.
3. Confirm the package version, dated changelog and release notes agree.
4. Create `v2.0.0` on that commit and push the tag when publication is approved.
5. The release workflow rejects a commit without successful exact-commit push CI,
   then tests platform builds/installers/packages and the release soak before
   publishing checksummed, provenance-attested assets.
6. Verify published installer smoke jobs and the public release description.

Preparing the candidate does not publish a tag. Existing live sessions must be
restarted after installation; do not terminate another user's active task.
