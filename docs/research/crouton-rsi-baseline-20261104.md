# Crouton RSI-lab benchmark: results and improvement priorities

4–5 November 2026. Branch: `research/recursive-self-improvement-v1`.

## Simple explanation

Crouton can fix real code through Pactrail. The qualified v3 run produced a
Preact fix that passed the externally executed targeted test and all 31 declared
regression tests. Its receipt, trace and source-isolation checks passed. Only
`compat/src/render.js` changed: 19 added lines, no removed lines, no test or
manifest edits. Nothing was applied to the user's repository.

But the complete benchmark did **not** finish: the endpoint returned an upstream
rate-limit error. We retained all results and stopped rather than changing
providers, spending money or retrying the failed call.

The most useful agent-quality signal is excessive investigation: two completed
failures spent all 16 model turns without making an edit. The successful run
edited on turn 7 and completed in 10 turns. This suggests narrow experiments on
read selection, retained source context and progress assessment. It does not
prove that any proposed change will help.

We also found and fixed two actual **lab** weaknesses: a short transport deadline
and grading a sparse workspace as though it were a full repository. Those are
evaluation/reliability fixes, not measured improvements to the coding agent.

## What was frozen

| Item | Identity / setting |
| --- | --- |
| Exact model | `stealth/crouton-alpha` |
| Actual provider on accepted responses | `Stealth`, all 49 accepted v3 responses |
| Endpoint | `https://openrouter.ai/api/v1/chat/completions` |
| Engine/runner snapshot | `ab80c21b76a5a3e6bc7979a13209a1ba41e98f6c` |
| v3 protocol SHA-256 | `afbcccfcd6b900d323d89dff542937ee4a1bb3a64e0c87acba099999fd4830ad` |
| Pactrail executable SHA-256 | `efece82aa0c14f020254b4a87da396d3c3ff7fb6146054c46fab8aa89913d605` |
| Execution image | `sha256:39702c3eb9ce2cfc26c6db60dbc78d59f4f5bac240d55da39a441226b3ee9559` |
| Model settings | Temperature 0; reasoning effort low; buffered responses |
| Per-trial ceilings | 16 physical model attempts; 900 s; 320,000 engine-counted tokens |
| Context / output | 32,768 / 8,192 tokens |
| Gateway context reservation | Separately 131,072 conservative text-byte upper bound |
| Campaign ceilings | 192 attempts; four hours; reported tokens plus outstanding conservative reservations <=4M |
| Network | Four-second pacing; 240-second total provider limit; engine request limit 300 s |
| Trial order | Seed `20261104`; three repetitions per task |

