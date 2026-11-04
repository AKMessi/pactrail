# Crouton RSI-lab baseline

An explicitly configured reuse of the existing Nemotron free-only runner, not
a new engine or a model substitution inside an existing experiment. Exact model:
`stealth/crouton-alpha`. Zero prompt/completion prices must be present in the
live catalog; zero-price provider routing and no fallback are enforced. Quota,
transport, identity or infrastructure failures stop further inference by default. This
does not assume unlimited access or independently verify invoice charges.

Four public **development** issues cover Python, TypeScript, JavaScript and Rust.
Three independent repetitions per issue give 12 declared trials. Source, goal,
model settings, random order, budget, binary, graders, runner and image identities
are frozen and journal-bound before the first scored request. Gold fixes, hidden
graders, history/remotes and credentials are excluded from agent execution.
All candidate effects still pass through Pactrail; source is read-only and agent
process execution is disabled. Independent upstream tests grade the candidates.

Limits are explicit in `profile.json`: 16 attempts, 900 seconds, 32,768 context,
8,192 output, 320,000 engine-counted tokens per run. The conservative gateway
reservation is separately 131,072. Campaign cap: 192 requests, four hours,
reported tokens plus next reservation <=4M. Token limits are checked after model
response, so one invocation may overshoot. Four-second pacing, no automatic
provider-error retries, no alternative model/provider route and no account upgrade.

The larger output/turn ceilings differ from the earlier Nemotron pilot; these
are not a matched model comparison. Public development cases are not held-out
confirmation. There is no child harness, RSI improvement claim or promotion.
This diagnostic locates observed failure mechanisms for a future controlled
change, not a claim of general model or harness quality.

```sh
python3 -m unittest discover -s benchmarks/crouton-rsi-v1 -v
python3 -m unittest discover -s benchmarks/nemotron-rsi-pilot-v1 -v
# First run container_grade.py base/gold validation for every selected task into
# OUTPUT/grader-validation/TASK/{base,gold}; both executed outcomes must qualify.
python3 benchmarks/nemotron-rsi-pilot-v1/pilot.py \
  --profile benchmarks/crouton-rsi-v1/profile.json \
  --lab /absolute/validated/harness-lab-data \
  --output /absolute/fresh-output-with-grader-validation \
  --binary /absolute/qualified/pactrail \
  --env-file /absolute/pactrail/.env
python3 benchmarks/crouton-rsi-v1/analyze.py /absolute/completed-output
```

Raw artifacts remain in ignored `benchmark-results/`; protocols and sanitized
analysis can be tracked. Missing metrics remain null. All declared trials count,
including failures and unattempted trials after a provider/infrastructure stop.
Failure summaries are descriptive, not causal proof. Never change a scored
protocol; retain it and create another version if infrastructure needs repair.

## v2 transport qualification

The immutable v1 experiment stopped after one turn-exhausted Requests run and
a 120-second total timeout during the next trial. Twenty-two responses were
accepted, one invocation remained uncertain, and ten trials were unattempted.
Its zero successes cannot be reported as 0/12 model accuracy.

`profile-v2.json` preserves tasks/order/model/engine budgets but explicitly
permits a 240-second total provider timeout. The relay permits 250 seconds;
the engine's 300-second request bound, trial/campaign deadlines and cancellation
still apply. The ordinary gateway default remains 120 seconds. A v2 timed-out
trial is terminal and counts as failed. Its invocation is never retried or
refunded; its entire conservative token reservation remains counted. Only later
independently preregistered trials may continue. A quota, pricing, identity or
other transport/infrastructure error still stops the campaign. No hidden retry
or model/provider substitution is allowed. Use a fresh output directory and
`--profile benchmarks/crouton-rsi-v1/profile-v2.json` to run it.

## v3: grade what Apply would change

V2 was invalidated after the grader patch included unrelated lockfile deletions
that the engine receipt did not include. The engine workspace intentionally
excludes ignored files; a sparse workspace is not a full-repository replacement.
Force-tracking the sealed source alone did not resolve this, as the regression
test demonstrated. The fix reconstructs a complete source snapshot from the
immutable baseline and overlays only changes validated by the frozen parent's
receipt AND diff. Ignored untouched files remain; explicit deletions still land.
Raw workspaces stay retained. A run lacking a verified receipt has no functional
grade and counts as strict failure. No real source is applied or modified.

`profile-v3.json` keeps v2 model/tasks/order/budgets. It changes source admission
and grading reconstruction, not the model or engine. Use a fresh output directory.
V1/v2 results and their failure/invalidation reasons remain in `preregistered/`;
their apparent successes must not be used as qualified performance results.

The retained v3 run stopped on a real upstream HTTP 429 after four attempted
trials (one strict success, two turn-limit failures, one provider interruption).
Eight declared trials were unattempted. The full four-task comparison is NOT
QUALIFIED. See `docs/research/crouton-rsi-baseline-20261104.md`. Do not rerun only
the failures, silently resume this protocol or call the endpoint unlimited.
