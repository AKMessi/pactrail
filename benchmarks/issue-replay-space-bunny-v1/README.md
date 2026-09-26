# Space Bunny architecture regression

This is a preregistered, exploratory comparison of Pactrail's architecture
branch and OpenCode 1.18.32 on three known Rust defects. The earlier DeepSeek
run exposed Pactrail's investigation-without-editing failure on these same
cases. These tasks are development regressions, not held-out evidence of broad
coding-agent superiority.

[`protocol.json`](protocol.json) freezes the Pactrail and comparator binaries,
runner, suite, config, graders, controls, execution order, scoring, and stopping
rule before the first model request. Every declared case is pass@1, including
failures and timeouts. Both harnesses use OpenRouter's
`stealth/space-bunny-alpha` with identical context, output, temperature,
thinking, step, process, and offline-workspace controls. The model was listed as
free when the protocol was frozen; the runner still requires a declared
estimated-spend cap. The API key must be supplied through
`OPENROUTER_API_KEY`; it is never committed.

The [first scored trial](../results/2026-09-26-space-bunny-first-trial/README.md)
failed on a repeated streamed finish marker before any completed model turn.
That outcome remains published. [`protocol-v2.json`](protocol-v2.json) freezes
the corrected adapter binary at `508eaba` as a separate revision; its outcomes
must not replace or be pooled silently with the first protocol's outcomes.

The [complete revision 2 comparison](../results/2026-09-26-space-bunny-v2/README.md)
scored 0/3 for both harnesses. [`protocol-v3.json`](protocol-v3.json) freezes
the new action-deadline controller and corrected runner at `d4d9579` before
another model trial. Its results are a separate engineering iteration.

First validate the gold graders with `-ValidateGraders` using
`benchmarks/issue-replay-v1/run.ps1`, this directory's `cases.json`, and
`benchmarks/issue-replay-v1/opencode-openrouter-space-bunny.json`. Then run the
six declared harness/case pairs in the exact protocol order. Pass the same
provider controls to every scored invocation:

```text
-Model stealth/space-bunny-alpha
-ApiKeyEnv OPENROUTER_API_KEY
-ApiBaseUrl https://openrouter.ai/api/v1
-OpenCodeProvider openrouter-direct
-InputPriceUsdPerMillion 0
-CachedPriceUsdPerMillion 0
-OutputPriceUsdPerMillion 0
-MaxEstimatedSpendUsd 1
```

For a complete run, retain every per-case raw result, normalize the outcomes,
and publish the actual reported tokens, cache hits, elapsed time, tool calls,
source isolation, trace integrity, and hidden-test results. A smaller token or
time total counts as an efficiency improvement only alongside successful task
completion.
