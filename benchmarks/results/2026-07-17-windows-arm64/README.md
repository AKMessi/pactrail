# Pactrail MVB v1 — Windows ARM64 baseline

Run date: 2026-07-17 UTC

Suite: [`pactrail-mvb-v1`](../../mvb-v1/README.md)

Pactrail: `0.1.0`, built from the source revision containing this report

This is a minimum viable, model-in-the-loop integration benchmark. It is a
reproducible baseline for Pactrail's transaction, tool, model, and trace path;
it is not SWE-bench, a statistically powered model evaluation, or proof that
Pactrail outperforms another coding agent.

## Headline results

| Model | Strict task score | Median task time | Model/tool calls | Failed tools | Recovery stops | Isolation | Trace integrity |
|---|---:|---:|---:|---:|---:|---:|---:|
| Qwopus3.5 9B Coder Q3_K_M | **6/7 (85.7%)** | 209.05 s | 30 / 25 | 0 | 0 | 7/7 | 7/7 |
| LFM2.5 230M Fable-5 F16 | **1/7 (14.3%)** | 8.18 s | 22 / 21 | 7 | 6 | 7/7 | 7/7 |

Across all 14 runs, Pactrail kept the source workspace byte-identical before
explicit apply in **14/14** cases and its integrity checker accepted the
hash-chained trace in **14/14** cases. Task success remains model-dependent, as
the difference between the 9B and 230M results shows.

The Qwopus matrix consumed 1,482.65 seconds end to end. Model invocations
accounted for 1,481.91 seconds; Pactrail tool execution accounted for 101 ms.
The LFM matrix consumed 63.96 seconds, of which 63.35 seconds were model time
and 73 ms were tool time. These figures show that local CPU inference dominated
latency in this environment.

## Per-case results

| Case | Qwopus 9B | LFM 230M |
|---|---:|---:|
| Exact file creation | Pass | Pass |
| Targeted config edit | Pass | Fail |
| Three-file version synchronization | Pass | Fail |
| Localized Rust bug repair | Pass | Fail |
| Requested deletion without collateral change | Pass | Fail |
| Write-scope defense | Pass | Fail |
| Read-only repository summary | **Fail** | Fail |

The strict Qwopus failure deserves context. Its answer was:

> This directory contains a tiny arithmetic library that provides two
> operations: addition and subtraction.

That answer is semantically correct and made no edits, but the public grader
required the literal term `calculator`. The result remains a failure. The
grader was not relaxed after inspecting the answer.

## Method

- Seven public cases, one attempt per case, no case-level retries or sample
  selection.
- Pactrail temperature `0.0`, context `8,192`, maximum output `512`, and maximum
  model turns `12` for both models.
- Native process permission disabled. Exact file contents, absent paths,
  complete changed-path sets, source isolation, required summary terms, and
  trace integrity were graded deterministically without an LLM judge.
- A candidate was applied only after the benchmark confirmed that the original
  workspace remained unchanged. Failed or missing candidates were not repaired
  manually.
- Raw stdout, receipts, portable JSONL traces, integrity-check renderings,
  per-assertion results, timings, and token counts are retained under the model
  directories linked below.

An initial 4K Qwopus preflight was discarded after a multi-file turn reached
4,181 tokens and the runtime correctly rejected it. Both official matrices were
then run at 8K. The preflight was not scored or included in the raw results.

## Environment

| Component | Value |
|---|---|
| Machine | Acer Swift SFG14-01 |
| CPU | Snapdragon X Plus X1P42100, 8 cores / 8 logical processors |
| RAM | 16,759,111,680 bytes |
| OS | Windows 11 Home Single Language, ARM64, build 26200 |
| Rust | `rustc 1.95.0`, native `aarch64-pc-windows-msvc` |
| Pactrail controls | 8K context, 512 output, 12 turns, temperature 0, processes disabled |

### Qwopus runtime

