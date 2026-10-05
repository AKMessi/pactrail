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
