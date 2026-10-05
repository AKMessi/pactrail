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

### Qualification correction before scored requests

Campaign v1 revealed upstream image setup state: Requests #1724 had a different
checkout than the task base (the task commit was present locally), and Preact
#3562 retained installation changes in `package-lock.json`. Both failed the
source-binding gate before grading. Their diagnostics, all completed case results,
the runner snapshot and the interruption record remain retained. Campaign v2
restores the declared base commit offline inside the grading container and verifies
clean tracked source before applying any patch; it repeats the whole population.
This repairs environment preparation rather than weakening the source-binding
gate. Nonempty regression coverage remains required. Gold patches and external
test definitions are unchanged. No model trial has been scored.

Candidate-patch integration has 23 infrastructure tests, including binary changes,
new ignored files, deletions, Unicode paths, CRLF under source-controlled EOL
attributes, source preservation, metadata rejection and file/byte/entry/depth bounds. An actual
Axios gold candidate passed targeted and regression tests through the candidate
adapter. This establishes grading mechanics, not model task success.

The next grading-adapter CI run (37293850155 at `8dd7b79`) passed Rust quality,
macOS/Linux, containment, dependencies and terminal workflows, but its Windows
byte-capture test expected LF from Python's platform-dependent `print`. The
fixture now writes the exact expected bytes through stdout's binary buffer,
preserving the stronger byte-for-byte assertion without changing capture behavior.

Buffered-mode connectivity is retained separately under
`model-availability-buffered-v1/`. Apodex reported the exact configured response
model on both turns. Space Bunny returned a malformed response without `choices`
in one unscored attempt; this remains recorded as a provider/protocol failure,
not silently replaced or converted into a successful trial. Its earlier streamed
connectivity succeeded. Transport settings and model-identity coverage still need
qualification and explicit preregistration before scored requests.

### Pre-scoring admission integration and expansion

The admission exporter rechecks parsed named outcomes against exact grader hashes
and retained log digests; raw shell exits stay separate from explicitly derived
behavioral exit codes. All rejected cases remain in an admission ledger. Its
regressions cover forged summaries, modified logs, incomplete campaigns and
exclusive output creation. Together with bounded candidate grading and population
expansion there are 27 local infrastructure tests, all passing.

The original 30-case pool cannot provide the requested minimum 20 fully qualified
tasks in these environments. Before any model scoring, an expanded candidate pool
uses the same seed, repositories and partitions, selecting up to three hash-ranked
issues per repository. Its 44 cases contain byte-identical specifications for all
30 originals; no task was chosen using model performance. Original failures remain
retained. Final environment-based eligibility introduces selection bias and must
be reported alongside future outcomes; public data is not independently sealed.

The pytest #10081 campaign rejects an upstream `XFAIL` status not supported by the
current strict interpreter. This is a parser/grading qualification failure, not a
model failure. No status is silently treated as a pass, and the original campaign
has not been rewritten. Any interpretation change needs separate tests and a new
explicit grading campaign before this task can qualify.

The completed original campaign retained 30 outcomes: 16 validated, eight
unsupported source preparations and six validation failures. Admission export
rechecked all 16 against exact parsed named tests, grader/source identities and
log hashes. The 44-case expanded campaign runs serially from a retained runner
snapshot; its 32 prepared sources and 12 refused sources are not model results.

A separate frozen buffered-transport diagnostic (`model-transport-qualification-v1`)
ran all six declared trials with a seeded order and no experiment-level retry or
fallback. Both exact configured models completed three of three; the CLI adapter
required the requested response-model identity on every turn. Source isolation,
engine trace and receipt inspection passed. The earlier Space Bunny malformed
response remains retained separately. Connectivity success is not coding quality
or a guarantee against later provider failures. No task has yet been scored.

Local admission-milestone gates passed: full workspace tests, strict Clippy,
formatting, rustdoc warnings and all-feature release build. Evidence is retained
under `admission-milestone-gates/`; final branch CI must qualify the eventual
committed implementation independently.

A pre-scoring source-sealing audit found that Git cloning a sealed baseline still
adds `origin` with the operator-side path. Harness Lab now requires explicit
`source_policy: sealed`; the shared runner verifies one reachable baseline commit,
removes remotes and checks cleanliness before invoking an adapter. A real Git
regression verifies removal and refusal of additional history. Existing historical
protocol defaults remain compatible. Seven runner tests and 27 lab tests pass.

Cohort derivation is now explicit: the complete all-case admission ledger selects
only validated environments, retains every exclusion and binds population,
admission and log hashes into later frozen protocols. It cannot produce a cohort
with fewer than 20 eligible tasks. Local tests cover unchanged task specs,
exclusive creation, insufficient cohorts and modified logs. The lab now has 29
passing infrastructure tests.

The matched runner separately records functional `task_success` and Pactrail
`strict_completion`. Strict completion requires targeted/regression success plus
validated receipt, trace, isolated source and ready-to-apply state. Missing checks
remain unknown. Existing functional scoring and old protocols/results are not
rewritten. Eight runner tests pass, including missing-vs-false assurance.

Sealed adapter source isolation now also compares bounded byte-level tree digests
before/after invocation, alongside Git status. The prior Git-only check could miss
ignored/new empty files. Both digests remain in provenance; control directories
are explicitly excluded. A regression detects ignored empty-file creation and
changed bytes. Thirty lab tests and nine matched-runner/real-CLI fixture tests
pass. This is an assurance measurement repair before scoring, not an agent-quality
improvement or a rewrite of earlier results.

The expanded campaign is complete: 44 declared cases, 22 qualified, 12
unsupported preparations and 10 grading-validation failures. The evidence-bound
cohort contains 13 development and nine confirmation tasks. All exclusions remain
in the admission ledger; confirmation tasks have not been scored or used to tune.

Failed runs may now receive explicitly opt-in partial-candidate diagnostic grading
only after trace and source-isolation checks pass. Diagnostic passes never turn a
failed run into functional or strict success. This preserves evidence of a correct
partial patch when completion fails. Normal scored-candidate grading is unchanged.

A live SIGTERM fault test removed its named grading container and retained the
cancellation failure; subsequent normal gold grading passed targeted and regression
checks. Container identity is persisted before creation. SIGKILL cannot run cleanup;
its retained identity supports operator cleanup, not an automatic-recovery claim.
Stray candidate arguments on base/gold grading are rejected. Thirty-one lab tests
and nine runner/real-CLI fixture tests pass. Full formatting, strict Clippy, workspace
tests, warnings-denied docs and release-build gates passed again; logs are retained
in `diagnostic-milestone-gates/`. No coding-quality trial has yet been scored.

Exact runner commit `96f42de75ed6eb4d2bff7ff6fd2922006d845f9b` passed all
six CI jobs in run `37340695463`, including Windows, macOS, Linux, Docker
containment and real terminal checks. The frozen Apodex development campaign
then encountered account-wide OpenRouter `free-models-per-day` HTTP 429 errors.
The campaign was interrupted and retained in full; it is not a completed quality
estimate. Laguna campaigns never started. Space Bunny was withdrawn by the user
before scoring because its free period ended. All unused protocols are retained.

The incomplete campaign also exposed a pre-model harness failure: a long issue
brief exceeded the memory search interface's 4,096-byte query bound. The CLI now
bounds only the advisory retrieval query at a UTF-8 boundary; the contract/model
still receive the complete task. Direct oversized memory queries remain rejected.
A real memory-context setup regression covers a multibyte character crossing the
limit. This fix requires a new scored protocol; earlier outcomes are not rewritten.
