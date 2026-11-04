# Crouton RSI-lab baseline

An explicitly configured reuse of the existing Nemotron free-only runner, not
a new engine or a model substitution inside an existing experiment. Exact model:
`stealth/crouton-alpha`. Zero prompt/completion prices must be present in the
live catalog; zero-price provider routing and no fallback are enforced. Quota,
transport, identity or infrastructure failures stop further inference. This
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
python3 benchmarks/nemotron-rsi-pilot-v1/analyze.py /absolute/completed-output
```

Raw artifacts remain in ignored `benchmark-results/`; protocols and sanitized
analysis can be tracked. Missing metrics remain null. All declared trials count,
including failures and unattempted trials after a provider/infrastructure stop.
Failure summaries are descriptive, not causal proof. Never change a scored
protocol; retain it and create another version if infrastructure needs repair.
