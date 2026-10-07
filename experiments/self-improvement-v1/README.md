# Pactrail Self-Improvement Lab v1

An experimental, separately launched supervisor for improving the harness.
It freezes the judge and budgets, lets the accepted Pactrail parent implement
one proposal in isolation, and admits a child only after external qualification
and explicit human review. **No performance improvement has been measured yet.**

Python 3.11+, Linux, Docker access, a prepared local image and an exact release
binary are required. No runtime dependencies, images, models or data are fetched
automatically. The ordinary Pactrail CLI, SDK and durable schemas are unchanged. Large request
histories use individual journal-bound records; earlier lab reservation maps
remain readable. Campaigns still have a finite physical-request cap.

## Run the implementation checks

```sh
python3 -m unittest discover -s experiments/self-improvement-v1/tests -v
python3 experiments/self-improvement-v1/fuzz.py --seed 20261007 --iterations 2500
node --check experiments/self-improvement-v1/web/app.js
docker build --tag pactrail-lab-qualification:local experiments/self-improvement-v1/tests/oci-fixture
export PACTRAIL_LAB_TEST_IMAGE="$(docker image inspect --format '{{.Id}}' pactrail-lab-qualification:local)"
export PACTRAIL_BENCH_BINARY=/absolute/path/to/current/release/pactrail
python3 -m unittest discover -s experiments/self-improvement-v1/tests -v
```

The OCI fixture tests use scripted responses, not paid inference. They exercise
the real binary, gateway, tools, candidate, parent receipt/trace checks and
containment. Scripted results are runtime evidence, never coding-quality proof.
The fixture image contains Python/git/glibc; a research image must additionally
contain the compiler and cached offline dependencies needed by its build/gates.

## Prepare a real campaign

Author a manifest matching `pactrail_lab/contracts.py`. Admission is strict:
unknown fields fail. Do not put credentials in JSON, source, argv or artifacts.

| Manifest field | Meaning |
|---|---|
| `schema_version`, `id`, `kind` | `1`, lowercase ID, `research` or `fixture` |
| `model` | Exact ID, HTTPS chat endpoint, credential **environment variable name**, context/output ceilings, input/output rates in integer micro-USD per million, temperature `0`, reasoning `null/low/medium/high` |
| `limits` | `cost_microusd`, physical `requests`, total `wall_seconds`, proposal `cycles`, `repetitions` (3–10) |
| `image` | Installed OCI image ID, `sha256:<64 hex>`; no pull fallback |
| `containment` (optional) | Explicit `memory_mb` (512–16384), `scratch_bytes` (64 MiB–16 GiB); defaults 2048 / 1 GiB |
| `source`, `source_commit` | Absolute local repository and full 40-character commit |
| `baseline_binary`, `baseline_binary_sha256` | Absolute release executable and verified SHA-256 |
| `configuration`, `memory` | `{"agent_mode":"single"}` or `text`; bounded advisory string list |
| `build` | Frozen build argv, input hashes and timeout; writes `/work/pactrail` |
| `gates` | Exactly engineering, authority, compatibility, recovery, mechanism |
| `development_protocol`, `confirmation_protocol` | Absolute protocol paths; disjoint task populations |

Rates are an explicitly declared conservative bound. Cache discounts are not
assumed. Reservations cover the full context/output ceiling for each physical
request and remain charged after failure/crash. Actual invoice cost is unknown.
The byte-based input ceiling is deliberately pessimistic for textual providers;
multimodal requests are unsupported. Provider/model drift fails, never falls back.

Every build/gate/grader uses this shape:

```json
{"argv":["python3","/absolute/external/check.py","{source}","{binary}","{report}"],
 "inputs":{"/absolute/external/check.py":"replace-with-the-real-sha256"},
 "timeout_seconds":300}
```

All program/resources in the external verifier must be listed and hashed.
Freeze helper imports too. Input paths must be standalone argv arguments; their
frozen copies are mounted under `/frozen/inputs/<original absolute path>`.
Placeholders are exact argv elements: `{source}` / `{candidate}` becomes the
contained source copy, `{binary}` the selected executable, `{output}` the build
output, `{report}` `/work/verification.json`. No shell expansion is performed.
Fixed system executables/libraries are bound by the pinned image.

The five gates must be meaningful external programs: complete Rust quality,
adversarial authority tests, old fixtures, recovery/effect replay tests, and a
test of the proposal's predicted mechanism. A successful process exit cannot
establish universal security. Candidate-authored tests are not an independent
authority gate. The trusted operator must not admit no-op gates.

## Task protocols and independent graders

Each protocol has `schema_version`, `id`, `seed`, `repetitions`, `limits`, `tasks`.
Limits are `model_turns`, `wall_seconds`, `context_tokens`, `output_tokens`,
`model_tokens`. Context/output match the frozen model; attempts/wall cannot
exceed campaign ceilings. Each of 1–50 tasks has:

* `id`, `repository`, full `commit` and `gold_commit`, `goal`;
* pinned grader `image`;
* `targeted` and `regression`, in the external operation format above.

Export validated historical cases from the existing Harness Lab when preparing
these records. This module reruns validation rather than trusting an admission
label. Gold patches and grader sources are not mounted in model containers.

Graders must write a structured report to `{report}` **after** executing their
declared tests, using independently parsed results:

