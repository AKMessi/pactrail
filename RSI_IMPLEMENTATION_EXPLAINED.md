# Pactrail’s self-improvement lab, in simple words

## The idea

Pactrail now has a separate experimental laboratory for improving its own
harness code. The language model can suggest and implement a change. A frozen
supervisor decides whether the experiment supports keeping it. You decide
whether the accepted version becomes active in the lab.

This changes the harness around the model. It does not train or rewrite the
model’s weights. We have not yet measured better coding performance.

## The loop

```text
Measure failures on development tasks
              ↓
Core model proposes one specific change
              ↓
Current accepted Pactrail implements it in an isolated copy
              ↓
Build the child and run independent checks
              ↓
Compare parent, child, and restored parent
              ↓
Fresh confirmation tasks
              ↓
Human review → keep in the lab, or leave the parent active
              ↓
Fresh experiment using the accepted child as the next parent
```

## What was implemented

| Piece | What it does | Why it helps |
|---|---|---|
| Frozen supervisor | Keeps evaluation, credentials and decisions outside the editable harness | The candidate cannot change the rules used to judge itself |
| Failure mining | Records failed development trials, trace observations and advisory categories | Proposals start with observed problems |
| Model proposal | Requires a hypothesis, mechanism, risks and exact parent | Makes each change testable |
| Parent-driven implementation | Uses Pactrail’s normal tools in a contained candidate | Keeps changes separate from your real source |
| Independent build and gates | Checks engineering, authority, compatibility, recovery and the predicted mechanism | A performance score cannot excuse a broken trust boundary |
| Matched evaluation | Runs parent, candidate and restored parent under frozen conditions | Helps distinguish a useful change from noise |
| Strict graders | Require actual executed behavior checks, not just a successful command | Empty or broken grading cannot count as success |
| Budget gateway | Reserves each request before it reaches the provider | Crashes and retries cannot silently reset spending |
| Immutable history | Stores snapshots and evidence by content hash | Lets us inspect exactly what was tested and accepted |
| Local review page | Shows the diff, prediction, risks and evidence | Gives you a concrete decision to review |
| Complete Undo | Restores code, executable, settings and memory; retires descendants | A bad change does not have to poison later generations |
| Fresh-cohort continuation | Carries accepted ancestry into a new campaign | Supports another iteration without repeatedly tuning on the same confirmation tasks |

## The judge stays outside the loop

The editable object is Pactrail’s candidate harness. The candidate gets no
provider API key, hidden grader, gold patch, campaign database or approval
control. All writes it can make are contained in bounded container scratch.

Your original repository is mounted read-only for a trial. Activation selects a
lab snapshot; it does not merge a branch, replace your installed binary, or apply
changes to your real repository. Those remain separate human actions.

## How “better” is decided

The candidate must pass the external gates and solve more tasks reliably.
The default threshold is at least five percentage points of strict completion,
with positive uncertainty bounds and a paired test against both controls.

Three repeats of one task do not become three independent tasks. The analysis
groups by task. Missing outcomes, provider failures, incomplete grading and
uncertain results cannot secretly become successful trials. A small experiment
may legitimately say **inconclusive**.

Confirmation tasks are used once. Future iterations need fresh ones. This
reduces the risk of making Pactrail better only at a familiar benchmark.

Statistics do not prove general improvement or eliminate every possible fluke.
They provide evidence about the specific frozen experiment. Safety tests and
human review are still required, especially when changing policy or recovery.

## What Undo means

Each logical change is one complete revision, even if it edits many files.
Undo restores its parent’s source, executable, configuration and memory.
Accepted descendants are retired together. Old evidence is kept.

It does not run `git reset --hard` on your working tree. It does not downgrade
Pactrail’s production database. It does not erase the experiment’s history.

## How this improves Pactrail today

It gives us a controlled way to test harness changes, reject unsupported claims,
retain useful evidence and reverse accepted changes. That makes research and
engineering decisions easier to audit.

It does **not yet establish** that Pactrail solves more coding tasks, uses fewer
tokens or improves itself faster on each generation. Those are the questions
for the later DeepSeek/GLM benchmark, after implementation qualification.

## Where everything lives

* `experiments/self-improvement-v1/` — supervisor, CLI, container workers, review UI and tests.
* `docs/design/recursive-self-improvement-v1.md` — architecture and trust boundaries.
* `experiments/self-improvement-v1/README.md` — configuration, commands and verifier contracts.
* `docs/design/recursive-self-improvement-qualification.md` — exact checks and remaining limitations.

The implementation checks include 33 lab tests, the real Pactrail binary inside
a pinned container, 460 Rust tests, and the existing terminal and installer
checks. Scripted model fixtures make these software tests; they are not paid
model-quality benchmarks. Browser visual review has not been qualified.

The feature is experimental and off by default. Ordinary Pactrail commands,
engine authority, SDK interfaces, receipts and checkpoint formats are unchanged.
