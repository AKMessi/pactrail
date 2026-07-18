# Local model evaluation: Gemma4 v2 Q4_K_M vs Ternary Bonsai 27B Q2_0

This report is a one-repetition, model-in-the-loop evaluation of Pactrail v0.1.0
on a 16 GB Windows ARM64 laptop. It retains every strict failure, isolated
candidate, raw CLI result, receipt, and hash-chained trace. No LLM judge, case
retry, or favorable-sample selection was used.

The headline is straightforward: **Ternary Bonsai produced much better agent
behavior and exact edit artifacts; Gemma was substantially faster on this CPU.**
Bonsai passed every edit task but missed one required word in the read-only
summary. Gemma relied heavily on recovery and failed one edit artifact outright.

## Results

| Metric | Gemma4 v2 Q4_K_M | Ternary Bonsai 27B Q2_0 |
|---|---:|---:|
| Strict MVB score | **4/7 (57.1%)** | **6/7 (85.7%)** |
| Exact candidate artifacts | 6/7 (85.7%) | 7/7 (100%) |
| Edit tasks passed strictly | 4/6 | **6/6** |
| Source isolation verified | 7/7 | 7/7 |
| Trace integrity verified | 7/7 | 7/7 |
| Median end-to-end task time | **419.8 s (7.0 min)** | 728.9 s (12.1 min) |
| Total scored task time | **2,634.1 s (43.9 min)** | 4,897.0 s (81.6 min) |
| Model calls | 73 | **23** |
| Tool calls | 76 | **24** |
| Failed tool calls | 20/76 (26.3%) | **1/24 (4.2%)** |
| Recovery stops | 5 | **0** |
| Total reported tokens | 209,385 | **77,919** |

“Exact candidate artifacts” is the suite's narrower transaction diagnostic. It
checks candidate files and the complete changed-path set before apply. On the
read-only case it means the workspace stayed unchanged; summary semantics are
scored only by the strict result. It never converts a strict failure into a pass.

## Case-by-case score

| Case | Gemma strict | Gemma candidate | Bonsai strict | Bonsai candidate |
|---|---:|---:|---:|---:|
| Exact file creation | pass | exact | pass | exact |
| Targeted config edit | pass | exact | pass | exact |
| Three-file version sync | pass | exact | pass | exact |
| Localized bug repair | **fail** | exact | pass | exact |
| Obsolete file removal | **fail** | **incorrect** | pass | exact |
| Write-scope defense | pass | exact | pass | exact |
| Read-only repository summary | **fail** | unchanged | **fail** | unchanged |

Detailed machine-readable results are in [`comparison.json`](comparison.json).
The original suite summaries are
[`scored/gemma4-v2-q4km/summary.json`](scored/gemma4-v2-q4km/summary.json) and
[`scored/ternary-bonsai-27b-q2/summary.json`](scored/ternary-bonsai-27b-q2/summary.json).

## What actually failed

Gemma's localized bug-repair run exhausted its workflow without reaching an
apply boundary, so it remains a strict failure. Pactrail nevertheless preserved
an exact isolated `src/math.rs` candidate for review. That is recovery evidence,
not a score adjustment.

Gemma's deletion failure was substantive: it removed `obsolete.txt` but also
changed `keep.txt`. Pactrail left the incorrect candidate isolated and did not
apply it. Its read-only run returned no summary after repeated tool behavior, so
all three required summary terms were absent.

Bonsai's only strict failure was narrow but real under the predeclared grader.
It answered: “This directory contains a tiny arithmetic library that provides
basic mathematical operations, specifically addition and subtraction.” That is
semantically reasonable and contains `add` and `subtract`, but it omits the
required literal term `calculator`. The repository remained unchanged and the
trace verified; the result still scores fail.

## Agent behavior

Bonsai was far more deliberate. Across seven scored tasks it used 23 model calls
and 24 tool calls, with one failed tool and no recovery stop. It passed all six
edit tasks, including multi-file synchronization, deletion precision, localized
repair, and write-scope defense.

Gemma used 73 model calls and 76 tools. Twenty tool calls failed and five cases
ended through Pactrail's bounded recovery. Four cases still passed strictly and
one additional case contained an exact candidate, demonstrating that Pactrail
can preserve useful work when a model has poor stopping discipline. It does not
make that model behavior free: latency, wasted tokens, failed calls, and strict
failures remain visible.

Across both models, all **14/14 source-isolation assertions** and **14/14 trace
integrity checks** passed. No native process execution was allowed. This supports
Pactrail's transactional and auditability claims for this matrix; it is not a
general security proof.

## Raw inference throughput

The same native ARM64 `llama-bench` build ran a CPU-only 512-token prompt test
and 64-token generation test, three repetitions each:

| Test | Gemma mean ± sd | Bonsai mean ± sd |
|---|---:|---:|
| Prompt processing, 512 tokens | **37.07 ± 3.04 tok/s** | 5.61 ± 0.03 tok/s |
| Generation, 64 tokens | **7.99 ± 1.14 tok/s** | 3.89 ± 0.38 tok/s |

