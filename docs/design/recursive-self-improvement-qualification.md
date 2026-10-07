# Self-improvement lab v1: implementation qualification

Date: 2026-10-08. This is an experimental research supervisor, off by default.
**Engineering tests are not evidence of improved coding performance.** Paid
DeepSeek/GLM trials have deliberately not been started.

## Code identity and publication

* Branch: `research/recursive-self-improvement-v1`.
* Base: `385a8407853f6c7fd53fc103c083645df168e642` (Pactrail 2.1.2).
* Runtime/artifact milestone: `f29ea72c20e9b87fdb342345f453fa5f806008f7`.
* Subsequent qualification commits contain documentation only unless explicitly
  identified in this ledger. Inspect the branch head and its CI before use.
* No merge to main, tag, release, production installation or workspace apply.
* No Rust crate, Cargo dependency, ordinary SDK, receipt or checkpoint change
  relative to the base. The supervisor is a separate Python stdlib program.

Logical commits cover design, storage, containment/gateway, selection/Undo,
review/recursion, real-engine qualification, large request histories, storage
failure handling and retained artifacts. Every milestone was pushed separately.

## Engineering evidence

| Gate | Result | Scope |
|---|---|---|
| Rust formatting | PASS | `cargo fmt --all -- --check` |
| Strict Clippy | PASS | workspace, all targets/features, locked, `-D warnings` |
| Rust tests | PASS | 460 passed, zero failed; one existing opt-in OCI test initially ignored |
| Explicit original OCI test | PASS | Previously ignored hostile-effects test executed separately: one passed |
| Rust documentation | PASS | workspace/all features/no deps; warnings denied |
| Release build | PASS | workspace/all features/locked/offline |
| Lab tests with actual engine and OCI | PASS | 33 passed, zero failures/skips |
| Existing agent-runner tests | PASS | 10 passed, including the real CLI adapter |
| Existing Harness Lab tests | PASS | 31 passed |
| Terminal workflows | PASS | 42 checks using the release binary and loopback fixture |
| Model setup workflows | PASS | 13 checks; disposable config, no paid inference |
| Installer | PASS | Verified asset accepted; tampering rejected without replacing installed fixture |
| Review JavaScript / installer syntax | PASS | Node syntax check / `sh -n` |
| Finite hostile-input smoke | PASS | Seed 20261007, 2,500 random inputs; 5,000 JSON/archive refusals, no crash |
| Actual review HTTP controls | PASS | Cookie/session, Host/Origin, stale head, approval retry, complete Undo |
| Browser visual/accessibility review | NOT QUALIFIED | No browser surface available in this session; no screenshots claimed |
| Paid-model improvement experiment | NOT QUALIFIED | Deferred by the user until implementation is complete |
| Independent security certification | NOT QUALIFIED | Tests and containment are not a formal security proof |

The real-engine smoke uses **scripted model responses**, not a hosted model.
It exercises two physically reserved gateway requests, the real `write_file`
tool, isolated candidate creation, independent parent trace/receipt/diff
inspection, and retention of `before` in the source versus `after` in the
candidate. It does not establish useful autonomous self-improvement.

### Exact local executable and environment

* Release SHA-256:
  `efece82aa0c14f020254b4a87da396d3c3ff7fb6146054c46fab8aa89913d605`.
* Qualification image ID:
  `sha256:39702c3eb9ce2cfc26c6db60dbc78d59f4f5bac240d55da39a441226b3ee9559`.
* Image Dockerfile pins Ubuntu 24.04 by digest; provisioning Python/git is an
  explicit development action. Candidate execution never pulls an image.
* Linux, Docker 29.8.2, Rust 1.95.0. The ignored evidence directory contains
  the observed OS/Python metadata without environment variables or API keys.
* Use the executable's actual canonical path. Trusted-input admission correctly
  refuses a symlink in the executable path.

## Failures found and repaired

1. **Container scratch permissions:** the non-root worker could not write its
   isolated copy. Tmpfs ownership is now explicit, bounded and non-root.
2. **Image entrypoint ambiguity:** worker argv now sets an explicit entrypoint.
3. **Missing inner model attribute:** older Pactrail traces do not consistently
   expose the response model. The inner CLI adapter does not assert that absent
   attribute; the external gateway independently enforces exact returned model
   identity on every physical request. There is no provider substitution.
4. **Failed worker diagnostics:** bridge failures now retain their archive/logs
   and explicit verification failure rather than discarding the temporary data.
5. **Large request ledger:** individual journal-bound reservation records avoid
   rewriting a monolithic JSON object. A 3,001-request regression covers this;
   the older map format remains readable with conservative charges intact.
6. **Real storage exhaustion:** temporary build quota produced an LLVM linker
   failure and a subsequent debug-cache write failure. The release build was
   rerun on disk. SQLite can auto-abort on I/O failure; rollback now preserves
   the original diagnostic. A regression injects this commit failure, proves no
   uncommitted selector survives, and verifies the journal afterward. CLI/HTTP
   surfaces report storage failure without replaying uncertain work.
7. **Undo after recursive continuation:** restoring an ancestor also resets
   baseline qualification, retires descendants and restores advisory memory.
8. **Nested inventory filename:** a source `sha256.json` is now verified as data,
   rather than accidentally excluded as though it were the root inventory.
9. **Admission hardening:** identical source snapshots across cohorts, incomplete
   implementation checks and unsupported nested provider modalities fail closed.

Failed local build logs are retained. No scored trial was run or removed.

## Security and recovery coverage

