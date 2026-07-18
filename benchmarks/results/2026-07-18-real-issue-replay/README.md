# Real issue replay: Pactrail vs OpenCode

This is the repository-level benchmark that the earlier synthetic matrix could
not answer. It compares Pactrail 0.1.0 and OpenCode 1.2.27 on real historical
Rust defects using pinned pre-fix source revisions, hidden behavioral tests,
fresh workspaces, process access for both harnesses, and pass@1 scoring.

The honest result is a **1/3 to 1/3 tie on hidden-test correctness**. OpenCode
leads **1/3 to 0/3 on strict end-to-end completion**. Pactrail used **6.7% fewer
provider-reported tokens**, **55.7% less summed agent wall time**, and **67.1%
less estimated API cost**. Every Pactrail source workspace remained untouched
before review and every Pactrail trace passed its hash-chain check.

This run does **not** support a broad claim that Pactrail is better than
OpenCode. It supports narrower claims about efficiency, transactional safety,
auditability, and patch focus while identifying completion reliability as the
most important remaining gap.

## Held-out headline

| Harness | Functional | Task | Strict | Agent time | Tokens | Estimated cost |
|---|---:|---:|---:|---:|---:|---:|
| Pactrail 0.1.0 | **1/3** | **1/3** | 0/3 | **110.96 s** | **578,090** | **$0.012920** |
| OpenCode 1.2.27 | **1/3** | **1/3** | **1/3** | 250.43 s | 619,764 | $0.039319 |

`Functional` means the hidden target test and broader regression command
passed. `Task` additionally applies the manifest's automated path rules.
`Strict` also requires a clean harness completion; for Pactrail it requires a
ready-to-apply isolated candidate, verified trace, and successful apply.
OpenCode is not penalized for lacking Pactrail-specific transaction concepts.

## Per-task result

| Real defect | Pactrail | OpenCode | What happened |
|---|---:|---:|---|
| Crossbeam #1096, cloned select handles | FAIL | FAIL | Neither produced a patch. Pactrail stopped on its 250k aggregate token guard; OpenCode exhausted 16 steps. |
| SmallVec inline `leak` memory safety | FAIL | PASS | Pactrail's exact edits failed on CRLF and the model fell back to unavailable `sed`. OpenCode passed both tests but reformatted 8,321 lines across `src/lib.rs` and `src/tests.rs`. |
| reqwest blocking request conversion | PASS, not strict | FAIL | Pactrail produced the focused upstream-equivalent 4-line fix and passed both graders, but unrelated auto-discovered checks prevented ready-to-apply. OpenCode implemented the behavior but added an unused import, so compilation failed under `deny(warnings)`. |

Pactrail's correct reqwest candidate changed one production file with four
additions. Its trace and failed receipt were retained even though the engine
did not promote the candidate. OpenCode's SmallVec candidate was functionally
correct, but its 265,337-byte patch also changed the formatting of a test
module despite the prompt's "do not modify tests" instruction. The primary
table preserves the preregistered automated score; this manual patch audit is
reported separately rather than silently changing the headline after seeing
the result.

## Efficiency

| Metric | Pactrail | OpenCode | Pactrail relative |
|---|---:|---:|---:|
| Provider-reported tokens | 578,090 | 619,764 | 6.7% fewer |
| Summed agent wall time | 110.96 s | 250.43 s | 55.7% lower |
| Estimated direct API cost | $0.012920 | $0.039319 | 67.1% lower |
| Model calls | 40 | 46 | 13.0% fewer |
| Tool calls | 52 | 72 | 27.8% fewer |
| Failed tool calls | 12 | 6 | Pactrail worse |

Time covers the harness/model phase, not deterministic grading. Token totals
are provider-reported input plus output, including cached input. Cost applies
the published DeepSeek V4 Flash cache-hit, cache-miss, and output rates to the
reported usage. The account balance moved from $1.71 to $1.66 across the six
trials; the endpoint reports only cents.

## Development set

Before the held-out run, a separate three-task development suite was executed
on both DeepSeek V4 Flash and Pro. It covered Bytes #798, fd PR #2036, and
Pactrail's historical idempotent-discard defect. Those results were used to
identify and fix general search, read pagination, and bounded-run recovery
problems, so they are not confirmation evidence.

| Harness | Functional | Strict | Time | Tokens | Estimated cost |
|---|---:|---:|---:|---:|---:|
| Pactrail | 1/6 | 0/6 | 208.66 s | 767,189 | $0.039426 |
| OpenCode | **2/6** | **2/6** | 402.33 s | 900,412 | $0.103965 |

This development result also does not support a universal superiority claim.
It does show the same efficiency/completion tradeoff observed in the held-out
set.

