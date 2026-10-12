# Groq Qwen free-plan diagnostic

This is a baseline feasibility experiment, not evidence that a proposed harness
change improves Pactrail. Two existing development cases (`psf-requests-6028`
and `vuejs-core-11915`) are each attempted twice with the exact hosted model
`qwen/qwen3.8-27b`. The cases are public historical issues, not independently
curated held-out tasks.

The operator must confirm that the key belongs to a **Free-plan organization**.
Groq's model-list API does not establish billing status. Never upgrade the
organization, enable a paid service tier, substitute another model or silently
retry a failed trial. Provider rate-limit headers are checked against the
published free-plan 1,000 requests/day and 8,000 tokens/minute. They are an
additional consistency check, not independent proof of zero billing.

The existing self-improvement supervisor owns request reservations and keys;
its OCI execution keeps source read-only, candidate mutations isolated and
model networking blocked. The benchmark-specific trusted transport uses an
explicit User-Agent, preserves allowlisted rate-limit headers, redacts the key,
rejects redirects/proxy routing and enforces a total deadline in a host worker.
No engine/provider implementation or authority rule is changed.

Before scoring, externally rerun and retain base/gold container validation for
both issues using `benchmarks/harness-lab-v1/container_grade.py`. Store results at
`OUTPUT/grader-validation/TASK/{base,gold}/result.json`. Base must fail executed
targeted tests; gold must pass targeted and declared regression tests. Gold and
grader sources are never mounted in model execution containers.

```sh
python3 -m unittest discover -s benchmarks/groq-qwen-pilot-v1 -v
python3 benchmarks/groq-qwen-pilot-v1/pilot.py \
  --lab /absolute/validated/harness-lab-data \
  --output /absolute/new-output-with-grader-validation \
  --binary /absolute/current/pactrail \
  --env-file /absolute/pactrail/.env \
  --free-plan-confirmed
```

The protocol is written and hashed before the first scored request. It freezes
the binary/code/image, public source snapshots, input hashes, goal, seed/order,
permissions and limits. Each trial permits six physical model attempts and
900 seconds, with a 10,240-token engine context target and 1,024 output-token
ceiling. Gateway byte-based admission uses a separate, pessimistic 65,536-token
reservation ceiling; these are not claims of actual token usage.

The campaign permits at most 24 physical attempts and stops before reported
tokens plus the next pessimistic request reservation exceed 180,000. An
unscored connectivity probe used 61 reported tokens. Physical requests are
spaced at least 65 seconds apart. Groq's account-level quota also applies to
other users/apps and may interrupt the pilot earlier. Interrupted or unknown
usage remains accounted for and cannot be replayed.

Every declared trial counts, including failed and unattempted trials after a
provider/infrastructure stop. Strict success requires actual targeted and
regression passes plus valid receipt, trace, ready-to-apply state and preserved
source isolation. No model assertion is grading evidence. Failed runs may
retain a partial candidate without independently qualified receipt; this
initial pilot does not grade that partial candidate as a strict success.

Raw requests/responses, hash-chain journal, tool traces, receipts, source and
candidate snapshots, grader logs and usage are retained in ignored results.
Missing cache/invoice metrics remain unavailable. Summaries should report task
counts and failure causes; four runs cannot establish broad performance or
causal self-improvement. Do not rewrite an experiment after observing results.

The pilot explicitly maps Pactrail's `reasoning: {effort: low}` request to
Groq's `reasoning_effort: low`, preserving both request artifacts. The gateway
validates either representation against the frozen effort and refuses mixed
or additional reasoning settings. This is declared provider-format adaptation,
not a model or reasoning-policy fallback.
