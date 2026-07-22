# Pactrail v1 real-issue confirmation

This is a preregistered, matched-harness comparison of Pactrail 1.0.0 and
OpenCode 1.2.27 on three real Rust repository defects that were not used in
Pactrail's earlier development or confirmation suites.

The frozen Pactrail revision is `5d0128862ac0688cd189c371843425f09e4befaa`.
The frozen release executable reports `pactrail 1.0.0` and has SHA-256
`90da5dc5053077a4bb6fc3294364583b93009a4f3667319b4f3632d9c821b41a`.

Both harnesses use DeepSeek V4 Flash in non-thinking mode at temperature zero,
a 16,384-token context, a 2,048-token output cap, and 16 turns or steps. Every
task is pass@1 with no retry or replacement. Both harnesses receive the same
prompt, unrestricted process execution, prefetched offline dependencies, and
the same hidden behavioral grader. OpenCode is not scored on Pactrail-only
transaction and trace properties.

The primary metric is strict end-to-end completion. Hidden-test correctness,
instruction compliance, tokens, estimated API cost, wall time, model/tool
calls, failed tool calls, patch size, and Pactrail's assurance properties are
reported separately. No aggregate result will be converted into a universal
"better than" claim.

The host uses one disclosed toolchain normalization for the HTTP repository:
it explicitly allows Rust 1.95's newly denied `dangerous_implicit_autorefs`
lint. The normalization is applied identically to the base, reference, and
both harness workspaces and does not touch `HeaderMap::reserve` behavior.

The execution order is paired and counterbalanced by task:

1. regex / Pactrail
2. regex / OpenCode
3. http / OpenCode
4. http / Pactrail
5. ripgrep / Pactrail
6. ripgrep / OpenCode

The run checks the DeepSeek balance before every case and stops before another
model call once it falls below $1.10. An incomplete trial is retained and is
not replaced.

Before model execution, validate that every hidden grader rejects the pinned
base revision and accepts the upstream reference fix with its broader
regression test:

```powershell
./benchmarks/issue-replay-v1/run.ps1 `
  -Harness pactrail `
  -Model deepseek-v4-flash `
  -ManifestPath ./benchmarks/issue-replay-v1-confirmation/cases.json `
  -OpenCodeConfig ./benchmarks/issue-replay-v1-confirmation/opencode-deepseek.json `
  -OutputDirectory ./benchmark-results/issue-replay-v1-confirmation `
  -WorkspaceDirectory D:/AKMESSI/CODING/AI/Projects/pactrail-benchmark-work/v1-confirmation `
  -ValidateGraders
```

The scored result directory, environment record, normalized comparison,
integrity hashes, and an honest limitations section are added only after all
six declared trials have either completed or hit the stopping rule.

## Secondary V4 Pro replication

After the six V4 Flash outcomes were frozen, a second manifest was registered
for an unchanged V4 Pro replication. It uses the same tasks, graders, prompts,
limits, order, harness binaries, and non-thinking mode, with a $1.30 balance
floor. Because the evaluator had already observed the Flash outcomes, this is
a model robustness replication and not another independent held-out result.
