# Pactrail v1.0.0

Pactrail v1.0.0 is the first stable release of the verification-native coding
agent harness. The stable boundary is the whole transaction: model-neutral
context and tools, isolated candidate edits, hash-linked execution evidence,
deterministic verification, receipt-bound review, and explicit apply or discard.

## What becomes stable

- The documented non-interactive CLI, interactive commands, exit behavior, and
  versioned JSON envelopes follow the 1.x compatibility policy.
- Every durable format reported by `pactrail compatibility --json` has an exact
  reader/migration strategy and immutable historical fixtures.
- `pactrail-sdk` is the stable static Rust embedding facade for custom model and
  tool integrations. The immutable Git tag is the v1 source distribution
  contract; implementation crates remain internal.
- Windows x86_64, Linux x86_64, and Apple Silicon macOS are Tier 1 platforms
  with checksum-verifying installers and release-blocking CI.
- OpenAI, Anthropic, Gemini, Ollama, and OpenAI-compatible local/hosted endpoints
  share provider-neutral conversation, capability, tool, cancellation, usage,
  and trace semantics.

## Reliability and security closure

The development series since the v0.3 public release adds content-addressed
repository analysis, bounded Tree-sitter structure with lexical fallback,
process-free Git evidence, integrity-bound image inputs, deterministic
repository-scale soak gates, historical compatibility fixtures, crash-safe
source replacement and fault injection, read-only upgrade/deprecation
reporting, and a public maintainer security audit.

That audit fixed link-redirection hazards in durable/MCP/artifact/cache state,
removed OCI environment values from runtime arguments, aligned buffered
OpenAI-compatible responses with streaming bounds, and sanitized dynamic engine
trace fields. The final review pass also bound every diff read to an
integrity-valid receipt and authoritative event head, rejected redirected
review/candidate paths, required the user-selected workspace to match every
inspect/diff/apply/discard transaction, and made unknown-run inspection fail
without creating state. It found no remaining critical or high-severity issue
in the reviewed scope. This was not an independent third-party audit; the
report and residual risks are published in `docs/security-audit-v1.md`.

The CLI closes the same evidence loop for humans and automation: narrow
terminal help and trace views use deliberate hanging indentation, while
`pactrail diff <RUN_ID> --json` and the `pactrail runs` alias expose immutable
review data and history without scraping terminal prose.

## Upgrade from 0.x

Finish or discard active candidates with the old binary, install v1, and run:

```console
pactrail --version
pactrail upgrade
pactrail migrate --apply  # only when the preflight reports known migrations
pactrail upgrade
```

The preflight is read-only. Apply mode refuses active run locks and unknown
future formats, migrates known state atomically, and re-audits integrity. Source
workspaces and pending candidate bytes are never silently rewritten. The 0.x
developer-preview line is unsupported after this release.

## Install

Windows PowerShell 5.1+:

```powershell
irm https://raw.githubusercontent.com/AKMessi/pactrail/main/install.ps1 | iex
```

Linux x86_64 or Apple Silicon macOS:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://raw.githubusercontent.com/AKMessi/pactrail/main/install.sh | sh
```

For source installation, Rust 1.95 or newer is required:

```console
cargo install --git https://github.com/AKMessi/pactrail.git --tag v1.0.0 --locked pactrail
```

Every binary archive, installer, deterministic three-run soak report, and
resource log is covered by `SHA256SUMS`. Every published asset, including that
manifest, receives a GitHub build-provenance attestation. Consult
`docs/support.md`, `docs/compatibility.md`, and `docs/threat-model.md` before
production deployment.
