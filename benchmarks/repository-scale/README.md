# Repository-scale performance budgets

This suite measures Pactrail's deterministic repository pipeline without a
model or provider. It is separate from task-correctness benchmarks: its job is
to catch index, cache, context, descriptor, latency, and memory regressions.

## Measured lifecycle

The runner creates a fresh deterministic polyglot repository containing Rust,
Python, JavaScript, and TypeScript files plus root project anchors. It then:

1. performs a cold content-addressed index build;
2. drops that index and performs a warm build over the same current bytes;
3. compiles targeted context under an explicit byte ceiling;
4. edits exactly one file, drops retained state, and performs an incremental
   build; and
5. repeats that entire lifecycle in fresh source/cache directories when
   requested; and
6. emits a schema-versioned JSON report with durations, file/byte counts,
   parser/fallback counts, cache hits/misses/writes, citation coverage,
   repository/incremental/context digests, cross-iteration stability, and every
   violated budget.

The runner strictly requires zero cold hits, complete warm reuse, exactly one
incremental miss, stable cold/warm repository identity, at least one targeted
citation, and a context pack no larger than its declared budget. Timing
ceilings are configurable and deliberately generous in shared CI; raw durations
remain measurements, not universal performance claims.

The synthetic task pins its acceptance-condition identifier. Production task
IDs remain unique, but pinning this fixture removes intentional per-run identity
from the context digest so the soak can detect actual retrieval/render drift.

The built-in tool catalog has a separate normal test gate: at most 32 tools,
32 KiB total compact JSON, 8 KiB per descriptor, and schema depth at most 24.
This prevents feature growth from silently consuming the model window before a
task begins.

## Run locally

Build first so compilation is outside the measured process, then run the
release binary:

```console
cargo build --release --locked -p pactrail-context --example repository_scale
target/release/examples/repository_scale --files 10000 --iterations 5 --context-bytes 32768 --max-cold-ms 60000 --max-warm-ms 15000 --max-incremental-ms 15000 --max-context-ms 5000
cargo test --release --locked -p pactrail-tools --test descriptor_budget
```

On Windows the executable ends in `.exe`.

`--files` accepts 100 through 100,000 and `--iterations` accepts 1 through 20.
Every duration is wall-clock milliseconds. Use the same hardware, filesystem,
build profile, commit, file count, and iteration count when comparing reports.

## CI memory measurement

The dedicated `Repository scale budgets` workflow runs one 2,000-file lifecycle
for pull requests, three fresh 5,000-file lifecycles after relevant changes
reach `main`, and five fresh 10,000-file lifecycles weekly or by manual dispatch.
It builds before measurement, wraps only the release runner in GNU `time -v`,
and retains both the runner JSON and raw resource report for 30 days. Linux
maximum resident set size must remain at or below 1.5 GiB.

Every release tag independently runs a three-iteration 5,000-file soak. Its
schema-v2 JSON report and raw resource usage are checksum-covered, provenance-
attested release assets alongside the binaries. Publish cannot start if the
soak, descriptor gate, latency ceilings, RSS ceiling, or identity stability
check fails.

RSS includes the generated fixture and native parser runtime; it is not a Rust
heap profile. Shared-runner timings are noisy, so they use regression ceilings
rather than leaderboard thresholds. A failed run uploads partial artifacts for
diagnosis and never rewrites a failure as a pass.
