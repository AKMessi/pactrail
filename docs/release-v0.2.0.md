# Pactrail v0.2.0

Pactrail v0.2.0 makes model execution more predictable under real provider,
repository, and verification pressure. It also ships reproducible evaluation
infrastructure and the raw evidence behind Pactrail's published results.

## Highlights

- Repository task loops recover from repeated reads and oversized tool output
  with bounded file pagination, continuation metadata, and explicit synthesis
  steering.
- Candidate work is preserved for review when a model uses its final turn after
  making changes; deterministic verification still runs and the receipt records
  the completeness risk.
- Token, turn, and model-attempt ceilings now stay aligned with the configured
  context and output limits instead of being preempted by hidden defaults.
- Exact replace and atomic multi-edit tolerate newline-equivalent model text
  while preserving each file's existing LF or CRLF convention.
- Rust verification avoids unnecessary benches and examples, while Python test
  discovery no longer mistakes a Rust `tests/` directory for pytest.
- OpenAI-compatible providers gain configurable request deadlines, bounded
  `Retry-After` handling, explicit non-thinking requests, and strict
  single-system-message compatibility for Qwen-style chat templates.
- The repository now contains reproducible local-model, OpenRouter, DeepSeek,
  matched OpenCode, and preregistered real-issue replay evaluation tooling with
  machine-readable scores, raw traces, receipts, patches, and checksums.

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
`SHA256SUMS`. GitHub also publishes build-provenance attestations for the
release artifacts.

Source installation remains available:

```console
cargo install --git https://github.com/AKMessi/pactrail.git --tag v0.2.0 --locked pactrail
```

## Upgrade notes

This release does not migrate or rewrite source workspaces. Existing Pactrail
state remains under `.pactrail`; review or discard any pending candidate before
switching versions.

Native process execution remains disabled by default. When enabled, child
processes inherit the host process's filesystem, network, secrets, and external
service authority. Pactrail's edit transaction is not an operating-system
sandbox.

See the complete user-visible history in
[`CHANGELOG.md`](https://github.com/AKMessi/pactrail/blob/v0.2.0/CHANGELOG.md).
