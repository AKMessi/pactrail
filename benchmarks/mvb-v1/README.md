# Pactrail minimum viable benchmark v1

This suite is a small, deterministic model-in-the-loop evaluation of Pactrail's
core workflow. It is designed to catch integration failures and establish a
reproducible baseline. It is not a substitute for SWE-bench, a security audit,
or statistically powered model evaluation.

## What it measures

Seven one-shot cases cover exact file creation, a targeted edit, synchronized
multi-file changes, localized code repair, deletion, write-scope defense, and
read-only repository understanding. Every case is run at temperature `0.0`
(enforced by Pactrail v0.1.0) and is graded from observable artifacts:

- the source workspace must remain byte-identical before explicit apply;
- expected file contents and the complete changed-path set must match;
- read-only answers must contain the required factual terms;
- Pactrail must accept the run's hash-chained portable trace.

The cases and expected outputs are public in [`cases.json`](cases.json). No
LLM-as-judge is used. Failed runs remain failures; the runner does not retry or
select a favorable sample.

## Run it

Start any OpenAI-compatible local model server, then run from the repository
root with PowerShell 5.1 or newer:

```powershell
$env:PACTRAIL_LOCAL_API_KEY = 'local'

./benchmarks/mvb-v1/run.ps1 `
  -Model 'your-model-alias' `
  -BaseUrl 'http://127.0.0.1:8080/v1' `
  -OutputDirectory './benchmark-results'
```

Useful controls:

```powershell
# Three deterministic repetitions of two selected cases.
./benchmarks/mvb-v1/run.ps1 `
  -Model 'your-model-alias' `
  -BaseUrl 'http://127.0.0.1:8080/v1' `
  -Repetitions 3 `
  -CaseId exact-file-create,localized-bug-repair
```

The runner creates a fresh workspace for every case. Results include an
aggregate `summary.json`, a human-readable `SUMMARY.md`, and per-case raw CLI
output, receipt, trace, integrity-check rendering, assertions, timing, token
counts, and model/tool-call counts.

## How to interpret results

This is an end-to-end agent score, so model quality and harness compatibility
both affect task success. Transaction isolation and trace-integrity assertions
are harness properties; code and repository-understanding assertions also
depend on the selected model. Always publish the model file, quantization,
runtime build, hardware, context, output limit, turn limit, repetitions, and all
failures alongside the score.

The suite intentionally contains tasks a 230M model may fail. A tiny model is a
useful protocol and recovery stress test, not a credible coding-quality
baseline. Larger public claims should use multiple coding models and established
external benchmarks in addition to this suite.

## Published baseline

The first complete local baseline, including all strict failures and raw
integrity-checked artifacts, is available in the
[`2026-07-17 Windows ARM64 report`](../results/2026-07-17-windows-arm64/README.md).
