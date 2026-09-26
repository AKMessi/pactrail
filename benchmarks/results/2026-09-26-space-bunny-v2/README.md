# Space Bunny architecture regression, revision 2

This is the complete six-trial pass@1 run frozen by
[`protocol-v2.json`](../../issue-replay-space-bunny-v1/protocol-v2.json).
All three known Rust defects were attempted once by Pactrail at `508eaba`
and once by OpenCode 1.18.32 with the same free OpenRouter model and declared
limits. Every failure is retained in `raw/`; `comparison.json` is the
machine-readable comparison and `SHA256SUMS` covers the selected raw files.

| Harness | Functional | Strict | Reported tokens | Model calls | Tool calls | Agent time |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Pactrail | 0/3 | 0/3 | 581,389 | 48 | 65 | 178.22 s |
| OpenCode | 0/3 | 0/3 | 538,239 | 52 | 56 | 434.42 s |

Neither harness edited production source or passed a hidden behavioral
grader. Pactrail reached its 16-turn limit on all three tasks after successful
read/search calls. OpenCode returned completion text after its step limit,
also without editing. Pactrail preserved source isolation and verified its
portable trace in all three trials. Its lower wall time does not count as a
task-efficiency win when both harnesses completed 0/3 tasks; Pactrail also
used more reported tokens.

The runner's raw `provider_errors` field treats every nonzero Pactrail exit as
a provider error. The traces and stderr show these three exits were turn-limit
failures, not provider transport failures. The runner was corrected after this
frozen run. `Cargo.lock` appears as a changed path in two Pactrail results
because the benchmark snapshot includes the generated lockfile while
Pactrail's candidate omits it under those repositories' `.gitignore` rules;
no mutation tool was called. Both measurement limits are retained in the raw
records rather than silently rewritten.

This suite measures a known regression and cannot establish broad coding-agent
superiority. It shows a remaining action-loop problem with this model: new
read results kept satisfying the novelty detector while the candidate stayed
empty. A later controller revision is evaluated under a new frozen protocol.
