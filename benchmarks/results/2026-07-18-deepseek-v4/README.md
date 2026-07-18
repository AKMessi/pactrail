# DeepSeek V4 harness evaluation: Pactrail vs OpenCode

This is a matched, model-in-the-loop comparison of Pactrail 0.1.0 and
OpenCode 1.2.27 using DeepSeek V4 Flash and V4 Pro. Each harness ran the same
seven deterministic coding tasks three times per model in fresh workspaces.
Every scored run is retained. There was no case retry, best-of-n selection,
LLM judge, or discarded failure.

The result is strong and specific: **Pactrail passed 42/42 trials; OpenCode
passed 36/42 under the same no-shell policy.** Pactrail also used 59.0% fewer
reported model tokens and 72.9% less summed end-to-end task time. Its source
workspace remained unchanged until explicit apply in all 42 trials, and all 42
hash-chained traces passed integrity verification.

This does not prove that Pactrail is universally better than every coding
agent. It demonstrates that Pactrail outperformed OpenCode on this declared
matrix and that its transaction/evidence guarantees worked on every trial.

## Headline results

| Model | Harness | Strict task score | Median task | Total tokens | Model/tool calls |
|---|---|---:|---:|---:|---:|
| DeepSeek V4 Flash | **Pactrail** | **21/21 (100%)** | **5.50 s** | **281,641** | 85 / 84 |
| DeepSeek V4 Flash | OpenCode 1.2.27 | 18/21 (85.7%) | 10.41 s | 667,772 | 99 / 110 |
| DeepSeek V4 Pro | **Pactrail** | **21/21 (100%)** | **7.07 s** | **265,961** | 81 / 69 |
| DeepSeek V4 Pro | OpenCode 1.2.27 | 18/21 (85.7%) | 15.36 s | 667,592 | 100 / 108 |
| **Combined** | **Pactrail** | **42/42 (100%)** | - | **547,602** | 166 / 153 |
| **Combined** | OpenCode 1.2.27 | 36/42 (85.7%) | - | 1,335,364 | 199 / 218 |

The token count is provider-reported prompt plus completion usage. OpenCode's
AI SDK reports uncached and cache-read input separately; its totals include
both. The scored Pactrail revision recorded total prompt input correctly but
did not recognize DeepSeek's separate cache-hit counter. That telemetry gap was
fixed in revision `452c211` after scoring and is not retroactively inferred.

## Case reliability

| Case | Pactrail Flash | OpenCode Flash | Pactrail Pro | OpenCode Pro |
|---|---:|---:|---:|---:|
| Exact file creation | 3/3 | 3/3 | 3/3 | 3/3 |
| Targeted config edit | 3/3 | 3/3 | 3/3 | 3/3 |
| Three-file version sync | 3/3 | 3/3 | 3/3 | 3/3 |
| Localized bug repair | 3/3 | 3/3 | 3/3 | 3/3 |
| Obsolete file removal | **3/3** | **0/3** | **3/3** | **0/3** |
| Write-scope defense | 3/3 | 3/3 | 3/3 | 3/3 |
| Read-only repository summary | 3/3 | 3/3 | 3/3 | 3/3 |

OpenCode's six failures were all real file-deletion failures. With shell access
denied, OpenCode exposed no bounded delete tool. Some runs correctly reported
that limitation; others claimed the file was deleted even though it remained,
or replaced its contents with an empty file. Pactrail completed all six trials
through its typed `delete_file` tool without receiving process authority.

That distinction matters. Granting an unsandboxed shell may let another harness
run `del` or `rm`, but it also grants a much broader capability than deleting a
workspace-relative file. This protocol intentionally compares both harnesses
with native process execution disabled.

## Pactrail assurance results

Correct final files are only one layer of the score. Across all 42 Pactrail
trials:

- **42/42 exact isolated candidates** matched the expected file contents,
  deletions, and complete changed-path set.
- **42/42 source-isolation checks** confirmed that the original workspace was
  unchanged before apply.
- **42/42 applies** landed the expected result after the candidate was graded.
- **42/42 portable traces** were accepted by Pactrail's hash-chain verifier.
- **6/6 adversarial write-scope trials** changed only `allowed/note.txt` and
  preserved the protected file and untrusted instruction verbatim.
- **0 recovery stops** occurred across 166 model calls.
- **0 native processes** were authorized.

OpenCode was not counted wrong merely because it edits in place or lacks
Pactrail receipts. Its task score uses exact final artifacts and required
summary terms. The direct-write workflow, unchained JSON event logs, and lack of
an explicit apply boundary are reported as architectural differences.

## Efficiency

Across both models, Pactrail used 547,602 reported tokens versus OpenCode's
1,335,364: **59.0% fewer**. Summed end-to-end task time was 266.5 seconds versus
982.0 seconds: **72.9% lower** on this machine and network path.

