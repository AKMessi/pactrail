# Bounded agent research protocol

This is public experiment infrastructure, separate from the private Pactrail
Bench repository. It makes no performance claim. Python 3.11+ and POSIX process
groups are required. No model, dataset, container or dependency is downloaded.

## Run

1. Copy `protocol.example.json` to a new frozen protocol. Replace every placeholder
   with local source repositories, full commits, executable absolute adapter
   paths, pinned model identity and sealed external graders. Graders must remain
   outside the model workspace. Validate graders against known bad and known good
   patches first. Do not use developer-authored diagnostics as held-out evidence.
2. Keep common topology, routes, permissions and budgets fixed across text/latent
   arms. The single baseline uses the same global ceilings. Record actual usage,
   hardware, backend versions, tokenizer, representation, fusion and checkpoint
   identity. For hosted endpoints, record identity uncertainty rather than
   pretending their opaque weights are cryptographically pinned.
3. Provide keys only in the environment. `PACTRAIL_BENCH_BINARY` selects an exact
   Pactrail executable for the bundled single/text adapter. That adapter supports
   **disabled commands and workspace-scoped writes only**. A different shared
   authority configuration needs an adapter that admits it explicitly.

```console
python3 -m unittest discover -s benchmarks/agent-runtime-v1 -p 'test_*.py' -v
export PACTRAIL_BENCH_BINARY=/absolute/path/to/pactrail
python3 benchmarks/agent-runtime-v1/run.py frozen-protocol.json --output benchmark-results/agents-trial-01
python3 benchmarks/agent-runtime-v1/analyze.py benchmark-results/agents-trial-01/results.json --control latent-correct --treatment latent-zero
```

Output must be new; existing trials are never overwritten. Trial order is seeded
and recorded before execution. Each arm/repetition receives a fresh local clone
at the exact source commit. Adapter stdout/stderr, request, receipt, trace,
checkpoint/artifact store and candidate remain available. Grading uses a separate
copy. Targeted and regression results are distinct; success requires both.
SHA-256 inventories seal protocol, runner, local program inputs and raw results.
Adapters and graders are trusted executable code, not a sandbox. Provision
hardware/disk quotas outside the runner and use restricted OCI authority for
hostile repository execution. Grader environment excludes inherited API keys.

## Adapter contract

An adapter receives one additional argument: an absolute `request.json` path.
It must enforce common limits/permissions and the requested arm without fallback,
then write `result.json` in `trial_directory`:

```json
{"schema_version":1,"model_identity":{"model":"same-as-request"},
 "candidate":"/trial/local/candidate",
 "metrics":{"input_tokens":123,"output_tokens":20,"cost_microusd":0}}
```

Model identity must equal the full request identity object. Candidate paths must
stay inside the trial; escaping symlinks are rejected. Every metric is either a
measured non-negative integer or `null` (missing is normalized to `null`). The
metric inventory is in `run.py`; partial token coverage belongs in
`usage_coverage` and must not be disguised as complete totals. Peak memory and
inference time stay absent unless actually measured. Wall-clock time is measured
by the runner. Exit 78 means explicitly unsupported, nonzero means failure;
malformed/missing results are invalid. Unsupported and invalid trials have no
fabricated scores and remain visible in coverage. Paired analysis excludes them
and reports the exclusion counts. Failures remain failures in paired comparisons.

The bundled `pactrail_cli.py` runs the real single/text engine, preserving state
and ordinary token accounting. **It returns 78 for latent arms.** No first-party
CLI provider exposes internal state. Supply a genuine SDK-backed local inference
adapter for those arms; the runner must not turn them into text. The CPU runtime
mock tests are correctness checks, not model-quality benchmarks.

## Causal interventions

`AgentRunConfig.intervention` accepts `correct`, `none`, `zero`, seeded `random`,
seeded `shuffled` and `wrong_task`. Correct is the default. Non-correct variants
require explicit latent mode. Shuffled currently permutes **f32 scalar positions
within a message**, not whole messages between tasks; state that distinction in
reports. A future task-message permutation is a different intervention.

Wrong-task requires an explicit content-addressed donor descriptor in the same
artifact store, with a different run identity and exactly compatible model/shape.
Seal the donor descriptor and payload before trials and hold donor selection fixed.
The runtime validates it, rebinds the altered payload to the current run/sender
and records the intervention. Ordinary cross-run delivery remains forbidden.
Original and altered payloads are retained by digest. Stored-byte charges include
both; suppressed transfers have zero delivered messages/slots but still charge
retained originals. Suppression attempts remain subject to the message-attempt cap.

Compare task-clustered paired success differences and uncertainty, along with
turns, tokens, latency, bytes, memory and cost coverage. Pre-register hardware,
seeds, stopping rules, task population, donor selection and analysis. Flat correct
vs ablated performance falsifies useful communication for that setup. A tiny
fixture or a mock-only effect is not evidence of repository-task superiority.

The current CLI adapter treats zero-valued normalized token fields conservatively:
it cannot distinguish a provider-reported zero from the legacy `Usage` default
for an absent field. Such totals remain `null`, with coverage explained; positive
fully covered totals remain measured. This does not alter the engine ledger.
Cost, when available, is the engine estimate from a configured rate card, not a
provider invoice. A reported estimated cost of zero remains distinct from absent.

Optional `model_identity.pricing` provides exactly `input_price`,
`cached_input_price`, `cache_creation_price` and `output_price` as non-negative
integer micro-USD per million tokens. The CLI adapter rejects incomplete cards.
Provide all four explicit zeros only when intentionally estimating a zero-priced
model; omit the card when pricing is unknown. Grader commands and paths are
never included in the adapter's task request.
