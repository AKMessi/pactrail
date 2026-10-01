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
- Three benchmark regression tests: all six broken baselines fail, all six
  gold candidates pass, and early process exit cannot impersonate completion.
- Result-accounting regression: absent scores remain null with all trials pending.

The final full workspace all-feature suite passed after full access was enabled:
392 tests passed, with no failed test summaries. Commit `66479fb` publishes the
engine implementation on `codex/evidence-completion-upgrade`.

## Remaining evaluation gates

The frozen 48-trial comparison is running after network and loopback access were
verified. Prior managed restrictions prevented any scored requests; those are
historical, not the current permissions. GitHub connector issue creation remains
blocked by its repository integration scope (HTTP 403), while Git SSH push works.
The research design is committed in `docs/design/0017-evidence-directed-completion.md`.

Before default rollout: complete and inspect the frozen comparison, all failures,
and token coverage, then evaluate on larger independent repository issues.
Independent reviewer context and acceptance-plan generation remain future designs.
