# Post-upgrade DeepSeek benchmark

This benchmark does not prove Pactrail is better than OpenCode. It proves the
opposite on this small regression suite, and it identifies a concrete release
blocker.

Pactrail 1.0.0 at `74c5ccd` and OpenCode 1.18.7 were given the same three real
Rust defect replays with DeepSeek V4 Flash and V4 Pro. Prompts, source commits,
hidden graders, context, output limits, process access, and offline policy were
matched. Every trial was pass@1. No failed sample was retried, replaced, or
hidden.

The complete protocol and its gold validation were committed and pushed at
`11adf39` before the first paid model request.

## Headline

| Model | Harness | Functional | Strict | Agent time | Reported tokens | Estimated cost |
|---|---|---:|---:|---:|---:|---:|
| V4 Flash | Pactrail 1.0.0 | 0/3 | 0/3 | 42.69 s | 166,509 | $0.013202 |
| V4 Flash | OpenCode 1.18.7 | **1/3** | **1/3** | 223.17 s | 556,587 | $0.032015 |
| V4 Pro | Pactrail 1.0.0 | 0/3 | 0/3 | 64.68 s | 168,670 | $0.046072 |
| V4 Pro | OpenCode 1.18.7 | **1/3** | **1/3** | 460.79 s | 565,811 | $0.092033 |
| Combined | Pactrail 1.0.0 | 0/6 | 0/6 | 107.37 s | 335,179 | $0.059274 |
| Combined | OpenCode 1.18.7 | **2/6** | **2/6** | 683.96 s | 1,122,398 | $0.124048 |

`Functional` requires the hidden targeted test and the broader regression
command to pass. `Strict` additionally requires instruction compliance, no
timeout, and clean harness completion. Pactrail is also required to preserve
source isolation, verify its trace, produce a reviewable transaction, and
apply exactly that reviewed candidate.

Pactrail used 70.1% fewer tokens, 52.2% less estimated API cost, and 84.3% less
agent time. Those numbers are not an efficiency win: Pactrail terminated early
without completing any task.

## Per-task result

| Task | Flash Pactrail | Flash OpenCode | Pro Pactrail | Pro OpenCode |
|---|---:|---:|---:|---:|
| regex one-pass excess capture slots | FAIL | FAIL | FAIL | FAIL |
| HTTP HeaderMap reserve capacity | FAIL | **PASS** | FAIL | **PASS** |
| ripgrep early-termination byte stats | FAIL | FAIL | FAIL | FAIL |

OpenCode produced three production edits. Its two HTTP edits passed both
graders; its Flash regex edit failed the hidden behavior. The other three
OpenCode trials never mutated production source.

Pactrail produced zero production mutation calls across all six trials.

## What failed in Pactrail

All six Pactrail traces show the same sequence:

1. The model gathers useful repository evidence.
2. The deterministic controller enters the `implementing` phase.
3. After one focused read, tools such as `read_file`, `search`, and
   `search_code_graph` are removed from the model's available schema.
4. DeepSeek requests more evidence instead of editing.
5. Pactrail rejects the unavailable requests.
6. Recovery stops after three consecutive failed tool turns.

Across the six runs, the controller rejected 20 requests and no production
mutation occurred. Flash and Pro failed the same way, which makes the
controller policy—not provider quality—the leading cause.

This is a more precise diagnosis than the earlier July 22 benchmark. Before
the phase-aware upgrade, Pactrail also scored 0/6, but usually spent the full
turn budget continuing to investigate. The new controller successfully forces
a phase transition, then makes that transition too rigid for models that need
another focused observation before editing.

## What held

Pactrail's safety architecture remained intact:

- all 6 source workspaces remained unchanged before explicit review;
- all 6 portable trace hash chains verified;
- every failed run remained durable and inspectable;
- no unreviewed candidate landed in source.

That validates the transaction and audit boundary. It does not compensate for
0/6 task correctness.

## Protocol and spend

The suite replays pinned pre-fix commits from rust-lang/regex, hyperium/http,
and BurntSushi/ripgrep. Each hidden targeted test was independently confirmed
to fail at the base commit and pass at the corresponding upstream fix; the
broader reference regression command also passed.

Both harnesses used:

- temperature 0 and non-thinking mode;
- a 16,384-token context and 2,048-token per-call output cap;
- a configured 16-turn or 16-step budget;
- fresh synthetic repositories without remotes;
- prefetched dependencies and Cargo offline mode;
- native process execution on the trusted benchmark repositories.

Execution order was paired and counterbalanced. A hard `$1.08` balance floor
was checked before each case. The account moved from `$1.38` to `$1.20`, an
observed spend of `$0.18`; provider-token pricing estimated `$0.183322`.
Rates came from DeepSeek's official
[pricing documentation](https://api-docs.deepseek.com/quick_start/pricing/),
and balances came from the official
[balance endpoint](https://api-docs.deepseek.com/api/get-user-balance/).

## Defensible conclusion

The only honest public claim from this run is:

> On a preregistered six-trial real-issue regression, Pactrail preserved source
> isolation and trace integrity in every run, but completed 0/6 tasks because
> its implementation-phase tool policy rejected additional evidence requests.
> OpenCode 1.18.7 completed 2/6. Pactrail does not yet support a superiority
> claim.

This result is useful because it gives the next engineering iteration a narrow,
measurable target: preserve bounded discovery while allowing evidence access
to degrade adaptively instead of disappearing after one implementing read.

## Evidence map

- [`comparison.json`](comparison.json) contains normalized aggregate and
  per-task outcomes.
- [`protocol.json`](protocol.json) records frozen hashes, controls, scoring,
  pricing, and spend.
- [`environment.json`](environment.json) records the host, toolchain, source
  revision, and executable identities.
- [`audit.json`](audit.json) records integrity checks, the trace-level failure
  cluster, and measurement caveats.
- The committed
  [`suite definition`](../../issue-replay-post-upgrade-v1/README.md) contains
  the manifest, configs, hidden graders, execution order, and
  [`gold-validation.json`](../../issue-replay-post-upgrade-v1/gold-validation.json).
- [`raw`](raw) contains all 12 result records, model streams, Pactrail traces,
  patches, and grader output, excluding redundant source workspaces.
- [`SHA256SUMS`](SHA256SUMS) authenticates every publication artifact.

## Limitations

- Three tasks with one sample per model and harness cannot establish a broad
  ranking.
- This is a post-upgrade regression confirmation on tasks that informed the
  architecture; it is not an independent held-out evaluation.
- The suite contains only Rust repositories and is not SWE-bench Verified.
- Provider load and prefix-cache state affect latency and billed cost.
- Two OpenCode regex trials emitted 18 `step_finish` events despite a configured
  16-step limit. No runner-side retry occurred; raw streams are retained.
- The raw Pactrail `provider_errors` field is misnamed for these failures: it
  records a non-zero harness exit, while traces show successful model
  invocations followed by controller rejections.
- Windows archive materialization adds unrelated binary-entry deletions to
  regex patch artifacts. Behavioral grading and `changed_paths` are
  authoritative.
- Native process execution is not an operating-system sandbox.

No failed trial was rerun, hidden, or replaced.
