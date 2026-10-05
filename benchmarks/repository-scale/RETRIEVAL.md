# Offline retrieval measurement

`retrieval_probe` measures existing navigation rankings without a model, writes,
process tools, a new production retriever or a change to the context compiler.
Use development trees only when investigating strategy changes.

```sh
cargo build --locked -p pactrail-context --all-features --example retrieval_probe
./target/debug/examples/retrieval_probe SEALED_BASELINE GOAL_FILE 20 > ranking.json
```

Inputs: a sealed pre-fix tree, UTF-8 task goal file (at most 64 KiB) and K (1–100).
Output pins repository/query digests and records returned paths, shared index
latency, per-arm ranking latency and graph truncation. No source content or gold
patch appears in the output.

- `lexical`: existing `retrieve` with an empty graph; filename, direct symbol and
  conventional-anchor behavior otherwise remains unchanged.
- `graph`: existing graph query; ordered symbol definitions then references,
  deduplicated by path and bounded at K. This declared extraction policy is not
  a new production broker.
- `current_hybrid`: unmodified production `retrieve` ranking.

The index is built once with the enabled native parsers; Go and unsupported
languages retain existing fallback behavior. Cloning index metadata for lexical
is outside the ranking timer. Shared indexing constructs the graph for all arms;
these timings cannot establish end-to-end lexical versus graph speed or memory.
No context token/byte budget is measured by path-only ranking.

For a proper retrieval study, freeze tasks, labels, parser features, K values,
extraction policy and probe commit/binary before measurement. Reference-fix
paths are only incomplete proxy labels: added files may not exist in the source,
unchanged dependencies may still be relevant, and test-file edits can inflate
apparent recall. Report exclusions and manually validate labels before calling
them relevant-file gold. Symbol labels need separate validation. Do not tune on
the confirmation split. Path recall does not prove task success or causal utility.

The bounded graph extraction has a deterministic deduplication regression test.
The ordinary compiler, Tool Kernel, receipts, policy and candidate lifecycle are
unchanged. No retrieval improvement has been demonstrated by this probe alone.
