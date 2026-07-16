# Pactrail v0.1.0

Pactrail's first public release is a verification-native coding-agent harness
for local and hosted OpenAI-compatible models. Models propose work inside an
isolated transaction; users receive an integrity-checked receipt, immutable
diff, and hash-linked execution trace before the source workspace changes.

## Install

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/AKMessi/pactrail/main/install.ps1 | iex
```

Linux x86_64 or Apple Silicon macOS:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://raw.githubusercontent.com/AKMessi/pactrail/main/install.sh | sh
```

The installers verify the downloaded archive against `SHA256SUMS`. Release
artifacts also carry GitHub build-provenance attestations. Source installation
remains available with:

```console
cargo install --git https://github.com/AKMessi/pactrail.git --locked pactrail
```

## What ships

- A persistent, width-aware terminal experience with live model/tool/context/
  verification activity and complete durable `/trace` inspection.
- Typed, schema-validated tools with capability policy, bounded output,
  path confinement, and parallel-safe read scheduling.
- Copy-on-run workspace transactions, immutable review diffs, explicit apply or
  discard, source-drift detection, and idempotent crash recovery.
- Provenance-aware workspace memory and integrity-backed applied-run history.
- Ollama, llama.cpp, vLLM, SGLang, LM Studio, LocalAI, OpenAI, and compatible
  hosted endpoints through the bounded Chat Completions adapter.
- Scriptable JSON commands, task contracts, shell completion, failed-run
  diagnostics, and portable JSONL traces.

## Trust boundary

Native process execution is disabled by default. When enabled, child processes
have the host process's filesystem, network, secret, and external-service
authority; the edit transaction is not an operating-system sandbox. Keep it
disabled for untrusted repositories. Integrated OS/OCI sandboxing, MCP,
streaming, and native provider adapters remain post-v0.1 roadmap work.
