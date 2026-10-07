# Recursive harness improvement: frozen external qualification

Status: implementation in progress. Experimental; no model-quality claim.
Base: `385a8407853f6c7fd53fc103c083645df168e642` (2.1.2 research infrastructure).
Branch: `research/recursive-self-improvement-v1`. No main/tag/release changes.

## Problem and acceptance contract

Pactrail should identify observed weaknesses, ask the configured core model for
a falsifiable proposal, implement one mechanism through the accepted parent,
build an isolated child, independently evaluate it, and request human promotion.
An accepted child is the next parent. Every logical change can be undone with
its derived memories and descendants. This is harness evolution, not weight
training, latent communication, or evidence of accelerating intelligence.

The user chose all harness subsystems editable, human-approved promotion,
strict causal evidence, and CLI plus a separate local review page. All editable
code runs as untrusted candidate code. Editing authority code is permitted;
weakening the supported authority contract is not permitted for promotion.

## Trust model

`experiments/self-improvement-v1` is a separately launched supervisor. Its code,
campaign configuration, evaluator commands, task partitions, credentials,
budget ledger, journal, and approval state are outside candidate mounts.
Frozen snapshots and hashes bind those inputs. Candidate traces/receipts are
checked with the accepted parent, not with the child's own verdict alone.
Hashes prove byte identity, not honesty. External functional tests, invariant
fixtures, containment and human review supply the acceptance evidence.

Candidates/build scripts execute only in Linux OCI containers: pinned image,
non-root UID, read-only root, no host repository or socket, no networking,
dropped capabilities, no-new-privileges, bounded time/memory/PIDs/output. A
read-only Unix socket provides access to an endpoint-bound external model
gateway via a frozen HTTP shim. Trial tokens are ephemeral and limited; actual
provider keys never enter the container. Host processes are never a fallback.
Unavailable OCI or loopback support is BLOCKED, not PASS.

## Components and durable behavior

* Strict bounded records: campaign, weakness, proposal, revision, experiment,
  verdict, approval and undo. Schema one; unknown fields/versions fail closed.
* SQLite transactions commit an append-only hash-linked event chain and
  content-addressed immutable records. A rebuildable projection selects the
  active lab revision. Every mutation has a caller command ID and expected head.
* Artifacts use SHA-256, exclusive atomic writes, length bounds, no symlinks and
  verified reads. Source exports reject links, special entries, paths escaping
  the root, excess files/bytes and secret/control metadata.
* Model reservations precede I/O. Ambiguous attempts retain maximum charge;
  crashed requests are not transparently replayed. Container identities are
  journaled before launch; recovery reconciles/removes only those named objects.
* Builds are independent of the installed executable. Promotion changes only
  the lab selector and requires an approval bound to candidate and verdict.
  Production apply/merge/install is a separate human action.

## Loop and causal evaluation

Admission verifies known-bad and gold grader evidence using existing Harness Lab
gates. Diagnostics contain development evidence only, with bounded drill-down
artifacts. The core model proposes up to three alternatives; one mechanism is
implemented. Its hypothesis names a weakness, predicted metric, risks and exact
parent. Changes require a fresh candidate and registration once scoring starts.

Development uses paired A/B runs. Confirmation uses A (accepted parent), B
(candidate), R (independently restored parent). Verify R's source, configuration
and memory identities against A before scoring. Equal model/routes, permissions,
budgets, task sources, graders and repeated randomized order are frozen. Default
three repetitions, strict completion primary, minimum gain five percentage
points. Positive task-clustered confidence bounds and paired randomization tests
are required against A and R; complete coverage and mechanism/invariant gates
are mandatory. Small cohorts can legitimately be inconclusive.

Confirmation is consumed once and not exposed to the improver. Further cycles
need fresh confirmation capacity; no significance-seeking reruns. Provider
failures remain in declared outcomes. Missing metrics remain null. Cost, safety,
correctness, completion and latency remain separate, not one synthetic score.

## Undo and recursive lineage

One logical mechanism is an atomic revision, even across multiple files. Undo
restores a complete parent snapshot, retires descendants, and invalidates their
derived memories. No `git reset --hard` on user work and no database downgrade.
Runs remain pinned to the revision that started them. Journal commit is the
authority for promotion/undo; interrupted materialization is reconstructed from
immutable objects. An uncertain external effect cannot be guessed away.

## APIs, compatibility and UI

Standalone `lab.py`: init, qualify, baseline, cycle, status, review, approve,
undo, resume, export. JSON is versioned and bounded. Existing ModelDriver,
TaskContract, RunCheckpoint, SDK and ordinary CLI defaults remain unchanged.
The local review UI has external static assets, strict CSP, loopback binding,
origin/session checks, safe text rendering and hash-bound approval/undo. It
never loads candidate scripts or private reasoning.

## Implementation and qualification sequence

1. Immutable storage, strict contracts, source snapshots, journal and budgets.
2. OCI lifecycle, gateway, parent-driven mutation and existing grading adapters.
3. Preregistration, weakness/proposal flow, causal analysis and verdict gates.
4. Approval, undo, local UI, crash recovery and offline two-generation fixture.
5. Full infrastructure/regression/security/compatibility/CI qualification.
6. Only afterwards: newly registered DeepSeek/GLM benchmark with explicit cap.

No paid inference is authorized by an offline fixture. Provider endpoints,
models, rates and billing coverage must be declared; unknown costs fail closed.
Source history/remotes, gold patches, private graders and operator credentials
are absent from agent inputs. Dependencies/images are prepared explicitly;
build/evaluation never downloads them implicitly.

## Research provenance

Conceptual inspiration only; no external implementation is copied:

* [Self-Harness](https://arxiv.org/abs/2606.09498): weakness/proposal/validation.
* [AHE](https://arxiv.org/abs/2604.25850): evidence-linked, reversible changes;
  independent regression checks because self-attribution misses regressions.
* [DGM](https://arxiv.org/abs/2505.22954): lineage/archive; bounded linear v1.
* [STOP](https://arxiv.org/abs/2310.02304),
  [AIDE²](https://arxiv.org/abs/2609.26457): accepted child as next parent;
  harness improvement does not prove improved meta-optimization.
* [GEPA](https://arxiv.org/abs/2507.19457),
  [ACE](https://arxiv.org/abs/2510.04618): trajectory feedback and explicit memory.
* [Skill contamination](https://arxiv.org/abs/2608.05810): dependency-aware undo.
* [Reward hacking](https://arxiv.org/abs/2609.28614): independent recomputation.
* [Generic holdout](https://arxiv.org/abs/1809.05596),
  [SEA](https://arxiv.org/abs/2607.00871): bounded testing and fresh confirmation.
* [HarnessDev](https://arxiv.org/abs/2609.01437),
  [SWE-agent](https://arxiv.org/abs/2405.15793): executable infrastructure/ACI.

## Risks and limits

Finite tests cannot certify arbitrary edited code safe. OCI shares a host kernel;
containment is not a formal proof. Hosted weights/cache/provider drift cannot be
cryptographically pinned. Public historical tasks are not independently curated
held-out tasks. Small task clusters limit statistical power. Final claims must
state evidence coverage and remaining blocks, never imply universal improvement.