## Protocol

The held-out suite was committed and pushed at `77ebbf4` before any scored
model call. Gold validation established that all three targeted tests fail at
the pinned base revision and pass, with regression tests, at the pinned
reference revision.

Both harnesses used:

- `deepseek-v4-flash` through the direct DeepSeek API;
- temperature zero and thinking disabled;
- 16,384 context tokens, 2,048 maximum output tokens, and 16 steps/turns;
- identical prompts, fresh sessions, synthetic single-commit workspaces, and
  hidden behavior graders;
- native process access and an explicit offline task instruction;
- one trial per task, no retry, no best-of-n, and no replacement sample;
- a 720-second case limit and a $1.05 account-balance stop floor.

Task order was counterbalanced: Pactrail first for Crossbeam, OpenCode first
for SmallVec, and Pactrail first for reqwest. The full manifest, hidden graders,
gold-validation record, and OpenCode configuration live in
[`issue-replay-heldout-v1`](../../issue-replay-heldout-v1/README.md).

## Assurance results

Across the three Pactrail trials:

- 3/3 source workspaces remained unchanged before explicit review;
- 3/3 portable traces passed Pactrail's integrity verification;
- the correct reqwest change remained isolated after model recovery stopped;
- no failed run was replaced or hidden.

OpenCode edits its source workspace directly. That architectural difference is
reported, not included in its functional score.

## What the benchmark found and fixed

The scored executable remains the preregistered `77ebbf4` build. After all six
results were frozen, commit `4e1dc1d` fixed three general defects without
rerunning or rescoring any task:

- generated CLI contracts now align aggregate token/attempt budgets with the
  configured context, output, and turn ceilings;
- exact and atomic edits accept newline-equivalent text while preserving LF or
  CRLF, preventing false misses and whole-file newline churn;
- Rust verification no longer compiles every bench/example by default, and a
  Rust `tests/` directory no longer falsely triggers `pytest`.

These fixes are engineering outcomes, not benchmark points.

## Reproduce

Checkout `77ebbf4`, set `DEEPSEEK_API_KEY`, build the release binary, validate
the gold revisions, then execute the six entries in the manifest's declared
order using the shared runner:

```powershell
cargo build --release --locked -p pactrail

./benchmarks/issue-replay-v1/run.ps1 `
  -Harness pactrail `
  -Model deepseek-v4-flash `
  -ManifestPath ./benchmarks/issue-replay-heldout-v1/cases.json `
  -OpenCodeConfig ./benchmarks/issue-replay-heldout-v1/opencode-deepseek.json `
  -ValidateGraders
```

Use `-CaseId TASK_ID` and the declared harness order for scored trials. A new
execution is a replication, not a replacement for the retained pass@1 sample.

## Evidence map

- [`comparison.json`](comparison.json): normalized task and aggregate results.
- [`protocol.json`](protocol.json): controls, hashes, order, spend, and known
  confounds.
- [`environment.json`](environment.json): host, toolchain, harness versions,
  revisions, and executable hashes.
- [`audit.json`](audit.json): artifact counts, integrity assertions, patch
  audit, and post-run fixes.
- [`raw/heldout`](raw/heldout): all six run logs, patches, grader outputs,
  Pactrail receipts/traces, and OpenCode JSONL streams, excluding redundant
  full source copies.
- [`raw/development`](raw/development): all twelve development-run artifacts,
  with the same exclusion.
- [`SHA256SUMS`](SHA256SUMS): SHA-256 manifest for the publication directory.

## Limitations

- Three held-out tasks and one pass@1 sample are too small for a broad ranking.
- This is not SWE-bench Verified and includes only Rust repositories.
- Pactrail's preregistered executable still had a 250k aggregate token ceiling.
  It stopped the Crossbeam trial after nine calls while OpenCode used 276,764
  total tokens across 16 calls. The step limit was matched; aggregate compute
  was not perfectly matched. This was fixed only after scoring.
- The manifest's automated forbidden-path rule for SmallVec covered `tests/`
  but missed `src/tests.rs`; the manual instruction audit discloses that gap.
- Provider load and prefix-cache state can affect latency and cost.
- Process access was enabled for trusted repositories and is not an OS sandbox.

## Defensible public claim

> On a preregistered three-task real-Rust issue replay with the same DeepSeek V4
> Flash model, Pactrail tied OpenCode on hidden-test correctness (1/3 each),
> used 6.7% fewer reported tokens, 55.7% less agent time, and 67.1% less
> estimated API cost, while preserving source isolation and verified traces on
> every run. OpenCode led strict completion 1/3 to 0/3, so this is evidence of
> Pactrail's efficiency and assurance advantages--not proof that it is broadly
> the better coding agent yet.