Flash median task latency was 5.50 seconds for Pactrail and 10.41 seconds for
OpenCode. Pro medians were 7.07 and 15.36 seconds. These are harness-level
latencies, not raw inference benchmarks; provider load, cache state, process
startup, prompts, and tool loops are all part of the measured system.

Pactrail recorded 6 unsuccessful tool calls out of 153; OpenCode recorded 27
out of 218. These totals are useful diagnostics but are not a standardized
cross-harness metric because the tools and event schemas differ.

## Spend

The DeepSeek account began at **$2.00** and ended at **$1.88** after all 84
scored trials, five retained compatibility pilots, and one preliminary
successful transport smoke that preceded the reproducible comparator runner.
The displayed balance delta was therefore **$0.12**. The balance endpoint
reports cents, so this is an account-level measurement rather than an exact
per-matrix invoice.

## Matched protocol

Both harnesses used:

- the direct DeepSeek endpoint and exact `deepseek-v4-flash` /
  `deepseek-v4-pro` model IDs;
- temperature 0;
- explicit non-thinking mode;
- an 8,192-token declared context and 512-token maximum output;
- identical user prompts, fixture bytes, and deterministic graders;
- fresh source workspaces and fresh agent sessions;
- no native shell/process execution;
- three repetitions without case retries.

Pactrail used a 12-turn cap and 300-second request deadline. OpenCode has no
equivalent logical-turn control in this runner, so it used a 180-second
per-case wall-time cap. All scored OpenCode event streams reported zero
reasoning tokens and no provider errors.

The OpenCode permission allowlist was `read`, `edit`, `glob`, `grep`, `list`,
and `todowrite`; every other permission, including `bash`, was denied. Its XDG
config, data, cache, and extension directories were isolated from the user's
normal OpenCode setup. See [`protocol.json`](protocol.json) and the checked-in
[`opencode-deepseek.json`](../../mvb-v1/opencode-deepseek.json).

## Reproduce

Set `DEEPSEEK_API_KEY`, build Pactrail revision `b2e5866`, and run:

```powershell
cargo build --release --locked

./benchmarks/mvb-v1/run.ps1 `
  -Pactrail ./target/release/pactrail.exe `
  -Model deepseek-v4-flash `
  -BaseUrl https://api.deepseek.com `
  -ApiKeyEnv DEEPSEEK_API_KEY `
  -ContextTokens 8192 `
  -MaxOutputTokens 512 `
  -MaxTurns 12 `
  -Repetitions 3 `
  -RequestBudget 252 `
  -DisableThinking

./benchmarks/mvb-v1/run-opencode.ps1 `
  -OpenCode opencode `
  -Model deepseek-direct/deepseek-v4-flash `
  -ApiKeyEnv DEEPSEEK_API_KEY `
  -Config ./benchmarks/mvb-v1/opencode-deepseek.json `
  -Repetitions 3 `
  -MaxCaseSeconds 180
```

Repeat both commands with `deepseek-v4-pro`. The runners create fresh
workspaces and timestamped result roots.

## Evidence map

- [`comparison.json`](comparison.json): normalized aggregate and case matrix.
- [`protocol.json`](protocol.json): controls, fairness decisions, and spend.
- [`environment.json`](environment.json): host, binaries, revisions, and hashes.
- [`audit.json`](audit.json): independent artifact regrade, parse checks, and
  secret scan.
- [`scored/flash`](scored/flash) and [`scored/pro`](scored/pro): every Pactrail
  run output, exact candidate, receipt, apply result, and portable trace.
- [`comparators/opencode-flash`](comparators/opencode-flash) and
  [`comparators/opencode-pro`](comparators/opencode-pro): every OpenCode JSONL
  event stream, stderr, assertion result, and summary.
- [`pilots`](pilots) and comparator pilot directories: unscored protocol
  compatibility evidence.
- [`SHA256SUMS`](SHA256SUMS): SHA-256 manifest for the retained package.

## Limitations

- Seven small synthetic tasks, even with three repetitions and two models, do
  not substitute for SWE-bench Verified or long-horizon repository work.
- This comparison covers OpenCode 1.2.27, not every open-source harness or
  every OpenCode configuration.
- The no-shell policy favors harnesses with narrowly scoped native filesystem
  tools. That is intentional and disclosed, but shell-enabled results may
  differ.
- Latency includes local process startup and internet/provider variance.
- DeepSeek's best-effort prefix cache can affect cost and speed across repeated
  prompts.
- Pactrail's extra isolation and integrity checks all passed, but this matrix is
  not a formal security proof.

The defensible conclusion is not "Pactrail has solved coding agents." It is:
**under a public, reproducible, no-shell protocol, Pactrail was more reliable,
more token-efficient, faster end to end, and provided transaction and evidence
guarantees that the comparator did not.**
