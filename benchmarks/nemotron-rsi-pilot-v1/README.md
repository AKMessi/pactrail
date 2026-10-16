# Nemotron RSI-lab historical baseline

This experiment uses real `nvidia/nemotron-3.5-super-vl:free` responses through
the existing RSI supervisor, credential gateway and isolated engine. It does
not implement an RSI proposal or establish improvement. The first question is
whether the model can complete real externally graded tasks through this lab.

Two public historical development issues, requests-6028 and vue-core-11915,
receive three independent baseline runs each. Frozen order seed: 20261016.
Gold/grader files are excluded from agent execution. Source is read-only;
candidate writes pass through Pactrail, and agent commands are disabled.
External tests run in previously installed immutable upstream grader images.

Limits: 12 model attempts and 900 seconds per trial, 32,768 engine context,
4,096 output tokens, 240,000 engine-counted tokens. The gateway's conservative
text-byte reservation is separately 131,072. Campaign limits: 72 attempts,
2 hours, and reported tokens plus the next conservative reservation <=2M.
Input/output rates are zero for this exact free model. Four-second pacing stays
below the standard 20 RPM limit. Daily allowance is monitored, never assumed
unlimited; quota rejection stops the campaign. No paid/provider/model fallback,
automatic retry, account upgrade or request rotation is permitted.

The trusted transport adds only zero-price provider routing and disables
fallbacks. It preserves the model conversation and tools byte-for-byte,
retains bounded raw responses/errors and allowlisted quota headers, checks exact
model identity, rejects nonzero reported cost, and stops further inference on
any transport/admission/protocol error. Credentials remain on the trusted host.
Unknown usage is never free budget for another request. Request reservations
persist after interruption. Provider responses are untrusted, not proof of
invoice status, unlimited access or stable future availability.

## Execute

Rerun base/gold validation using `benchmarks/harness-lab-v1/container_grade.py`
into `OUTPUT/grader-validation/TASK/{base,gold}` before scoring. Do not expose
the gold patch or grader to the model. All four reports and their logs are hashed.

```sh
python3 -m unittest discover -s benchmarks/nemotron-rsi-pilot-v1 -v
python3 benchmarks/nemotron-rsi-pilot-v1/pilot.py \
  --lab /absolute/validated/harness-lab-data \
  --output /absolute/new-output-with-grader-validation \
  --binary /absolute/qualified/pactrail \
  --env-file /absolute/pactrail/.env
```

The protocol and its SHA-256 are written before the first scored request. Every
declared trial remains in results, including model failure and unattempted trials
after infrastructure failure. No rerunning only failures or changing graders.
Strict success requires targeted/regression passes AND ready-to-apply, valid
receipt/trace, unchanged source. A correct partial candidate is not strict
completion. Raw journals, requests/responses, traces, receipts, candidates and
external grader reports are retained in ignored `benchmark-results/`.

Reuse of the earlier Groq pilot is limited to historical-task external grading
and shared imports; no Groq transport or model is used. These two cases were
already inspected during development, so their results cannot serve as held-out
confirmation or establish general harness quality. No statistical claim from
six trials, no causal self-improvement claim, and no automatic promotion.

## Audit a completed run

```sh
python3 benchmarks/nemotron-rsi-pilot-v1/analyze.py /absolute/completed-output
```

The analyzer verifies the frozen protocol, all stored artifact digests,
journal-bound trial results, usage exports and exact response model identities.
It writes `analysis.json` and `inventory.sha256.json`. Missing grading/usage stays
unknown; functional success is separate from strict workflow completion. Trace
metrics include rejected tools and patch writes. Failure labels are descriptive
log-derived summaries, not independent causal diagnoses. Raw artifacts remain
the evidence. Do not redirect stdout into the inventoried directory while
creating the inventory, or regenerate it after the output file closes.

The first completed result is documented in
`docs/research/nemotron-rsi-baseline-20261016.md`. The two development tasks must
not be recycled as independent held-out confirmation for a later RSI proposal.