The model catalog advertised zero input/output prices, tool support and
1,048,576-token context. Its maximum advertised completion size was not the
experimental output ceiling. [Official model page](https://openrouter.ai/stealth/crouton-alpha).
The anonymous provider's underlying model/checkpoint identity is unknown.
Temperature 0 does not make hosted inference deterministic.

## Task population and independent grading

Four existing public **development** issues were selected before scoring. No
held-out confirmation case was consumed. All gold/grader material remained
outside the model's execution container. Sealed source had a synthetic baseline
commit, with no original Git history or remote. Agent process execution was
disabled; tests ran externally in immutable, already-installed upstream images.

All bad/gold graders were executed and validated before v1 scoring. V2/v3
reused those unchanged, hash-bound reports and source/image identities.

| Task | Language | Known bad targeted failures | Reference targeted passes | Reference regression passes |
| --- | --- | --- | --- | --- |
| `psf-requests-6028` | Python | 2 | 2 | 185 |
| `vuejs-core-11915` | TypeScript | 2 | 2 | 43 |
| `preactjs-preact-3562` | JavaScript | 1 | 1 | 31 |
| `tokio-rs-tokio-6752` | Rust | 1 | 1 | 32 |

Strict success required externally executed targeted **and** regression passes,
a verified receipt/diff, ready-to-apply state, valid trace and unchanged source.
Model statements never supplied verification truth. V3 grades only the complete
baseline plus parent-verified receipt changes; no verified receipt means an
unknown functional result and a strict failure, not a guessed patch grade.

## v3 results: partial, not a completed twelve-trial comparison

| Frozen execution order | Trial | Outcome | External tests | Runtime |
| --- | --- | --- | --- | --- |
| 1 | requests / 3 | Turn limit; no edit | Not graded; no verified receipt | 106.311 s |
| 2 | Preact / 2 | **Strict success** | 1/1 targeted, 31/31 regression | 80.162 s |
| 3 | Preact / 1 | Turn limit; no edit | Not graded; no verified receipt | 94.697 s |
| 4 | requests / 2 | Upstream HTTP 429 | Not graded; interrupted | 77.316 s |
| 5–12 | Remaining declared repetitions | Unattempted after provider stop | Unavailable | Unavailable |

Thus: one verified fix, two completed failures, one provider interruption, eight
unattempted trials. The Rust and TypeScript tasks were not reached. Do not report
this as 1/12 model accuracy, a four-language quality result or a statistically
credible estimate of general coding ability. Even the resolved runs cover only
two public issues. No task-clustered improvement interval can be estimated from
this baseline: there is no matched child/control experiment.

## Rate limit and cost evidence

V3 made 50 physical requests. Forty-nine returned HTTP 200 with the exact model;
the fiftieth returned HTTP 429. The retained error identified:

```text
limit_source: upstream_provider_shared_pool
provider_name: Stealth
stealth/crouton-alpha is temporarily rate-limited upstream.
```

The account's free-model daily counter stayed at **0 used / 50 remaining** before
and after. This establishes that an unchanged daily counter is compatible with
an upstream rate limit. It does not establish unlimited service. No retry-after
header or precise reset time was reported. No quota bypass, alternate provider,
paid fallback, account upgrade or uncertain-call replay was used.

| Resource | Reported result | Coverage |
| --- | --- | --- |
| Input tokens | 538,149 | 49/50 requests; partial |
| Output tokens | 1,479 | 49/50; partial |
| Cached input | 0 | Explicitly reported on 49/50 |
| Reasoning within output | 202 | 49/50; partial |
| Reported inference cost | $0 | 49/50; rejected request has no cost report |
| Tool actions | 62 | Four retained traces |
| Tool failures | 0 | Four retained traces |
| Agent process calls | 0 | Disabled; all four traces checked |
| Aggregate runtime | 358.486 s | Four runtime records; excludes preparation/external grading |
| Provider round trips with responses | 328.470 s | Includes network/queue; not GPU-only time |
| GPU compute, peak memory, invoice | Unavailable | No estimate substituted |

Unknown rejected-call usage remains reserved conservatively. No zero-token or
zero-cost observation was invented for it. Reported zero cache usage does not
prove that a different provider, future run or prompt strategy cannot cache.
The earlier Nemotron settings differed, so this is not a matched model comparison.

## What to improve, in priority order

### 1. Preserve useful source context instead of repeatedly reacquiring it

Both turn-exhausted runs compacted context twice; the successful run compacted
zero times. Requests / 3 reduced one normalized context from 139,583 to 43,510
bytes. The four attempted runs used six `read_observation` calls to recover
artifact content. There were no exact duplicate tool requests in v3, so these
facts are **not** proof that all repeated reading was redundant or that
compaction caused failure.

The current batch-read tool can return up to 1 MiB, while this experiment's
model-context high-water mark was 78,643 bytes. The engine correctly bounds
context and retains artifacts, but a successful bulk read can still be too
large to remain directly usable in the next request.

**Cheapest falsifiable experiment:** compare the current read surface with
context-budget-aware source ranges/chunks and a small pinned set of current
file/symbol observations. Keep original bytes, revisions, digests and artifact
access. Measure strict success, recovery reads, tool-result bytes, input tokens
and first successful edit turn. Keep the mechanism only if the matched result
improves without reducing correctness or authority guarantees.

Relevant code: `crates/pactrail-tools/src/advanced.rs`,
`crates/pactrail-tools/src/observation.rs`,
`crates/pactrail-engine/src/context_window.rs`.

### 2. Distinguish new observations from progress toward a fix

Of 62 tool actions, 60 were reading/navigation, one inspected workspace changes,
and one edited. Both completed failures reached 16 turns with no candidate.
The successful run edited on turn 7 and finished on turn 10.

Pactrail **already has** phases, progress assessment, action-deadline prompts and
steering. Adding another generic reminder would duplicate existing machinery.
The current controller resets its no-progress count when a tool result has a
new content digest. A new observation can be genuine evidence without bringing
the run closer to implementing the requested change.

**Experiment:** augment the existing assessment with separately recorded
read-only turns since the last candidate change and unresolved task obligations.
After a bounded investigation period, request a concrete next implementation
step or a precise missing fact/blocker. Do not force speculative edits, mark
obligations verified by model claim, remove read authority or just raise turn
limits. Measure first-edit latency, no-edit exhaustion, strict success and patch
quality under the same global budget.

Relevant code: `crates/pactrail-engine/src/controller.rs` and `engine.rs`.

### 3. Make evaluation and provider failure states explicit

The benchmark exposed two genuine lab faults and one real provider limitation.
They must not all become “model failed.” Keep distinct outcomes for turn
exhaustion, provider quota, uncertain transport, invalid grading infrastructure,
missing receipt, failed targeted tests and failed regression tests.

The implemented fixes below improve the trustworthiness of the research loop.
They do not establish coding-quality gains. For future provider resilience,
experiment with an explicitly preregistered bounded-backoff policy only after
defining physical-request accounting and uncertain-call semantics. Do not
silently retry or call this endpoint unlimited.

### What this experiment does not justify

No neural latent backend, larger agent swarm, new vector database, mandatory
LSP, arbitrary larger turn quota or new edit protocol is justified by these
results. V3 had zero failed tools and no output-length failures. Earlier invalid
attempts exposed an artifact-read bound error, but that is diagnostic evidence,
not a reason to claim that editing is the primary v3 bottleneck.

## Repairs actually implemented

1. Reused the existing free-only runner with a strict explicit Crouton profile;
   exact model, zero-price routing, finite budgets, retained request/response
   artifacts and no silent fallback.
2. Added an optional bounded transport timeout. Default remains 120 s; the
   preregistered profile permits 240 s, with relay/engine/trial limits enclosing
   it. A timed-out trial fails; future independently declared trials may proceed
   only under explicit policy. Unknown calls retain full token reservations.
3. Force-tracked and checked the synthetic baseline index against all admitted
   files. This improved source admission but was **not sufficient** to fix the
   sparse-workspace grading problem; the retained failing test proves that.
4. Reconstructed external candidates from the immutable full baseline plus only
   frozen-parent-verified receipt/diff changes. Ignored untouched files survive;
   genuine deletions remain. Raw sparse workspaces stay retained separately.
5. Journal-bound baseline/raw/projected/receipt identities, and audited actual
   grading bytes and changed paths against that projection. Added a regression
   that deliberately deletes an unrelated lockfile and requires rejection.
6. Preserved missing usage, pricing, provider and error metadata as unavailable
   rather than zero; added analysis coverage for real HTTP 429 responses.

No Rust engine/provider implementation, authority, process grant, apply policy,
model weights or production memory behavior was changed. The agent-quality
mechanisms above remain proposals for controlled experiments, not shipped gains.

## Every experiment remains retained

| Protocol | Outcome | Use in quality claims |
| --- | --- | --- |
| v1, snapshot `eeb6ea1` | Two attempted; one turn exhaustion, one 120-second total timeout; ten unattempted | NOT QUALIFIED |
| v2, snapshot `598126c` | Five attempted; stopped by operator after discovering unrelated lockfile deletions in grader patches; seven unattempted | INVALID; two apparent successes are not qualified |
| v3, snapshot `ab80c21` | Four attempted; one verified fix, two turn exhaustion, one upstream quota interruption; eight unattempted | Valid partial observations; complete four-task benchmark NOT QUALIFIED |

No failed run or invalid experiment was deleted. Model/tasks/order/engine budgets
were preserved across the infrastructure repairs. V3's grading reconstruction
and ungraded incomplete-run policy were frozen before its scored requests.

Across all three attempts, 135 accepted model responses remain retained, plus
the transport/quota/invalidated-call records. Only v3 supplies the partial
qualification counts above; earlier apparent successes are not pooled into them.

## Tests, security and reproducibility

Local suites passed: 39 RSI-lab tests with real OCI/engine execution, 31 Harness
Lab tests, 10 agent-runtime tests, 12 shared pilot tests, 8 Crouton profile/audit
tests, 6 Groq pilot/analysis tests. Total: **106**. The finite smoke fuzz campaign
used seed `20261105` and 2,500 iterations, with no unresolved crash. It covers
bounded JSON/archive behavior, not formal security or the whole Rust engine.
The ordinary full Rust qualification was not rerun for these Python changes.
This report is not a release qualification or claim that pending CI passed.

Source isolation and trace checks passed on all four attempted v3 runs. One
valid receipt/ready candidate exists; the other three lack a verified completed
receipt. The parent validates receipts, diffs and traces; model claims are not
authority. Tests, receipts and projection integrity constrain this evaluation,
but do not independently attest a compromised trusted host/operator.

Frozen protocols, analysis and evidence hashes:
`benchmarks/crouton-rsi-v1/preregistered/`.
Raw evidence is retained at:

```text
/home/akmessi/pactrail-crouton-20261104/benchmark-results/crouton-rsi-20261104-v{1,2,3}/
```

Each directory retains its journal/artifacts, source snapshots, protocol,
results, model requests/responses, candidates, traces, receipts when available,
grader logs and SHA-256 inventory. Credential scans check actual configured key
bytes without printing or committing them. The original workspace is read-only;
work is in a separate checkout, with no main merge, release or source apply.

```sh
python3 -m unittest discover -s benchmarks/crouton-rsi-v1 -v
python3 -m unittest discover -s benchmarks/nemotron-rsi-pilot-v1 -v
python3 experiments/self-improvement-v1/fuzz.py --seed 20261105 --iterations 2500
python3 benchmarks/crouton-rsi-v1/analyze.py /absolute/completed-v3-output
```

The complete inference command and prerequisites are in the benchmark README.
Do not resume the stopped experiment or rerun only failures. A future independent
campaign must recheck exact model availability/zero prices, declare a new
protocol, preserve old evidence, and count every outcome. Confirmation requires
fresh disjoint cases and a matched parent/child/restored-parent comparison.

**Conclusion:** useful real-model and lab-reliability evidence was obtained,
including a fully tested code fix. Unlimited service, four-language benchmark
completion and causal RSI improvement were not established. The next quality
experiment should target context/read selection and progress toward implementation.
