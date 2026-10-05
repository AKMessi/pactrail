# Harness Lab v1 — admission foundation

This expands the candidate population to 30 public historical issues across
Python, Rust, JavaScript/TypeScript and Go. It does **not** yet qualify them for
scoring. Repository-disjoint development/confirmation splits reduce shared-repo
coupling; upstream public data can still have training contamination. The set
is not independently curated for Pactrail.

No behavior experiment may start until every task in its declared split has
locally retained base-targeted failure, gold-targeted pass and gold-regression
pass. A dataset label, compilation failure, timeout or zero-test success is
insufficient. Confirmation results must not be used for iterative tuning.

Gold patches, hidden tests and evaluation scripts remain outside agent trees.
The public catalogue stores hashes/identities, not gold content. Preparation must
export only the pre-fix tree and create one clean synthetic commit without a
remote; never clone the upstream full history into an agent workspace.

The lab reuses `agent-runtime-v1/run.py` for matched trials and retained failure
artifacts, rather than introducing another engine/adapter lifecycle. `lab.py`
adds admission gates and an exclusively created hash-bound protocol. It makes no
model requests. Validation evidence is curator-produced, integrity-bound data;
the gates do not attest that an arbitrary trusted curator told the truth.

## Current local evidence

Current-main baseline: 456 Rust tests passed, zero failed, one existing local OCI
test ignored; fmt, strict Clippy, docs, release build, seven agent-runner tests,
four completion-runner tests, real terminal checks and installers passed.
Three 5,000-file index/context lifecycles passed with stable identities.
Raw data: `benchmark-results/agent-quality-program-20261005/`.
Docker is now installed, but the current account cannot access its daemon socket.
Upstream container evaluation remains **not qualified**, and no scored request
has been admitted. Installation alone does not validate grading environments.

## Reproduce admission checks

```sh
python3 -m unittest discover -s benchmarks/harness-lab-v1 -p 'test_*.py'
python3 benchmarks/harness-lab-v1/lab.py check-catalogue benchmarks/harness-lab-v1/catalogue.json
```

Captured dataset-server row snapshots live under the ignored evidence directory.
They include private grader/reference content; do not copy them into agent trees.
To reproduce the registry from these exact hash-pinned snapshots:

```sh
python3 benchmarks/harness-lab-v1/curate.py \
  benchmark-results/agent-quality-program-20261005/task-sources/SWE-bench_Verified.json \
  benchmark-results/agent-quality-program-20261005/task-sources/SWE-bench_Multilingual.json \
  --output /tmp/harness-lab-catalogue.json
```

`lab.py freeze CATALOGUE PREPARED_PROTOCOL VALIDATION_DIRECTORY OUTPUT.json`
refuses missing validation, changed goals/graders, contaminated Git history and
unbound binaries. Output creation is exclusive: a scored protocol is never
rewritten. Prepared protocols use the existing runner schema plus `lab_split`,
`runtime_identity.binary`, each task's `source_base_commit`, and
`targeted_inputs`/`regression_inputs` source hash maps. This stage deliberately
has no runnable production experiment template until graders are validated.

Validation files are named `TASK_ID.json`, schema 1, with `task`, `task_sha256`
(SHA-256 of canonical registry task JSON), `grader_identity` and three records:
`base_targeted`, `gold_targeted`, `gold_regression`. Each record carries integer
`tests_executed`, `tests_failed`, `exit_code`, boolean `timed_out` and
`infrastructure_error`, plus a validation-local `log` and `log_sha256`.
Base must fail executed tests; both gold phases must pass nonempty executed
tests. Retain all attempts, including unavailable/invalid preparation cases.

Next work: prepared isolated task exports, executable pinned grading environments,
base/gold validation, immutable baseline protocol and actual repeated model runs.
Only then begin the anchor-edit experiment described in design 0020.

## Source preparation

`prepare.py CATALOGUE SOURCE_SNAPSHOTS LOCAL_MIRRORS NEW_OUTPUT` exports pinned
pre-fix revisions from already fetched operator-owned Git mirrors. Mirror names
use `owner--repository`. It performs no network access or dependency installation.
It seals a single baseline commit and keeps reference patches, gold trees and
grader JSON outside that source. Nonregular files/submodules and oversized trees
are rejected; failures remain in the per-case ledger. Preparation is not grading.

The first actual preparation retained 30 outcomes: 22 prepared, eight refused for
tracked symlinks/submodules. The exact paths and all failures are retained in the
ignored evidence directory. No refused task has been replaced or scored, and no
image tag has been treated as an immutable image digest. Those issues need an
explicit pre-scoring decision and validated environments before any protocol.

## Immutable grading image identities

`image_lock.py PREPARED_RESULTS NEW_OUTPUT` resolves explicitly declared public
SWE-bench image tags to integrity-checked Linux/amd64 manifest digests. Responses
are bounded; redirects and other image namespaces are rejected. Anonymous registry
tokens stay in memory. This step downloads manifests only, not image layers.

The local campaign pinned 22 image manifests and retained eight unsupported
preparation cases. Its `manifest_locked_not_executed` status is deliberately
different from a validated grader. Container execution must use these immutable
identities and separately prove base failure and gold targeted/regression passes.
