# Groq Qwen free-plan diagnostic — 12 October 2026

## In simple words

We connected the real Qwen model to Pactrail's isolated benchmark runner. It
successfully chose tools, listed repository files and read source. However,
Groq's Free-plan input allowance stopped the runs before a fix was completed.
**This is useful infrastructure evidence, not a coding-quality result or proof
of recursive self-improvement.** No candidate improvement was accepted.

The immediate lesson is that Pactrail's full instructions/tool catalog and
growing conversation need more input allowance than this free endpoint offers
for these tasks. Waiting longer does not make an oversized individual request
fit. We did not enable paid access, upgrade the account or substitute a model.

## Scope and identity

- Branch: `research/recursive-self-improvement-v1`; main and release tags unchanged.
- Exact model requested and returned on accepted calls: `qwen/qwen3.8-27b`.
- Endpoint: `https://api.groq.com/openai/v1/chat/completions`.
- Account: operator confirmed Free-plan organization; invoice not independently verified.
- Catalog: active, Alibaba Cloud, 131,072 context / 16,384 maximum completion tokens.
- Model settings: temperature 0, reasoning effort low, no streaming.
- Release executable SHA-256:
  `efece82aa0c14f020254b4a87da396d3c3ff7fb6146054c46fab8aa89913d605`.
- Rust engine executable unchanged throughout this diagnostic. Python supervisor
  fixes/configuration changes are identified separately in each protocol.
- OCI execution image:
  `sha256:39702c3eb9ce2cfc26c6db60dbc78d59f4f5bac240d55da39a441226b3ee9559`.

