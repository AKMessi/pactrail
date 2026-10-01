# Space Bunny Alpha: completion audit comparison

Completed the frozen 48 trials: six synthetic Python cases, four harness arms, two repetitions.
This measures this model on this small suite; it does not establish universal harness superiority.

## Aggregate outcomes

| Harness | Functional passes / graded | Recorded strict passes / retained | Admitted model requests | Reported tokens | Median agent seconds |
| --- | ---: | ---: | ---: | ---: | ---: |
| Pactrail before upgrade | 12 / 12 | 12 / 12 | 93 | 685,212 | 21.7 |
| Pactrail with audit | 12 / 12 | 11 / 12 | 141 | 1,168,320 | 36.8 |
| OpenCode 1.18.34 | 12 / 12 | 9 / 12 | 117 | 944,743 | 46.6 |
| mini-SWE-agent 2.4.6 | 11 / 12 | 11 / 12 | 85 | 200,665 | 30.6 |

Functional grading examines the candidate using external behavioral assertions.
Recorded strict success requires clean exit, no timeout, and the frozen cleanliness check.
Pactrail strict success also checks unchanged original production-file bytes, verified trace, successful apply,
and matching production-file bytes after apply. These extra assurance conditions differ
from the external harness checks; use functional outcomes for the common comparison.

## Rollout decision

Keep `--completion-audit` opt-in. Audit and baseline both passed 12/12 functional
trials, while audit used 1,168,320 reported tokens versus 685,212: **70.5% more**.
It also left a temporary check file in one candidate. This suite provides no
measured functional benefit to justify enabling unconditional audits by default.
Baseline Pactrail used fewer reported tokens than OpenCode here; mini used the
fewest tokens but missed one functional case. These are workload-specific
tradeoffs, not a universal ranking or a monetary-cost result.

The next evaluation should use harder independent repository issues with
caller-supplied acceptance checks, and test a separate bounded reviewer context.
Those capabilities and gains are not established by this run.

## Paired audit comparison

| Comparator | Paired graded trials | Audit wins | Ties | Audit losses |
| --- | ---: | ---: | ---: | ---: |
| Pactrail before upgrade | 12 | 0 | 12 | 0 |
| OpenCode 1.18.34 | 12 | 0 | 12 | 0 |
| mini-SWE-agent 2.4.6 | 12 | 1 | 11 | 0 |

## Cases

| Case | Before | Audit | OpenCode | mini |
| --- | --- | --- | --- | --- |
| paired-clamping | pass, pass | pass, pass | pass, pass | fail, pass |
| unicode-key-collisions | pass, pass | pass, pass | pass, pass | pass, pass |
| decimal-invoice-rounding | pass, pass | pass, pass | pass, pass | pass, pass |
| pagination-boundary | pass, pass | pass, pass | pass, pass | pass, pass |
| expiry-timezone-consistency | pass, pass | pass, pass | pass, pass | pass, pass |
| stable-topological-order | pass, pass | pass, pass | pass, pass | pass, pass |

## Limits and reproducibility

- Every scored trial is retained; no replacement samples or post-result parameter tuning.
- Frozen v1 cleanliness checks missed newly added empty files. Strict scores are as recorded, not a complete file-inventory audit; functional grading is unaffected. The runner is corrected for future experiments.
- The request count is admitted upstream attempts. Local requests rejected after the cap are not counted in this ledger.
- The frozen console progress label could show the last changed filename. Result identities and frozen order use JSON, which are unaffected.
- Reproduce this scored version from commit 09e4ba6. Original runner sources are archived under frozen-runner; the current runner may contain subsequent fixes.
- Six synthetic cases are not independent real repository issues; repeated trials share cases.
- Exact free model, temperature 0, low reasoning, 8,192 output tokens, 12 HTTP attempts, 300-second agent deadline.
- Common input bound is serialized message bytes, not equal tokenizer context windows.
- Harness prompts, tools, recovery, and stopping differ. Native local commands are allowed.
- Buffered upstream responses cannot measure true streaming latency.
- API pricing was zero; this cannot prove monetary savings. Request usage coverage is in summary.json.
- Agent seconds exclude external grading and Pactrail apply. Provider errors are retained per trial.
- Original and published artifact hashes are in artifact-manifest.json; only ephemeral proxy credentials are redacted.
- Runner and task hashes, pinned versions, original implementation hashes, and frozen order are in protocol.json.
- Official comparator sources: https://opencode.ai/docs/cli and https://mini-swe-agent.com/latest/reference/run/mini/.

The policy remains opt-in. Default rollout requires broader independent issues and cost/outcome evidence.
