# Ledger Rail browser UI

Pactrail's browser UI treats a run as an auditable contract. The rail contains run history; the workspace contains the task composer, run trace, candidate changes, evidence, receipt, or settings.

The server binds to loopback. The frontend uses vanilla HTML, CSS, and JavaScript embedded in the binary. IBM Plex fonts and the Lucide sprite are local assets. No package manager, bundler, remote asset, or runtime dependency is needed to use the app.

## Reading a run

- **Trace** shows recorded actions, model usage, effects, phases, and evidence. It does not expose model reasoning or tool output bodies, because the engine does not serve them.
- **Changes** shows the isolated candidate, unified or split diffs, viewed marks, and action provenance derived from recorded file effects.
- **Evidence** groups evidence by contract obligation. A passing status is green only when its grade is deterministic. A successful generic command does not automatically establish a deterministic contract check.
- **Receipt** shows the outcome, contract, changes, evidence summary, approvals, metadata, and hashes. Integrity is described as verified only after the engine's diff endpoint reports it as verified.
- **Answer** displays the stored browser-run summary. CLI runs without that summary get an explicit explanation and trace fallback.

A reported zero is different from missing data. Unavailable values display `—` with a reason. Incomplete token coverage displays `≥`. Costs are not estimated from guessed prices. Pricing cards require all four prices; zero is a valid price.

## Dispatch and decisions

The composer previews the consequences of command permissions. **None** cannot run tests or builds. **Sandbox** requires a local image. **Host** requires a fresh acknowledgment for every dispatch; that acknowledgment is never restored from storage.

Only one run can execute in a workspace. There is no queue. Starting uses the accepted job ID until a run ID becomes available. Jobs are associated with runs conservatively when the backend does not supply a run ID.

Apply and discard always open dialogs. Apply starts with focus on Cancel and requires acknowledgment for missing deterministic checks, failed checks, and evidence that predates the last recorded write. Stopped runs have no Apply action. Resume asks the existing engine to validate the durable boundary; the engine can refuse it. The UI does not promise recovery from every terminal state.

Consequential actions leave persistent result banners. Errors preserve the engine message and offer recovery. Connection loss disables network actions; reconnect merges events by sequence without rebuilding existing trace rows.

## Browser-local data

Titles, drafts, preferences, pricing cards, discard notes, recent models, and viewed-file marks live in this browser. They are labeled accordingly and are not engine records. A storage failure falls back to memory and is reported in Settings.

Run views, selected files, and manual diff layout use URL parameters. Trace expansion, filters, follow state, and tab scroll positions remain in memory for the session.

## Security and delivery

The existing loopback binding, origin checks, and Content Security Policy remain intact. Engine and model strings are rendered through `textContent`. Markdown is built with DOM nodes and supports no HTML or image execution. Theme initialization uses the external, blocking `/boot.js` route; there are no inline scripts or inline styles.

The server changes are limited to `/boot.js` and five IBM Plex font routes replacing the three previous font routes. Font and icon licenses are bundled beside the assets.

## Verification

Run the dependency-free projection, contrast, and markup checks from the repository root:

```sh
node --test crates/pactrail-cli/web/tests/*.test.cjs
cargo fmt --all --check
cargo test -p pactrail --bin pactrail web::
cargo clippy -p pactrail --all-targets -- -D warnings
cargo build -p pactrail
```

The browser checks are development tools, not application dependencies. Install Playwright in a directory outside this repository and set `NODE_PATH` to its `node_modules`. Set `PACTRAIL_CHROMIUM` when supplying an existing Chromium executable.

```sh
NODE_PATH=/path/to/node_modules node crates/pactrail-cli/web/tests/browser-check.cjs
python3 crates/pactrail-cli/web/tests/model-fixture.py
# In another terminal, while the local model fixture is running:
NODE_PATH=/path/to/node_modules node crates/pactrail-cli/web/tests/engine-check.cjs
```

`browser-check.cjs` serves explicitly synthetic API fixtures with the real frontend and CSP. It checks both themes, mobile and desktop screens, the specified eight viewport widths, 320px coarse-pointer targets, a 32-file candidate, a 5,000-line diff, long and Unicode paths, more than 2,000 events, bounded append batches, duplicate SSE delivery, and reconnect preservation of filter, expansion, selection, and scroll. Screenshots are written to `/tmp/pactrail-ledger-qa` by default. A 720px effective viewport covers the layout pressure of a 1440px viewport at 200% zoom; this does not simulate every browser or mobile keyboard.

`engine-check.cjs` uses the real compiled engine in disposable workspaces and a deterministic local OpenAI-compatible test provider. It checks read-only answers, isolated edits, guarded apply, discard with a local note, host verification, obligation-bound deterministic acceptance checks, stopping, provider failure, and engine-process restart. Screenshots and results go to `/tmp/pactrail-ledger-engine-qa`. This tests UI and engine integration, not the quality or reliability of a paid model provider. The script serves current source frontend assets over the engine page during development; the compiled embedded assets are checked separately when launching the final build.

Projection tests cover absent/partial/measured usage, zero versus absent cost, pricing conversion and completeness, unified-diff parsing, freshness, provenance, and deterministic evidence. Design tests verify token contrast in both themes and reject inline code/styles, unsafe rendering, gradients, all-caps transformation, out-of-token colors, and unrestricted pass coloring.

## Backend capabilities that remain unavailable

The UI does not pretend to offer assistant reasoning, evidence artifact bodies, expandable unchanged diff context, live candidate diffs, persisted discard reasons, or apply preflight checks. Those require backend capabilities and are represented by truthful explanations or receipt fallbacks.
