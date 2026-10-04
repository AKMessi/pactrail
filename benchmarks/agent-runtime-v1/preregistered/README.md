# Frozen qualification protocols

Never edit a protocol after its first scored request. Runner correctness repairs
require a new protocol and a complete restart; failures are retained.

`space-bunny-20261005-v1.json` is **INVALID for model-quality conclusions**.
Targeted and regression grading reused one mutable grading copy. Installing the
sealed target tests caused regression's forbidden-file validation to reject
those tests. The experiment was stopped; all started trials and original runner
sources remain in the ignored qualification artifacts. No successes or failures
from this invalid setup are used to claim architectural performance.

The replacement runner isolates each grading phase and has a regression test
that writes a grading artifact and rejects contamination between phases. A new
registration retains the same tasks, seed, arms, repetitions and model budgets;
grader timeouts are made explicit using the original task manifest ceilings.

`space-bunny-20261005-v2.json` completed all 18 declared trials after validation
of the complete known bad/gold grading pipeline. Its immutable SHA sidecar and
[public summary](space-bunny-20261005-v2-summary.json) retain the observed negative
quality result: single 6/9, full text 1/9. The ignored evidence directory retains
raw failed trials, receipts, traces, status, candidate diffs and grading output.
