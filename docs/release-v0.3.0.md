# Pactrail v0.3.0

Pactrail v0.3.0 upgrades the parts of a coding agent that sit between the model
and the source tree: repository navigation, trajectory context, edit feedback,
and deterministic validation. The transaction boundary is unchanged—the model
still works in an isolated candidate, and source files change only after an
explicit, receipt-bound apply.

## Highlights

- A bounded repository evidence graph connects project-defined symbols to
  exact lexical definition and reference locations. Initial context can expand
  one relationship hop, and models can query the current candidate with the new
  read-only `search_code_graph` tool.
- Long runs no longer carry every historical tool result forever. Pactrail
  deterministically compacts older observations into BLAKE3-bound envelopes
  while retaining recent evidence and valid tool-call/result ordering. Every
  compaction is visible in the live timeline and durable trace.
- Successful writes and exact edits return current candidate evidence: final
  digest, changed-line bounds, and bounded line-numbered previews. Distant or
  oversized edits say explicitly when source was omitted and how to read it.
  Exact no-op replacements are rejected.
- Authorized deterministic checks now act as a completion gate. A genuine
  compiler or test failure can trigger one bounded repair cycle with
  model-aware, digest-bound diagnostics labelled as untrusted process output.
  A repaired candidate is verified again in a fresh disposable snapshot.
- Passing completion gates become final evidence directly when the candidate
  digest is unchanged, avoiding a duplicate test run.
- Native verification inherits an explicit non-secret Windows toolchain and SDK
  discovery environment, fixing spurious `link.exe not found` failures while
  continuing to exclude API keys, external Cargo target directories, and
  compiler wrappers.

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
`SHA256SUMS`. GitHub also publishes build-provenance attestations for every
release artifact.

Source installation:

```console
cargo install --git https://github.com/AKMessi/pactrail.git --tag v0.3.0 --locked pactrail
```

## Upgrade notes

This release does not rewrite source workspaces or silently apply pending
candidates. Existing `.pactrail` state remains in place; review or discard a
pending candidate before changing versions.

The evidence graph is deterministic lexical navigation, not a type-resolved
call graph. Native process execution remains disabled by default and is not an
operating-system sandbox when enabled. Automatic validation repair is capped at
one cycle and consumes the existing model-turn, token, process, and wall-time
budgets.

See the complete user-visible history in
[`CHANGELOG.md`](https://github.com/AKMessi/pactrail/blob/v0.3.0/CHANGELOG.md)
and the research-to-invariant mapping in
[`docs/research-foundations.md`](https://github.com/AKMessi/pactrail/blob/v0.3.0/docs/research-foundations.md).
