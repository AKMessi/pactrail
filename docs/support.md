# Support matrix

This matrix separates Pactrail runtime support from model quality. A transport
can be fully supported while an individual model is too small, lacks native
tool calling, or emits invalid calls. `/probe`, `/status`, and the durable trace
make that distinction visible.

## Platforms

| Tier | Platform | Distribution | CI contract |
|---|---|---|---|
| 1 | Windows x86_64, PowerShell 5.1+ | Provenance-attested GitHub release ZIP and checksum-verifying installer | Full format, Clippy, test, release-build, failure-matrix, installer, and real-binary coverage |
| 1 | Linux x86_64 | GitHub release tarball and checksum-verifying installer | Full gates plus restricted-Docker hostile-repository containment and release soak |
| 1 | macOS Apple Silicon | GitHub release tarball and checksum-verifying installer | Full format, Clippy, test, release-build, failure-matrix, installer, and real-binary coverage |
| 2 | Other Rust 1.95-compatible targets | Build from source | Best effort; no prebuilt artifact or platform-specific response-time promise |

Tier 1 means a release is blocked when its required build, test, or installer
job fails. It does not mean every container runtime, shell, filesystem, model
server, or corporate network configuration is certified.

## Model providers

| Provider family | Pactrail adapter | Support level |
|---|---|---|
| OpenAI API | Native bounded Chat Completions mapping | Supported |
| Anthropic API | Native Messages mapping | Supported |
| Gemini API | Native GenerateContent mapping | Supported |
| Ollama | Loopback OpenAI-compatible mapping and model discovery | Supported local default |
| llama.cpp, vLLM, SGLang, LM Studio, LocalAI | Loopback OpenAI-compatible mapping | Supported when the server implements Chat Completions tool calls |
| Hosted OpenAI-compatible gateways | HTTPS OpenAI-compatible mapping | Supported at the protocol boundary |
| Custom Rust providers | `pactrail-sdk::model::ModelDriver` | Stable 1.x embedding contract |

All built-in transports have bounded buffered and streaming parsers, strict
tool-argument validation, cancellation, credential-safe errors, explicit
capability profiles, and deterministic fixtures. Provider-specific beta APIs,
Assistants/Responses APIs, implicit model fallback, remote image fetching, and
provider-managed code execution are not part of v1.

Model behavior is never guaranteed by provider support. A model must be able to
follow the exposed JSON Schema tool protocol within the configured context and
output limits. Missing `/models` support affects discovery only; an explicitly
configured model remains usable.

## Tools and execution

- Built-in file, search, context, Git-evidence, memory-recall, mutation, and
  verification tools are supported on every Tier 1 platform.
- MCP 2025-11-25 stdio and Streamable HTTP are supported through explicit
  inspect/snapshot/enable lifecycle. Legacy MCP SSE and OAuth discovery are not.
- Process execution defaults to disabled. Native mode is trusted host execution,
  not a sandbox. Restricted OCI is supported with a locally attested Docker or
  Podman runtime and the assumptions documented in the threat model.
- Docker is the release-gated hostile-repository containment runtime. Podman
  follows the same command contract and unit suite but is not a Tier 1 CI runtime.

## Maintenance window

The latest stable 1.x minor receives correctness and security fixes. After a new
1.x minor ships, the previous minor remains eligible for critical/high security
fixes for 90 days. The 0.x developer-preview line is unsupported after v1.0.0.
State created by 1.0 remains readable or atomically migratable throughout 1.x as
defined by the compatibility contract.

Bug reports must include the Pactrail version, platform, provider family,
process backend, reproduction, and a sanitized trace when available. Never post
credentials or private source. Use the private channel in `SECURITY.md` for
suspected vulnerabilities.
