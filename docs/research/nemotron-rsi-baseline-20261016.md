# First completed Nemotron RSI-lab baseline

16 October 2026 — research branch `research/recursive-self-improvement-v1`.

## What happened, in simple words

We ran six real coding attempts through Pactrail's Self-Improvement Lab using
the exact free OpenRouter model `nvidia/nemotron-3.5-super-vl:free`.
**One attempt fixed a real Vue bug and passed independently run tests. Five
attempts failed.** All six were graded; none was dropped or selectively rerun.

The successful candidate changed one line in
`packages/compiler-core/src/tokenizer.ts`, passed both targeted tests and all
43 declared regression tests, and left a valid receipt and reviewable candidate.
The source repository stayed untouched. No apply or automatic promotion occurred.

This establishes a working free route for small real-model lab experiments.
It also gives the improvement loop concrete weaknesses to investigate. It does
**not** show that Pactrail has improved itself: this was the unchanged parent
baseline, with no child implementation or parent/child/restored-parent comparison.

## Frozen identity

| Item | Value |
| --- | --- |
| Experiment | `nemotron-rsi-baseline-20261016-v1` |
| Runner/harness snapshot commit | `a90dca5a5b742823025b31c2e71638c219651a95` |
| Model | `nvidia/nemotron-3.5-super-vl:free` |
| Provider returned | `Nvidia`, all 61 responses |
| Endpoint | `https://openrouter.ai/api/v1/chat/completions` |
| Protocol SHA-256 | `8e0c843dd55dec7216e92dadfb885e18a7c142c05f3076e48ceb442975a4f5b3` |
| Release executable SHA-256 | `efece82aa0c14f020254b4a87da396d3c3ff7fb6146054c46fab8aa89913d605` |
| Execution image | `sha256:39702c3eb9ce2cfc26c6db60dbc78d59f4f5bac240d55da39a441226b3ee9559` |
| Repetitions and order | Three per task; seed `20261016` |
| Model settings | Temperature 0; reasoning effort low; buffered responses |
| Trial ceilings | 12 model attempts; 900 seconds; 240,000 engine-counted tokens |
| Engine context/output | 32,768 / 4,096 tokens |
| Gateway reservation | Separately 131,072 conservative text-byte/token upper bound |
| Campaign ceilings | 72 requests, 2 hours, reported tokens + next reservation <=2M |

Catalog metadata advertised 1,000,000 context and 131,072 maximum completion
tokens; the experimental settings above were deliberately smaller. These results
characterize those settings, not the model's best achievable performance.
The ordinary Rust engine/binary was unchanged. Added code is benchmark transport,
preregistration and evidence analysis; it grants no new engine authority.

## Grader validation and isolation

Both issues are public historical **development** cases. They were already
inspected during Harness Lab development and cannot count as independently
held-out confirmation. The original upstream source/commit, preparation,
reference fix, parser, grader and installed image identities are hash-bound
by the protocol and prepared task records.

Fresh external container validation preceded scoring:

| Issue | Known bad source | Reference fix | Regression tests |
| --- | --- | --- | --- |
| `psf-requests-6028` | 2 targeted failures | 2 targeted passes | 185 passes |
| `vuejs-core-11915` | 2 targeted failures | 2 targeted passes | 43 passes |

The agent received a sealed source snapshot and public issue description, with
no original history, remote, gold patch or hidden grader. Source mounts were
read-only; candidate work used the normal Tool Kernel. Agent process execution
was disabled. External graders ran separately in immutable upstream containers.
Model networking was unavailable except through the trusted credential gateway.

The gateway required the exact free model, restricted provider prices to zero,
disabled provider fallback, bounded I/O and retained each response. It would
stop the campaign on provider, quota, identity, transport or admission errors.
Normal engine output-recovery turns remained within the same charged budgets;
no provider-error retries or model substitutions occurred.

## All declared results

Rows below follow frozen execution order. Targeted/regression entries are
independently executed passing-test counts; all required coverage was present.

| Issue / repetition | Targeted | Regression | Strict completion | Observable stopping condition | Engine wall time |
| --- | --- | --- | --- | --- | --- |
| requests / 1 | 0/2 | 185/185 | FAIL | Repeated 4,096-token output exhaustion | 197.4 s |
| Vue / 3 | 2/2 | 43/43 | PASS | Candidate ready for review | 135.0 s |
| requests / 3 | 0/2 | 185/185 | FAIL | 12 model-turn limit | 166.0 s |
| Vue / 1 | 0/2 | 43/43 | FAIL | 251,201 tokens exceeded the 240,000 cap | 88.7 s |
| Vue / 2 | 0/2 | 43/43 | FAIL | Three consecutive tool turns failed | 73.4 s |
| requests / 2 | 0/2 | 185/185 | FAIL | Repeated 4,096-token output exhaustion | 146.5 s |

**Strict: 1/6; functional: 1/6.** By task: requests 0/3, Vue 1/3.
The five failed candidates had empty externally generated patches, so their
regression passes do not establish useful implementation progress. Failed runs
without receipts are not counted as valid-receipt successes.

These six repeated runs involve only two tasks. Treat the counts as a small
diagnostic, with substantial uncertainty. Repetitions are not six independent
repository problems. No general success-rate estimate, significance claim,
cross-harness ranking or causal improvement claim is supported.

## Resources and free access

All 61 model requests returned HTTP 200 and the exact requested model. Every
response reported inference cost **0**; no paid route or account upgrade was used.
Reported cost is provider evidence, not independently verified invoice data.