The public catalog lists commercial prices; those are not evidence of billing
on the user-confirmed Free account. No commercial cost estimate is substituted
for an invoice. Rate headers are only a consistency check. See Groq's
[rate-limit documentation](https://console.groq.com/docs/rate-limits) and
[billing documentation](https://console.groq.com/docs/billing-faqs).

## Tasks and independent grading

Two public historical **development** issues were selected before scoring:
`psf-requests-6028` (proxy authentication) and `vuejs-core-11915` (`v-pre` with
textarea interpolation). They are neither a representative benchmark nor an
independently curated held-out set. Each protocol declared two repetitions per
task with order seed 20261012, six model attempts/run and 900 seconds/run.

External upstream container graders were validated before inference:

| Task | Known bad | Reference fix | Regression coverage |
| --- | --- | --- | --- |
| requests | 2 targeted failures | targeted pass | 185 passes |
| Vue | 2 targeted failures | targeted pass | 43 passes |

Gold patches, grading tests, repository remotes and original history were not
available to the agent. Source was mounted read-only; writes were permitted only
in Pactrail's candidate. Agent process execution was disabled. Verification was
external, not a model assertion. Strict success required targeted and regression
passes, ready-to-apply state, receipt integrity, trace integrity and isolation.

## Preserved experiment ledger

Every changed configuration received a new frozen protocol. Old results were
not rewritten, deleted or counted as successful model tests.

| Attempt | Code commit | Context / output ceiling | Physical requests | Outcome |
| --- | --- | --- | --- | --- |
| v1 preflight | c363859 | not scored | 0 | interrupted slow artifact import before scoring |
| v2 | b51b4adb87626734f2fb23b964bb9c9f59552e26 | 8,192 / 2,048 | 0 | instruction budget / reasoning-format admission failures |
| v3 | 28c8ecc47a1b6f327f8b9be9d380bfe9e574dc37 | 16,384 / 2,048 | 1 | first request rejected HTTP 413 |
| v4 | cac767baca64228af188ac94de5f4922e436f6c7 | 10,240 / 1,024 | 2 | first request accepted; next required 8,076 input tokens, limit 7,000 |
| v5 | 66bb595efd0791dde2f7de6d54ba67db20effdaa | 8,192 / 1,024 | 3 | two requests accepted; next required 7,581 input tokens, limit 7,000 |

v3–v5 stopped further inference after the first provider failure, retaining the
other three declared trials as unattempted. v2 completed its four runner
invocations without a physical model request; its original `completed` label
describes runner execution, not task completion. All receipt/ready checks failed.
No scored protocol produced a completed candidate. **A 0/4 model accuracy claim
would be misleading.** No paired harness-improvement comparison was performed.

An additional unscored connectivity probe returned `OK`, with 45 input and 16
output tokens. All six scored physical requests remain recorded: three accepted,
three rejected. Rejected requests have no reported usage; they are not assigned
zero tokens. Across accepted scored responses, reported input was 19,996 and
output 1,675; including the probe, reported total is at least 21,732 tokens.
No cache usage was reported. No latency/success superiority claim is justified.

### What the final attempt actually did

The two accepted v5 responses reported 6,459/541 and 6,624/569 input/output
tokens. Both reported 512 reasoning tokens within their completion counts.
Tools requested were `list_files`, then two `read_file` calls.

Pactrail's normal compaction ran before the rejected third request: normalized
context fell from 50,168 to 29,078 bytes, with two observations stored as
artifacts. Its high-water mark was 22,937 bytes and target 18,636 bytes. The
remaining request still exceeded the provider allowance. This demonstrates a
limit of byte-based compaction with a large fixed prompt/schema footprint; it
does not demonstrate that compaction was absent or that the model cannot code.

The catalog's model context size and advertised combined TPM do not guarantee
that an account will accept a request that size. Actual errors enforced 7,000
input tokens per minute despite the 8,000-token combined header.

## Implementation and verification

1. Added a bounded, exact-model Groq pilot using the existing RSI supervisor,
   physical-request reservations, protected credentials and isolated execution.
2. Batched immutable artifact import into transactions, preserving FULL SQLite
   durability at commit instead of forcing one sync per imported file.
3. Fixed gateway admission to accept either strictly bound reasoning format:
   `reasoning_effort` or `{reasoning: {effort}}`. Mixed formats/extra settings
   remain refused. Groq-specific wire adaptation is only in the pilot.
4. Retained original requests, actual wire requests, raw responses/errors,
   allowlisted quota headers, usage, traces, grader logs and source snapshots.
5. Added post-run integrity analysis with explicit absent/partial/complete usage,
   exact response-model checks and SHA-256 evidence inventories.

Tests: all 35 existing RSI lab tests passed with real OCI execution enabled;
all 6 pilot/analysis tests passed. No tests were weakened. The full Rust suite
was not rerun for these Python-only changes, and these results are not a new
release qualification. Source isolation and trace checks passed on every
attempted runtime record; no completed receipt was available. No application
to the user's source tree occurred.

## Retention and reproducibility

Tracked frozen protocols and analyses:
`benchmarks/groq-qwen-pilot-v1/preregistered/20261012-v{2,3,4,5}-*`.
They include exact input, executable, image, source, grader-validation and
protocol hashes. Each original protocol is immutable.

Private local evidence is retained under
`benchmark-results/groq-qwen-20261012-v{1,2,3,4,5}/` in the dedicated RSI clone.
Each scored directory contains the journal/artifact database, raw output,
protocol, results, usage and analysis; `inventory.sha256.json` inventories the
retained files. v3 did not preserve its upstream error body, so its finer-grained
quota diagnosis remains unavailable. Later protocols retain bodies explicitly.

```sh
python3 -m unittest discover -s benchmarks/groq-qwen-pilot-v1 -v
PACTRAIL_LAB_TEST_IMAGE=sha256:39702c3eb9ce2cfc26c6db60dbc78d59f4f5bac240d55da39a441226b3ee9559 \
PACTRAIL_BENCH_BINARY=/absolute/path/to/qualified/pactrail \
python3 -m unittest discover -s experiments/self-improvement-v1/tests -v
python3 benchmarks/groq-qwen-pilot-v1/analyze.py /absolute/completed-output
```

Fresh inference requires a fresh output directory with validated base/gold
grader evidence. Use the README command, with an operator-confirmed Free key.
The existing full-catalog protocol is expected to encounter this account's
input ceiling; do not repeatedly rerun it hoping for a different quota.

## Decision and next experiment

**Free-provider feasibility: limited tool-use smoke PASS; completed-task quality
evaluation NOT QUALIFIED; causal RSI improvement NOT QUALIFIED.**

Retain the gateway compatibility fix and diagnostic infrastructure. No RSI
proposal is promoted from these results. Meaningful paid-model evaluation still
requires the frozen baseline-versus-candidate design and validated graders.

Before paying for a larger experiment, a useful separate engineering experiment
would measure a smaller, explicitly declared tool/context surface against the
full catalog. It must use supported engine configuration or a reviewed feature,
preserve tool semantics and authority, and receive a new preregistration. Do not
strip requests silently in the gateway. That work was not implemented or claimed
as a benefit in this pilot.