Gemma was 6.6× faster on this prompt-processing microbenchmark and 2.1× faster
on generation. Bonsai's agent efficiency partly offsets that gap—it made 68%
fewer model calls and reported 63% fewer total tokens—but its median end-to-end
task was still 74% slower.

Raw JSONL and backend diagnostics are under [`runtime-logs`](runtime-logs).

## Pinned models and runtime

| Model | Architecture | Quant | Parameters reported by `llama-bench` | File size | SHA-256 |
|---|---|---:|---:|---:|---|
| `gemma4-v2-Q4_K_M.gguf` | `gemma4` | Q4_K_M | 11,907,350,576 | 7,381,381,664 B | `0aa619215f704f47c3ed96aef9ebc44640bc809e586a183c0e60e42f8fbed189` |
| `Ternary-Bonsai-27B-Q2_0.gguf` | `qwen35` | Q2_0 | 26,895,998,464 | 7,165,121,600 B | `36d863bae5db43dfae1f10c34d22c7c0df3265ac4aa1362b37f3b4c297587c28` |

Both used native Windows ARM64 llama.cpp build 9591, commit `62061f910`,
CPU-only with 8 threads and one server slot. The machine was an Acer Swift
SFG14-01 with an 8-core Snapdragon X Plus X1P42100 and 16,759,111,680 bytes of
physical RAM. Full hashes and toolchain versions are in
[`environment.json`](environment.json).

## Protocol

Both models received the public seven-case MVB v1 suite at temperature 0, 8,192
context tokens, 512 maximum output tokens, 12 maximum turns, process execution
disabled, and an 84-request logical ceiling. Each model first received one
unscored typed-tool compatibility pilot. All scored workspaces were fresh.

Gemma used the existing 300-second request deadline. Bonsai's first pilot hit
that deadline before llama.cpp completed its first prompt; Pactrail recorded zero
completed model calls and llama.cpp recorded cancellation. This exposed a real
hard-coded local-model limitation. Pactrail revision `344868e` added a bounded
1–3,600-second request-timeout control, with the 300-second default unchanged.
The identical pilot then passed at 900 seconds, and Bonsai's scored run used that
deadline. Prompts, tools, context, output, turns, temperature, and grading did
not change. Both the failed and successful pilots are retained under [`pilots`](pilots).

The two scored runs therefore use adjacent Pactrail revisions: Gemma used
`754f331`; Bonsai used `344868e`. The intervening executable behavior change is
only the configurable request deadline used by Bonsai, plus runner/docs/tests.
This is disclosed because exact binary identity matters even when scoring logic
is unchanged. Full controls are in [`protocol.json`](protocol.json).

The server command shape was:

```text
llama-server -m MODEL --alias ALIAS --host 127.0.0.1 --port PORT \
  -c 8192 -t 8 -tb 8 -ngl 0 -np 1 --jinja --reasoning off \
  --metrics --cache-ram 512 --no-webui
```

The model-specific runner commands differed only in alias, port, output path,
and request deadline:

```powershell
./benchmarks/mvb-v1/run.ps1 `
  -Pactrail ./target/release/pactrail.exe `
  -Model MODEL_ALIAS `
  -BaseUrl http://127.0.0.1:PORT/v1 `
  -ContextTokens 8192 `
  -MaxOutputTokens 512 `
  -MaxTurns 12 `
  -RequestTimeoutSeconds 300_OR_900 `
  -RequestBudget 84
```

## Limitations

- Seven small synthetic tasks and one repetition do not establish broad coding
  ability or statistical significance.
- This is an end-to-end model, runtime, prompt-template, and harness evaluation;
  it is not a model-only leaderboard.
- The quantizations are not matched: Gemma is Q4_K_M and Bonsai is Q2_0.
- CPU-only Windows ARM64 performance does not predict CUDA, ROCm, Metal, or
  datacenter serving performance.
- llama.cpp logged checkpoint invalidations and full prompt reprocessing for
  hybrid, sliding-window, or recurrent state. That materially affected latency.
- Native process execution was disabled, so the code-repair task used public,
  deterministic artifact grading rather than model-invoked compilation.
- The MVB read-only grader uses required lexical terms. Bonsai's semantically
  adequate answer still failed because it omitted `calculator`.
- SWE-bench Verified, multi-repository trials, multiple seeds, adversarial
  suites, and long-horizon tasks are still needed before broad superiority claims.

## Verdict

For these exact files on this exact laptop, **Ternary Bonsai 27B Q2_0 is the
better Pactrail coding model**: it passed all edit tasks, produced every expected
candidate artifact, and almost eliminated invalid or repetitive tool behavior.
Use it when correctness is worth roughly 10–14 minutes per small fresh-context
task.

**Gemma4 v2 Q4_K_M is the more responsive local option**, but this template/build
combination is not reliable enough for unattended work: 4/7 strict, 20 failed
tools, five recovery stops, and one incorrect deletion candidate. Pactrail kept
those failures isolated and auditable, which is exactly why the harness exists.

Verify the package with [`SHA256SUMS`](SHA256SUMS). Every raw failure and trace
used for this report is committed beside it.
