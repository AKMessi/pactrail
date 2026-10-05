# Agent-quality baseline — October 5, 2026

Starting main: `64da001c565a7cf8d662a872c5f9d6c564c289b9`, version 2.1.2.
No new scored model requests or promoted agent behavior are part of this baseline.

| Gate | Result |
| --- | --- |
| Full locked all-feature workspace tests | 456 passed, 0 failed, 1 existing local OCI test ignored |
| Formatting / strict Clippy / docs / release build | PASS |
| Agent / completion benchmark infrastructure | 7 / 4 passed |
| Real PTY terminal workflows | PASS |
| Installer integrity and tamper refusal | PASS |
| Three fresh 5,000-file index/context lifecycles | PASS; stable digests, exact expected cache invalidation |
| Local OCI execution | NOT QUALIFIED; Docker/Podman absent; noninteractive sudo unavailable |
| New 30-issue lab base/gold validation | NOT QUALIFIED; preparation and executable environment validation pending |
| New comparative model trials | NOT RUN |

The baseline all-feature release executable is preserved separately as
`/home/akmessi/.cache/pactrail-v2-build/release/pactrail-agent-quality-baseline-64da001`.
SHA-256: `f48d5a352666f1f47c4ad259424011a69cabe41e1ea7019f93edc8d067864e3b`.
Size: 25482960 bytes. Twenty `--version` subprocess launches
had a median of 3.57 ms. This measures the version command,
not interactive startup or model latency.

Release repository-scale medians over three fresh iterations: cold 883 ms,
warm 145 ms, one-file incremental 145 ms,
context 16 ms. GNU time maximum RSS was 28,556 KiB for the
whole runner, including generated fixtures. These are local measurements, not
universal timing claims or an idle interactive RSS measurement.

Raw logs, executable identity, exact commands and JSON are retained under
`benchmark-results/agent-quality-program-20261005/baseline/`. Public model metadata
is captured separately; inference availability is not inferred from catalog presence.

## Prospective performance gate

Compare identical workloads on this machine using at least five fresh repeated
samples before attributing small changes. Investigate median increases exceeding
20% **and** 50 ms for startup/index/context workloads. Investigate binary increases
above 10% and runner RSS increases above 20% **and** 20 MiB. These investigation
thresholds are not claims of statistical precision; required CI scale budgets and
security/compatibility gates remain hard gates. Interactive startup, idle RSS,
candidate creation, event append and tool latency still need their own baselines
before corresponding mechanisms change.

## Population custody

The 30-case catalogue is deterministic and repository-disjoint (16 development,
14 confirmation). It selects public upstream issues, not independently curated
Pactrail held-out evaluation. The initial pre-scoring selection requested two
Flask issues, but the upstream population had fewer than two; Pylint replaced
Flask before any validation/model outcomes. No task was removed based on scored
performance. The exact catalogue and source snapshot hashes preserve the final
candidate selection. All cases remain ineligible until retained local checks pass.

## Unscored model connectivity and platform checks

Both exact configured models completed read-only connectivity runs against the
frozen current-main binary: `stealth/space-bunny-alpha` and
`apodex/apodex-1.1-mini:free`, through `https://openrouter.ai/api/v1`.
Both outcomes were `answered`, with unchanged source fixtures. These are not
benchmark results. Raw responses, traces and receipts are retained under
`model-availability-v2/` in the ignored evidence root. The first attempt failed
CLI parsing before model I/O because `--temperature` is not a supported CLI flag;
that failed setup remains retained separately under `model-availability/`.

Branch CI run 37289618974 passed all Rust quality gates on Linux, macOS and
Windows, but the new lab test failed on macOS and Windows: its expected temporary
path spelling differed from the canonical path correctly stored in the protocol.
The test now compares canonical identities and their exact hashes. This repairs
the expectation without weakening admission, changing runtime behavior or skipping
platform coverage. Final-head CI must still pass before this milestone qualifies.

Docker is installed and account membership includes its group, but the current
agent process has not inherited the new supplementary group. Container grader
execution remains blocked pending a refreshed session. Immutable manifests for
22 prepared cases are pinned; that is not proof of grader correctness.

### Docker access resolved and first grader qualification

The operator granted access to the current Docker socket. Docker server 29.8.2
is now accessible; no daemon authority is exposed to agents. Linux/amd64 Axios
issue #4738 ran in two fresh containers with networking disabled, no host mounts
or provider credentials, dropped capabilities, bounded output/time/processes and
explicit memory/CPU limits. Base: one targeted failure and three regression
passes. Gold: one targeted pass and three regression passes. Both raw shell
exits were zero; the behavioral result was established by the commit-pinned
official SWE-bench TAP parser, not the shell status. Evidence is retained under
`validation-v1/`; its external parser environment and dependency inventory are
under `grading-sources/`.

The full population campaign is under `validation-population-v1/`, including a
runner-source snapshot. Unsupported preparation and grader failures remain in
the ledger. The first Gin case illustrates an admission failure: its upstream
grader declares no regression tests, so it cannot satisfy the nonempty-regression
gate. This is not converted into a pass or silently removed. No scored model
request or behavior change has occurred.

All six jobs in CI run [37292023633](https://github.com/AKMessi/pactrail/actions/runs/37292023633)
passed for commit `83ffb6a54c47d8bdbafce8e35d482d89614e391e`: Linux, macOS and
Windows quality, dependency policy, real terminal workflows and Docker
containment. This qualifies that earlier interpretation-component milestone,
not subsequent grading adapter changes.