- Model: [`Jackrong/Qwopus3.5-9B-Coder-GGUF`](https://huggingface.co/Jackrong/Qwopus3.5-9B-Coder-GGUF/blob/main/Qwopus3.5-9B-coder-Exp-Q3_K_M.gguf)
- File: `Qwopus3.5-9B-coder-Exp-Q3_K_M.gguf`
- Size: `4,623,526,880` bytes
- SHA-256: `d652b6a26842ead8ebe8f27b9b77a1c66e400e096615900d5efa849eea546862`
- Runtime: Prism llama.cpp `9591` (`62061f910`), Windows ARM64 CPU build
- Server controls: CPU-only, 8 threads, one slot, reasoning off, 512 MiB prompt
  cache

```powershell
llama-server.exe `
  -m $env:QWOPUS_MODEL `
  --alias qwopus3.5-9b-coder-q3km `
  --host 127.0.0.1 --port 18081 `
  -c 8192 -t 8 -tb 8 -ngl 0 -np 1 `
  --jinja --reasoning off --metrics --cache-ram 512
```

### LFM runtime

- Model: [`AKMESSI/lfm2.5-230m-fable-5`](https://huggingface.co/AKMESSI/lfm2.5-230m-fable-5/blob/main/lfm2.5-230m-fable-5-f16.gguf)
- File: `lfm2.5-230m-fable-5-f16.gguf`
- Size: `461,883,712` bytes
- SHA-256: `1e1624de4d7ebe413d82dbe2b5f48fcf83e6c46b1b5414c8aef0596009380cc6`
- Runtime: stock llama.cpp `7786` (`5bd341c9a`), Windows x86_64 CPU
  build under Windows ARM emulation
- Server controls: CPU-only, 8 threads, operation offload disabled, prompt cache
  disabled

```powershell
llama-server.exe `
  -m $env:LFM_MODEL `
  --alias lfm-230m `
  --host 127.0.0.1 --port 18080 `
  -c 8192 -t 8 -tb 8 -ngl 0 `
  --no-op-offload --jinja --metrics --cache-ram 0
```

The two runtimes differ because the native Prism build produced invalid empty
tool paths for this LFM file, while the stock build was compatible. Conversely,
native ARM64 was required to avoid a severe emulation penalty for Qwopus. Task
scores use identical Pactrail controls, but cross-model speed comparisons should
not be interpreted as model-only performance.

The stock x86_64 Vulkan backend was also tested and crashed on the Snapdragon
Adreno translation layer with `ErrorOutOfHostMemory`, including for the 230M
model. Official runs were therefore CPU-only. That runtime failure was excluded
from model task scores and is disclosed here rather than silently omitted.

## Raw evidence

- [Qwopus 9B summary and artifacts](20260717T012212Z-qwopus3.5-9b-coder-q3km/)
- [LFM 230M summary and artifacts](20260717T012033Z-lfm-230m/)

Each case directory contains `result.json`, `run-output.json`, the original
`trace.jsonl`, and `trace-render.json` produced by Pactrail's
integrity-validating trace command. Apply output and `receipt.json` are included
when the run produced an applicable candidate.

## Reproduce

After starting the appropriate server, run:

```powershell
$env:PACTRAIL_LOCAL_API_KEY = 'local'

./benchmarks/mvb-v1/run.ps1 `
  -Pactrail ./target/release/pactrail.exe `
  -Model qwopus3.5-9b-coder-q3km `
  -BaseUrl http://127.0.0.1:18081/v1 `
  -ContextTokens 8192 `
  -MaxOutputTokens 512 `
  -MaxTurns 12
```

The runner exits `0` only when every strict case passes and exits `2` when one
or more cases fail. It still writes complete artifacts on a scored failure.

## Limitations and next benchmark

- Seven synthetic tasks and one repetition are enough for integration evidence,
  not general capability claims or confidence intervals.
- Runtime-specific chat templates and CPU implementations affect results.
- Process execution was disabled, so this suite uses deterministic external
  assertions rather than model-invoked compilation or tests.
- There is no comparison with another agent harness in this result.
- Repository scale, long-context retrieval, baseline drift, malicious tool
  payloads, and established agent benchmarks remain future evaluation work.

The next credible step is a multi-seed run on at least two stronger coding
models, followed by SWE-bench Verified or an equivalent patch-level benchmark
with cost, latency, regression, and policy-violation reporting.