| Boundary | Result | Evidence |
|---|---|---|
| Host source writes / outbound networking | PASS | Real non-root OCI fixture cannot perform either |
| Alternate writable host binds / mount escapes | PASS | Rejected before container admission |
| Provider key exposure | PASS | Key remains external; redaction tests inspect retained objects |
| Hidden grader exposure | PASS | Model container excludes frozen external grader inputs |
| Model identity / capability drift | PASS | Exact gateway binding; wrong response/model/modalities rejected |
| Spending / request / wall admission | PASS | Reservations precede I/O; exhausted/expired leases fail |
| Crash after reservation/dispatch | PASS | No automatic physical-request replay or refund |
| Interrupted operation / container recovery | PASS | Named ownership check; Docker uncertainty stays pending |
| Journal/projection/command cache corruption | PASS | Authoritative replay rejects forged cached state |
| Approval / stale browser / retry | PASS | Exact head/verdict/parent; explicit acknowledgment; idempotent completed command |
| Recursive Undo | PASS | Full snapshots and ancestry; descendants retired; evidence retained |
| Grader setup/empty/skipped/error result | PASS | Cannot qualify bad/gold states or count as behavioral success |
| Statistical fluke / missing/duplicate trials | PASS | Task-clustered tests and complete coverage; unknown is not success |

These tests cover the stated mechanics. They do not prove arbitrary modified
authority code safe. The operator, frozen verifier programs, accepted parent,
container image and Docker daemon are trusted. OCI shares a kernel. Candidate
history checked by the parent is not an independent observation of every
internal instruction. Meaningful external invariant gates and human review
remain essential when accepting changes to authority code.

Network socket timeouts and bounded container cleanup may outlast an admission
deadline. A wall ceiling prevents new admissions and bounds contained work; it
is not a hard real-time guarantee of instantaneous host-side cleanup.

## Compatibility and scope audit

The feature adds no default startup work, Rust dependencies or engine behavior.
Old campaign reservation projections can be read; schema one remains strict.
New campaigns freeze supervisor bytes and verifier inputs. Editing the
supervisor invalidates an admitted campaign instead of silently changing its
judge. Fresh campaigns must bind the exported accepted source, executable,
configuration and memory and avoid previously used confirmation source hashes.

No automatic host install, merge, apply, neural latent backend, weight update,
unattended promotion or guarantee of general improvement is implemented.
Configuration and memory are immutable advisory revision snapshots; this is
not an autonomous editor for the user's production memory database.

## Evidence retention and CI

Local evidence:
`benchmark-results/self-improvement-qualification/20261008/` (ignored).

It contains stdout/stderr logs, terminal captures, source-input hashes,
environment metadata, a real-engine content-addressed export, raw request and
response artifacts, candidate/run artifacts and a SHA-256 inventory. The
real-engine export passed `verify-export` with 57 files. No real provider key
or credential environment snapshot is retained.

The earlier complete CI run
[37666834266](https://github.com/AKMessi/pactrail/actions/runs/37666834266)
passed all seven jobs at `9782e0ca1e05c046ebcd661e699372d494dbe57d`.
Later qualification commits require their own checks; earlier green CI is not
evidence that later code passed. Final commit/run identity is retained in the
local evidence metadata and reported with the completion message. The workflow
now includes the actual release binary + pinned-container lab integration test,
alongside existing Linux/macOS/Windows, dependency, containment and terminal gates.

## Reproduce

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
cargo build --workspace --all-features --release --locked
python3 -m unittest discover -s benchmarks/harness-lab-v1 -p 'test_*.py' -v
PACTRAIL_BENCH_BINARY="$PWD/target/release/pactrail" python3 -m unittest discover -s benchmarks/agent-runtime-v1 -p 'test_*.py' -v
python3 devtools/check_installers.py target/release/pactrail
docker build --tag pactrail-lab-qualification:local experiments/self-improvement-v1/tests/oci-fixture
export PACTRAIL_LAB_TEST_IMAGE="$(docker image inspect --format '{{.Id}}' pactrail-lab-qualification:local)"
export PACTRAIL_BENCH_BINARY="$PWD/target/release/pactrail"
python3 -m unittest discover -s experiments/self-improvement-v1/tests -v
python3 experiments/self-improvement-v1/fuzz.py --seed 20261007 --iterations 2500
node --check experiments/self-improvement-v1/web/app.js
```

For the opt-in original containment test, explicitly build
`tests/fixtures/oci-sandbox` as `pactrail-oci-fixture:local`, then run:

```sh
PACTRAIL_OCI_TEST=1 PACTRAIL_OCI_TEST_IMAGE=pactrail-oci-fixture:local \
PACTRAIL_HOST_SECRET=pactrail-host-secret-must-not-escape \
cargo test -p pactrail-tools oci_backend_blocks_hostile_effects -- --ignored --nocapture
```

Terminal reproduction requires the development-only pinned packages in
`devtools/terminal-requirements.txt` and `PACTRAIL_CLI_TEST_BINARY` pointing at
the release binary, followed by `devtools/check_cli_experience.py` and
`devtools/check_model_setup.py`.

## Next experiment: explicitly deferred

Prepare meaningful external engineering/invariant/mechanism gates, disjoint
validated historical task cohorts, an offline compiler image, exact available
DeepSeek/GLM provider identities and conservative pricing ceilings. Freeze them
before the first paid request. Then execute the documented `qualify → baseline
→ cycle → review` flow. Both model evaluations count all declared outcomes.
Do not infer useful recursion from scripted tests or claim that a statistically
supported task-specific change proves self-sustaining acceleration.
