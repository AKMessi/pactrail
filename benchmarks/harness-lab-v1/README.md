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
Docker is now accessible. The first actual container validation (Axios #4738)
proved base-targeted failure and gold-targeted/regression passes. Population
validation remains in progress; no scored model request has been admitted.

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

## Behavioral grading boundary

`grading.py` summarizes the exact declared targeted/regression test identities
from an external parser's status map. It rejects malformed/duplicate identities;
missing, skipped, errored, timed-out or infrastructure-failed tests cannot qualify
either a base failure or a gold pass. Extra passing tests cannot replace a missing
required test. Shell exit status alone is never behavioral evidence.

`container_grade.py` executes one prepared base/gold case in a fresh container,
with no host mounts, no network, no provider environment, bounded time/output,
explicit CPU/memory/PID limits, dropped capabilities and cleanup. It refuses an
image whose platform, digest, clean tracked source or pre-fix commit does not match.
Added gold files are included by applying the integrity-bound reference patch.
It never pulls implicitly. `parse_upstream.py` uses the externally installed
official SWE-bench parsers pinned to commit
`02e7a74ffd0b707aab73d203fe87bdc7c76afc8e`; their source hashes and parsed statuses
are retained. The external development dependency is not part of Pactrail runtime.

`validate_population.py` explicitly pulls locked images and serially validates
base/gold for the whole declared population, retaining unsupported cases and
infrastructure failures. Output is exclusive and every model result is unscored.
Candidate grading is available through `--source candidate --candidate . --phase
targeted|regression`, using a separate external grading copy. Patch preparation
preserves added/deleted/ignored files, binary data and raw line endings; source
attributes cannot invoke filters or normalize bytes. Source size/file counts are
bounded, and nested Git metadata is rejected. An actual Axios gold candidate
passed through this path. Admission-record export and the frozen cohort protocol
still need integration before any scored baseline can be admitted.

Example (after explicitly pulling the declared immutable image):

```sh
python3 benchmarks/harness-lab-v1/container_grade.py \
  --catalogue benchmarks/harness-lab-v1/catalogue.json \
  --prepared benchmark-results/agent-quality-program-20261005/prepared-v1/results.json \
  --images benchmark-results/agent-quality-program-20261005/image-lock-v1.json \
  --parser-python /home/akmessi/.cache/pactrail-harness-lab-swebench/bin/python \
  --task axios-axios-4738 --source base --output /tmp/axios-base-evidence
```

Actual Axios validation found a failing targeted test despite shell exit `0`.
The reference fix passed that test and three regressions in a separate container.
Reports distinguish raw `shell_exit_code` from behavioral test outcomes. Unit
tests establish interpretation/bounds, not container or model-quality results.

Qualification campaign v1 remains retained, including image setup failures and
unstarted cases at interruption. Campaign v2 repeats the whole population. It
restores the exact pre-fix Git commit inside each fresh container using existing
objects, verifies the resulting tracked source is clean, then applies the candidate
or reference patch. This handles upstream environment-setup checkouts and installed
dependency edits without changing graders/gold fixes or deleting dependency caches.
An unavailable commit or failed restoration still fails closed; there is no fetch
inside grading. No model requests have been scored under either campaign.

## Admission export and pre-scoring pool expansion

`admit.py CATALOGUE PREPARED_RESULTS COMPLETE_CAMPAIGN NEW_OUTPUT` checks the
campaign/input hashes, reinterprets named test statuses against the exact grader,
checks retained log hashes and emits admission records plus an all-case ledger.
Incomplete campaigns emit no directory. `exit_code_kind: derived_behavioral`
explicitly distinguishes the admission result from the raw `shell_exit_code`;
no shell success is converted into evidence of passing tests.

The initial population has environment-ineligible cases (nonregular source,
empty regression declarations and offline service dependencies). Before scored
model requests, an expanded pool selects up to three hash-ranked issues per
original repository with the same seed and repository partition. This yields
44 cases because Immutable.js has only two captured rows. All original 30 task
specifications are unchanged. Use `curate.py --issues-per-repository 3` to
reproduce it. This is an environment-qualified sampling process, not independent
held-out curation; its exclusions and selection bias must accompany results.
Original campaigns are retained. A final eligible catalogue and its protocol are
not yet frozen; this expansion itself is not evidence of model quality.