| Metric | Measured result | Coverage |
| --- | --- | --- |
| Input tokens | 1,100,778 | 61/61 requests |
| Output tokens, including reported reasoning | 56,606 | 61/61 |
| Cached input tokens | 663,408 | 61/61 |
| Uncached input, derived by subtraction | 437,370 | 61/61 |
| Cache fraction, derived | 60.3% of input | Same reported-token basis |
| Reasoning tokens within output | 50,822 | 61/61 |
| Cache-write tokens | 0 | 61/61, actually reported |
| Tool actions | 81 | All six traces |
| Read/navigation actions | 74 | All six traces |
| Write attempts | 7 | 6 `apply_patch`, 1 `replace_text` |
| Failed tool actions | 19 | 13 reads, 6 patches |
| Agent process calls | 0 | Explicitly disabled; all traces checked |
| Total engine wall time | 807.014 s | Sum of six runtime records |
| Provider round-trip time | 695.950 s | Includes provider queue/network, not GPU-only time |
| GPU/model compute time, peak memory, invoice | Unavailable | No value invented |

The user key's daily free-model counter was **0 used / 50 remaining before**
and **20 used / 30 remaining after**, although 61 requests succeeded. Thus an
unchanged counter after one probe was not proof of unlimited service. The
different accounting is observed but unexplained; exemption or delayed/other
counting semantics must not be assumed. We did not deliberately exhaust the
quota or bypass a limit. Access can still change or become rate-limited.

## What the traces suggest testing next

1. **Edit interface/tool selection.** Every attempted strict unified patch
   failed. The successful trial recovered using the existing `replace_text`
   tool. Test clearer tool selection/instructions for small exact replacements
   before adding another edit system. Preserve strict patch mismatch rejection.
2. **Read-range usability.** Thirteen read calls failed with invalid ranges,
   including `409..=120`. Inspect whether clearer schemas/examples prevent
   interpreting an absolute `end_line` as a line count. Do not silently repair
   invalid ranges or relax path/capability rules.
3. **Reasoning/output allocation.** About 90% of reported output was reasoning;
   six responses ended with `length`. Two trials ultimately stopped at that
   output ceiling. Compare explicitly declared provider reasoning/output
   strategies with matched resource caps; simply spending more is not proof of
   a better harness.
4. **Loop detection.** One trial stalled after repeated failed tools. Test
   evidence-grounded recovery guidance within existing budgets rather than
   unbounded retries.

These are hypotheses grounded in observable events. Failure labels were derived
descriptively from logs after scoring; primary success criteria, tasks, budgets
and graders stayed frozen. No hypothesis was implemented, promoted or declared
effective from these results.

## Authority and evidence audit

| Check | Result |
| --- | --- |
| Source isolation | PASS, 6/6 |
| Trace integrity | PASS, 6/6 |
| Valid receipt and ready-to-apply candidate | PASS for the one successful run; absent/failed for five incomplete runs |
| Successful candidate scope | One production source line changed; no tests/manifests changed |
| Failed patch effects | Rejected without accepting mismatched hunks |
| External targeted/regression grading | Complete, 6/6 |
| Exact model identity | PASS, 61/61 responses |
| Reported zero cost | PASS, 61/61; invoice unverified |
| Automatic source apply/promotion | None |
| RSI causal improvement / independent held-out qualification | NOT QUALIFIED |

Post-run analysis checks the protocol against its journal-bound artifact, every
content-addressed object, trial results against durable result events, usage
exports against reservations, and accepted response model identities. A SHA-256
inventory binds retained evidence. These integrity checks do not constitute
independent attestation against a malicious operator or trusted host.

## Tests and retention

Local verification passed: **35 RSI-lab tests with actual OCI execution, 31
Harness Lab tests, 10 agent-runtime tests, and 12 new pilot/analysis tests**.
No Rust engine changes were made, and the full Rust suite was not rerun for
these Python benchmark changes. This is not a release qualification.

Frozen protocol and its hash:
`benchmarks/nemotron-rsi-pilot-v1/preregistered/20261016-v1-protocol.*`.
Sanitized analysis and evidence identities are retained alongside it.
Private raw evidence lives in the dedicated RSI clone at
`benchmark-results/nemotron-rsi-20261016-v1/`: journal and artifact blobs, protocol,
requests/responses, run traces, receipts, candidates, diffs, grader validation,
test logs, raw results, account counters and `inventory.sha256.json`.
Credentials and credential environment dumps are excluded.

```sh
python3 -m unittest discover -s benchmarks/nemotron-rsi-pilot-v1 -v
python3 benchmarks/nemotron-rsi-pilot-v1/analyze.py /absolute/completed-output
```

To repeat a fresh baseline, validate both base/gold graders into a new output
directory, check exact model availability/zero prices, and use the runner command
in the benchmark README. The runner refuses an existing scored protocol. Never
reuse these development cases as held-out confirmation for a tuned child.

## Decision

Keep this free-only benchmark path and its evidence audit. A paid key is not
required for the next small experiment while this exact route remains available.
Keep finite budgets and stop-on-provider-failure behavior.

Next, preregister one narrow improvement and a matched parent/child/restored-parent
development comparison. Use fresh disjoint confirmation cases and frozen
engineering/authority/recovery/mechanism gates before accepting any child.
The present results authorize no performance claim, automatic merge or promotion.