```json
{"schema_version":1,"executed":2,"passed":2,"failed":0,"errors":0,"skipped":0}
```

Targeted bad-state grading requires executed failures and exit 1. Gold targeted
and regression require executed passes and exit 0. Empty/skipped/setup-failed
or contradictory reports refuse qualification. Freeze grader behavior before
model execution. Graders are trusted external code; a script that invents this
report is not a valid verifier. Public historical tasks are not independent
held-out curation merely because they are labelled confirmation.

## Workflow (paid execution only when explicitly invoked)

```sh
python3 experiments/self-improvement-v1/lab.py init campaign.json --campaign /absolute/new-campaign
python3 experiments/self-improvement-v1/lab.py status --campaign /absolute/new-campaign
```

Use the exact current `head` returned by status for each mutation:

```sh
python3 experiments/self-improvement-v1/lab.py qualify --campaign CAMPAIGN --expected-head HEAD
python3 experiments/self-improvement-v1/lab.py baseline --campaign CAMPAIGN --expected-head HEAD
python3 experiments/self-improvement-v1/lab.py cycle --campaign CAMPAIGN --expected-head HEAD
python3 experiments/self-improvement-v1/lab.py review --campaign CAMPAIGN --port 3091
```

`qualify` has no paid calls. `baseline` and `cycle` do. Individual `propose`,
`implement --proposal HASH`, `evaluate --candidate HASH` commands expose the
same stages. `cycle` stops at review; it never automatically promotes.
Use stable `--command-id` for transport retries. Completed operations return
their existing result. Uncertain model/evaluation work cannot be replayed.

Development failures produce advisory categories and bounded trace/source
excerpts. The configured core model supplies the proposal. The accepted parent
uses ordinary governed tools to implement it. The child is built/checks run in
OCI; scratch defaults to 1 GiB, memory to 2 GiB, output archive at most 64 MiB.
Declare higher bounded containment limits before freezing a compiler-heavy
campaign. Oversized builds/artifacts fail explicitly. Host/native execution is
never a fallback. Container admission also obeys the campaign wall deadline.

Both cohorts randomize matched parent/candidate/restored-parent arms with the
same model and total per-trial ceilings. A frozen protocol precedes requests.
Strict completion requires external targeted/regression passes and independent
parent receipt/trace/source-isolation checks. Unknown metrics stay null.
Infrastructure failure cannot produce a supported improvement verdict.

Promotion requires a gain ≥5 percentage points, positive task-clustered 95%
bootstrap lower bounds and one-sided paired task-sign tests against both
controls (alpha `0.025 / cycles`). Confirmation is spent once, including after
an interrupted attempt. Finite repeated hosted trials support task-specific
causal evidence under recorded conditions, not a mathematical proof or a claim
of general intelligence improvement. Mechanism and invariant gates also pass.

## Review, Undo and the next generation

The review UI binds to `127.0.0.1`, uses Host/Origin/session checks and CSP, and
renders untrusted text through `textContent`. It shows the diff, prediction,
risks, external records and acceptance comparisons. Cancel is the initial
dialog focus. Approval requires an explicit human review acknowledgment.

```sh
python3 experiments/self-improvement-v1/lab.py approve --campaign CAMPAIGN --expected-head HEAD --verdict HASH --acknowledge-human-review
python3 experiments/self-improvement-v1/lab.py undo --campaign CAMPAIGN --expected-head HEAD --revision HASH
python3 experiments/self-improvement-v1/lab.py export --campaign CAMPAIGN --output NEW_EXPORT
python3 experiments/self-improvement-v1/lab.py verify-export NEW_EXPORT
```

Activation changes only the lab selector. It does not merge, install or apply
to the real repository. Undo restores source/binary/configuration/memory and
retires descendants; evidence remains. Export includes an inventory and exact
active source/executable. Never run an untrusted exported child on the host.

To continue recursively, explicitly commit the exported accepted source into
a separate repository, prepare a new manifest bound to that source, binary,
settings and memory, and provide fresh confirmation tasks:

```sh
python3 experiments/self-improvement-v1/lab.py fork next.json --parent-campaign PARENT --campaign NEW_CAMPAIGN
```

Ancestral Undo remains available in the child campaign. Previously consumed
confirmation source identities cannot become development or confirmation tasks.

After interruption: `recover --campaign CAMPAIGN --expected-head HEAD` cleans
only named, ownership-bound containers, keeps model reservations, and marks
incomplete experiments unqualified. A provider error is not silently retried.
If storage reports a disk/quota/I/O failure, stop, make storage available and
check the journal before recovery. SQLite auto-abort preserves the original
error; uncertain model calls are not refunded or replayed.
Only status/export/recovery should be used until the interrupted operation is
reconciled. New work needs a new command/proposal; there is no free crash retry.

## Limits of this boundary

The operator, frozen verifier programs and Docker daemon are trusted. OCI
shares a kernel. Hashes bind bytes, not truth. Candidate traces are reported
history verified by the parent, not proof that malicious code faithfully logged
every internal action. OS containment independently protects the host source
and credentials; external adversarial gates and human review remain necessary
before accepting changes to the authority core. No arbitrary neural latent
backend, self-modifying weights, unattended production promotion or secret
access is implemented.
