# Real Issue Replay: held-out confirmation v1

This suite is the preregistered confirmation set created only after the
development replay exposed three general Pactrail failure modes. None of its
repository states, tasks, or graders appeared in the development set.

The frozen Pactrail revision is `988dacb`. The comparison uses DeepSeek V4
Flash, temperature zero, thinking disabled, a 16,384-token context, a
2,048-token output cap, and 16 turns/steps. Every task is pass@1. Both harnesses
receive the same prompt, unrestricted process execution, offline dependencies,
and the same hidden behavioral grader. OpenCode is not scored on Pactrail-only
assurance properties.

The tasks replay real defects in Crossbeam Channel, SmallVec, and reqwest. The
Crossbeam grader comes directly from its pinned reference commit. SmallVec and
reqwest use disclosed evaluator-owned regression tests because their reference
fixes did not add independently overlayable tests. Exact patch similarity is
never scored.

Two platform controls are applied before the synthetic baseline commit. Git
symlinks in Crossbeam are materialized as ordinary files for Windows, and
reqwest's default TLS feature is disabled to avoid an unrelated aws-lc ARM64
linker failure. Neither control touches the behavior under test; both are fully
specified in the manifest.

Execution is paired and counterbalanced by task. The run stops before another
model call if the DeepSeek balance reaches $1.05. There are no retries,
best-of-n selection, replacement samples, or post-result harness changes.

Run gold validation before scoring:

```powershell
./benchmarks/issue-replay-v1/run.ps1 `
  -Harness pactrail `
  -Model deepseek-v4-flash `
  -ManifestPath ./benchmarks/issue-replay-heldout-v1/cases.json `
  -OpenCodeConfig ./benchmarks/issue-replay-heldout-v1/opencode-deepseek.json `
  -ValidateGraders
```
