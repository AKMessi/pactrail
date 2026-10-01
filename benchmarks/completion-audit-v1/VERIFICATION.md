# Verification and remaining gates

October 1, 2026. Implementation is opt-in; performance improvement is unproven.

Passed locally:

- Engine all-feature tests: 81 passed, including repair of a second call path,
  no fabricated passed evidence, audit allowance replay, and a real interrupted
  model call followed by durable resume without replaying the completed write.
- CLI unit tests: 81 passed.
- Workspace formatting and diff whitespace checks.
- Strict workspace Clippy, all targets and all features.
- Documentation build with warnings denied.
- Locked offline release build.
- Four benchmark regression tests: all six broken baselines fail, all six
  gold candidates pass, early process exit cannot impersonate completion, and empty-file additions/deletions are detected by the corrected runner.
- Result-accounting regression: absent scores remain null with all trials pending.

The final full workspace all-feature suite passed after full access was enabled:
392 tests passed, with no failed test summaries. Commit `66479fb` publishes the
engine implementation on `codex/evidence-completion-upgrade`.

## Completed controlled comparison

All 48 frozen v1 trials completed using the exact free Space Bunny Alpha model:
Pactrail baseline 12/12 functional passes, Pactrail audit 12/12, OpenCode 12/12,
mini-SWE-agent 11/12. All trials were graded; none were replaced. Audit used
70.5% more reported tokens than baseline without functional improvement here.

The recorded v1 cleanliness detector missed newly added empty files. This
limitation is disclosed alongside unchanged original strict scores; the current
runner has a regression-tested correction for future experiments. Original
runner sources are archived with the raw results. Console label fixes do not
alter trial JSON identities. Redacted publication retains original/published
artifact hashes and never includes the real provider key.

Git SSH commit/push succeeded. GitHub connector design-issue creation remains
blocked by repository integration scope (HTTP 403); the design is committed in
`docs/design/0017-evidence-directed-completion.md`. The installed home-directory
CLI was updated to the tested release executable after full access was enabled.

## Remaining rollout gates

The audit remains opt-in. Broader independent repository issues and acceptance
checks are needed before any default rollout or superiority claim. Independent
reviewer context and acceptance-plan generation remain future designs.
