# Pactrail v1 real-issue benchmark

This is an intentionally unflattering result, and that is why it is useful.
Pactrail 1.0.0 and OpenCode 1.2.27 were given the same three real Rust defect
replays, the same DeepSeek model, the same prompts, the same 16-step limit,
offline dependencies, process access, and hidden behavioral graders. Every
trial was pass@1. Nothing was retried or replaced.

The primary V4 Flash protocol was gold-validated, committed, and pushed at
`4f4fad6` before the first model request. After all six Flash outcomes were
known, an unchanged V4 Pro replication was registered and pushed at `dd9ee83`.
The Pro run is useful model-robustness evidence, but it is not a second
independent held-out set.

## Headline

| Model | Harness | Functional | Strict | Agent time | Tokens | Estimated cost |
|---|---|---:|---:|---:|---:|---:|
| V4 Flash | Pactrail 1.0.0 | 0/3 | 0/3 | **94.32 s** | **545,610** | **$0.015768** |
| V4 Flash | OpenCode 1.2.27 | **1/3** | **1/3** | 260.16 s | 613,479 | $0.032198 |
| V4 Pro | Pactrail 1.0.0 | 0/3 | 0/3 | **128.55 s** | **580,742** | **$0.053923** |
| V4 Pro | OpenCode 1.2.27 | **1/3** | **1/3** | 375.68 s | 678,104 | $0.166807 |
| Combined | Pactrail 1.0.0 | 0/6 | 0/6 | **222.87 s** | **1,126,352** | **$0.069691** |
| Combined | OpenCode 1.2.27 | **2/6** | **2/6** | 635.83 s | 1,291,583 | $0.199005 |

`Functional` means the hidden target test and the broader regression command
passed. `Strict` also requires instruction compliance and clean harness
completion; Pactrail additionally requires source isolation, a verified trace,
and a successfully applied ready-to-review candidate. OpenCode is not
penalized for lacking Pactrail-specific transaction concepts.

Across both models, Pactrail used 12.8% fewer reported tokens, 65.0% less
agent wall time, and 65.0% less estimated API cost. Those efficiency numbers do
not offset the correctness result. On this suite, OpenCode won.

## Per-task outcome

| Task | Flash Pactrail | Flash OpenCode | Pro Pactrail | Pro OpenCode |
|---|---:|---:|---:|---:|
| regex one-pass excess capture slots | FAIL | FAIL | FAIL | FAIL |
| HTTP HeaderMap reserve capacity | FAIL | **PASS** | FAIL | **PASS** |
| ripgrep early-termination byte stats | FAIL | FAIL | FAIL | FAIL |

OpenCode's two passing HTTP candidates were focused changes to
`src/header/map.rs`. Neither harness completed the regex fix. Both OpenCode
ripgrep candidates changed the intended production file but failed the hidden
behavioral test. The Flash ripgrep patch also rewrote line endings across the
file; the Pro patch was focused but still incorrect.

Pactrail made no production-source mutation call in any of its six trials.
Five runs exhausted all 16 turns while continuing to search and read; one
Flash run stopped after three identical read requests. This behavior occurred
with both Flash and Pro, making the harness loop/controller—not just model
quality—the leading explanation.

## Assurance result

Pactrail preserved its architectural guarantees even while failing the coding
tasks:

- 6/6 source workspaces were byte-identical before explicit review;
- 6/6 portable traces passed hash-chain verification;
- all failed runs and their evidence remained available;
- no candidate was silently landed in the real source tree.

That is meaningful evidence for Pactrail's transaction and audit design. It is
not evidence that Pactrail is currently the stronger coding agent.

## Protocol

The suite replays pinned pre-fix revisions from rust-lang/regex,
hyperium/http, and BurntSushi/ripgrep. Hidden tests were independently checked
to fail at each base revision and pass at the corresponding upstream fix.
Exact patch similarity was never scored.

Both harnesses used temperature zero, non-thinking mode, a 16,384-token
context, a 2,048-token output cap, 16 turns or steps, identical task text,
fresh synthetic repositories without remotes, prefetched Cargo dependencies,
and offline Cargo mode. Execution order was paired and counterbalanced.

The HTTP repository required one disclosed toolchain normalization:
`dangerous_implicit_autorefs` was allowed so its older source compiles under
Rust 1.95. The change was applied identically to base, reference, and every
harness workspace and did not touch the behavior under test.

The DeepSeek account moved from $1.65 to $1.38 after billing settled. The
$0.27 observed depletion is consistent with the $0.268696 estimate from
provider-reported cached input, uncached input, and output tokens. The
preregistered $1.30 stopping floor was preserved.

## What this proves

The defensible claim is:

> Pactrail 1.0.0's source isolation and trace integrity held in all six real
> issue trials, while it used materially less time, tokens, and estimated cost
> than OpenCode. But it failed all six coding trials because its model loop did
> not transition from investigation to editing; OpenCode passed two. This is a
> credible diagnosis and a concrete release blocker, not a superiority result.

This benchmark rejects any current claim that Pactrail is broadly better than
OpenCode. Publishing that negative result is itself evidence that the project
is being measured rather than marketed by cherry-picked demos.

## Evidence map

- [`comparison.json`](comparison.json) contains normalized aggregate and
  per-task outcomes.
- [`protocol.json`](protocol.json) freezes hashes, scoring, prices, order, and
  spend controls.
- The committed [`suite definition`](../../issue-replay-v1-confirmation/README.md)
  contains both frozen manifests, provider configurations, and the
  [`gold-validation.json`](../../issue-replay-v1-confirmation/gold-validation.json)
  record proving all three hidden graders reject the base and accept the
  reference fix.
- [`environment.json`](environment.json) records the host, toolchain, harness
  versions, executable hash, and source revision.
- [`audit.json`](audit.json) records integrity checks, tool-loop findings, and
  known artifact confounds.
- [`raw`](raw) contains all 12 result records, model streams, traces, patches,
  and grader outputs, without redundant full source workspaces.
- [`SHA256SUMS`](SHA256SUMS) authenticates every publication artifact.

## Limitations

- Three tasks and one sample per model/harness are not enough for a broad
  harness ranking.
- The suite contains only Rust repositories and is not SWE-bench Verified.
- V4 Pro reused the Flash tasks after the evaluator had observed Flash
  outcomes; it is a secondary replication, not independent confirmation.
- Provider load and prefix-cache state affect latency and cost.
- OpenCode's JSON stream reported 18 model events on one Flash trial despite a
  configured 16-step limit; internal bookkeeping may not map one-to-one to
  Pactrail turns.
- Windows archive/symlink materialization polluted regex `candidate.patch`
  files with unrelated binary-entry deletions. `changed_paths` and behavioral
  grading remain authoritative; regex patch size is not interpreted.
- Flash/OpenCode's ripgrep patch line count includes CRLF-to-LF churn.
- Native process execution was enabled for trusted repositories and is not an
  operating-system sandbox.

No failed trial was rerun, hidden, or replaced.
