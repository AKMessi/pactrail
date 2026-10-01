# Completion audit: controlled evaluation

This is an exploratory six-case Python suite for incomplete fixes. It is not
SWE-bench, a representative production workload, or proof of harness superiority.
External graders are never placed in agent workspaces. Every broken baseline
fails and every supplied gold implementation passes the offline grader tests.
The graders require a completion sentinel: an imported module exiting early
cannot turn an incomplete grader into a pass.

## Arms and controls

Compare Pactrail before this upgrade, Pactrail with `--completion-audit`,
OpenCode 1.18.34, and official mini-SWE-agent 2.4.6. The mini adapter uses its
standard agent and prompt templates with a custom compatible endpoint.
Two repetitions give 48 trials. Order rotates across arms and cases.

All use the exact `stealth/space-bunny-alpha` model, temperature 0, low reasoning,
8,192 output tokens, at most 12 HTTP attempts, and a 300-second agent deadline.
A shared proxy caps serialized message bytes at 131,072; this is **not** an
identical tokenizer-based context budget. Harnesses retain their own prompts,
tools, context handling, retries, and termination semantics.

The upstream response is buffered for every arm and converted to SSE when
needed. These results cannot compare real streaming latency. Local command
execution is authorized for these disposable trusted repositories. This driver
requires Linux process groups and loopback networking. Agent environments receive
only an ephemeral proxy credential; the OpenRouter key stays in the orchestrator.
Model pricing is checked live before each trial; no paid fallback is allowed.
Zero-dollar model use cannot establish a monetary cost advantage.

Functional grading examines the final candidate, including a Pactrail isolated
candidate when apply is unavailable. Strict success additionally requires clean
exit, no timeout, and no unrelated file changes. Pactrail strict success also
requires original-source isolation, verified trace, successful apply, and matching
production-file bytes after apply. Compare functional scores across harnesses;
report these extra Pactrail assurance checks separately. Provider errors and
missing usage remain explicit. Token totals with incomplete coverage are partial.
A two-repetition synthetic suite cannot support broad statistical conclusions.

## Reproduce

Provide the pinned executables and a Python environment containing the official
mini package. Supply paths through `--baseline`, `--pactrail`, `--opencode`, and
`--mini-python`; defaults describe this development machine's temporary installs.

```sh
python3 -m unittest discover -s benchmarks/completion-audit-v1 -p 'test_*.py'
python3 benchmarks/completion-audit-v1/run.py freeze --output /tmp/completion-experiment
python3 benchmarks/completion-audit-v1/run.py run --output /tmp/completion-experiment
python3 benchmarks/completion-audit-v1/summarize.py /tmp/completion-experiment
```

Freeze records executable hashes, versions, implementation source hashes, model
metadata, task/runner hashes, order, and baseline/gold validation. Optional
`--metadata-file` allows offline preregistration from a captured API metadata
array; execution still requires a live free-price check. An optional
`--implementation-commit` records a published implementation SHA when available.
Never overwrite a frozen directory. Do not edit its source files between freeze
and execution. Interrupted trials are retained without replacement; rerunning
continues only not-yet-started trials. Unexpected exceptions are retained as
orchestration failures, not functional passes. Raw artifacts accompany each trial.

## Completed pilot and reproduction

The frozen v1 comparison finished all 48 trials. See
[results and rollout decision](../results/2026-10-01-space-bunny-completion-audit/README.md).
Audits did not improve functional outcomes here and used 70.5% more reported
tokens than baseline. The engine policy stays opt-in.

The scored runner is commit `09e4ba6`; exact frozen source copies accompany the
results. The current runner fixes empty-file detection and console labels and
freezes schema version 2. **Use a fresh experiment directory for new runs.**
Do not run the modified runner against the completed v1 protocol. Reproducing
v1 requires its original source version and pinned executables. API model
behavior may change even with the same public model identifier.

V1 strict scores are preserved as recorded: its cleanliness detector missed
newly added empty files. Functional grading is unaffected. Source-isolation
measurements checked the original production-file bytes, not a full inventory
of unrelated original workspace paths. Request counts represent admitted
upstream attempts, excluding local rejections after the cap.
