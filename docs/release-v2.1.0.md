# Pactrail v2.1.0

Publication requires human review. This branch does not create a release or tag.

## Bounded specialist collaboration

Pactrail v2.1 adds opt-in specialist agents with independent conversations,
addressed text communication, explicit budgets and durable recovery. The built-in
coding profile runs localizer → solver → critic → implementer in a fixed serial
order. Single-agent execution remains the default.

Agent roles do not grant authority. Read-only specialists cannot write or execute
commands. The implementer requests effects through the existing Tool Kernel;
the contract, policy, approvals, isolation and verification still decide what is
permitted. There are no parallel candidate writes, recursive spawns or hidden
child agents. More specialists can consume more tokens and latency or perform
worse. This opt-in workflow remains experimental, not a promise of better fixes.

```console
pactrail agent-template > agents.json
pactrail run "Fix the regression" --agent-config agents.json --max-turns 24
pactrail agents RUN_ID --json
pactrail resume RUN_ID
```

Use the existing configured provider. Selecting agents does not enable commands.
Provider attempts, ordinary input/output usage, messages and communication bytes
are separate counters. Missing provider reporting remains unknown; a genuine
reported zero is preserved. Intermediate language-token counts remain unknown
when the provider does not expose a separate generated-text count.

## Recovery and compatibility

Agent lifecycle and message delivery commit atomically with content-addressed
session checkpoints. Reserved attempts are not refunded by resume. Completed
model usage is checkpointed before communication persistence. A new pre-reservation
checkpoint preserves the controller prelude when an agent invocation checkpoint
cannot commit. Uncertain tool completion, changed authority/identity, corrupt
payloads and invalid journal references continue to fail closed.

Ordinary events, receipts and schema-three checkpoints retain their compatibility
ranges. Experimental agent config/session/latent formats are schema one and
agent checkpoints use `agents-v1:` references. Older binaries cannot resume these
references. SDK API revision 8 adds the experimental agent/latent namespace;
existing model drivers remain valid without implementing a latent backend.
No migration is required to keep using single-agent runs.

## Experimental latent transport

The SDK includes bounded, integrity-bound model-state artifacts and a
`LatentModelDriver` extension for genuinely compatible model backends. All bundled
hosted and ordinary Chat Completions providers reject latent mode explicitly.
Space Bunny Alpha does not expose arbitrary hidden states through this API.

The deterministic CPU backend and correct/none/zero/random/shuffled/wrong-task
interventions qualify runtime mechanics only. They do not prove useful neural
communication, reduced compute, lower cost or improved task success. No real
hidden-state inference backend, KV transfer, cross-model translation or parallel
model execution ships in this release.

## Qualification evidence

See [the qualification ledger](design/0019-v2.1-release-qualification.md) and
[frozen benchmark protocols](../benchmarks/agent-runtime-v1/preregistered/).
The diagnostic comparison uses three public historical defects, externally
validated bad/gold graders and three repetitions per arm. It is not independently
curated held-out evaluation and cannot establish general coding superiority.
The invalid first experiment is retained and excluded from quality conclusions.

On the frozen Space Bunny Alpha diagnostic suite, single-agent Pactrail passed
6/9 trials and the full text profile passed 1/9 under the recorded global limits.
Eight text trials exhausted specialist ceilings. Lower text-arm resource use
reflects early failure, not a speed/cost advantage. The profile remains
experimental; no broad superiority or latent efficiency claim is made.
