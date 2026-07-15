# Contributing to Pactrail

Thank you for helping build a trustworthy, model-agnostic coding harness.
Focused bug reports, threat analysis, design discussions, tests, docs, provider
fixtures, accessibility work, and code are all valuable.

## Before coding

- Search existing issues and pull requests.
- Small fixes can go directly to a pull request.
- Open a design issue before changing a public API, task/event/receipt/memory
  format, security boundary, provider protocol, tool contract, or architecture.
- Security vulnerabilities must follow [SECURITY.md](SECURITY.md), not the public
  issue tracker.

A design proposal should cover the user problem, alternatives, capability and
threat impact, durable-state/recovery behavior, observability, compatibility,
and test plan.

## Development workflow

1. Fork the repository and create a focused branch.
2. Follow [docs/development.md](docs/development.md) and the crate boundaries in
   [docs/architecture.md](docs/architecture.md).
3. Add regression and negative tests for observable behavior.
4. Update documentation and `CHANGELOG.md` for user-visible changes.
5. Run formatting, strict Clippy, all-feature tests, docs, and a release build.
6. Keep commits reviewable and sign them off with `git commit -s`.

By adding a `Signed-off-by` line, you certify the contribution under the
[Developer Certificate of Origin 1.1](https://developercertificate.org/).

## Pull requests

Explain the invariant or user behavior being changed, not only the files. State
how failure behaves, what authority is introduced, how durable compatibility is
handled, and the exact verification performed. Keep unrelated cleanup separate.

Review is expected to be rigorous around paths, capabilities, process/network
authority, memory provenance, persistence, concurrency, and apply/recovery.
Maintainers may request adversarial fixtures or a design issue before merging.

## Data hygiene

Do not include API keys, proprietary source, private model prompts/responses,
real user data, `.pactrail` state, large model files, or generated build outputs.
Sanitize paths and provider errors in reproductions. Test fixtures must be safe
to publish under the repository license.

Participation is governed by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
