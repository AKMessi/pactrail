# CLI experience verification

## V2 running-input verification — October 3, 2026

The real debug binary passed the expanded 42-scenario terminal suite using a
loopback provider and disposable workspaces. This includes type-ahead/caret
preservation, explicit-only next-task dispatch, cooperative stop with retained
Unicode drafts, denied buffered approval, fresh per-request consent, SIGINT at
approval and live resize from 32 to 100 columns. Unix plain Ctrl-C cancels both
the post-run prompt and Apply confirmation. Plain
and dumb captures contain no terminal control sequences; Apply remains guarded.

Eight real browser-engine workflows also passed: dispatch/answer, guarded
Apply, Discard, real Cargo verification, Stop, provider failure inspection,
contract-bound deterministic evidence and reconnect stability. Ten browser
projection/design tests passed.

These are deterministic interaction checks, not model quality measurements.
The final v2 version/build/platform evidence is tracked separately in the launch
checklist; the October 2 measurements below remain historical evidence.

## Ledger branch verification — October 2, 2026

Platform: Linux, Rust 1.95. Debug and optimized executables were exercised against
local deterministic providers in disposable workspaces; no live model credentials
were used.

- 33 PTY scenario groups passed, including draft/retry/editor/pager flows, guarded
  Apply/Discard, same-ID recovery after a provider failure and restart, ASCII mode,
  explicit focus in the composer and two sessions that cannot redirect each other.
- Startup widths: 32, 40, 60, 80, 100, 120 and 160 columns. Colored startup, answer
  and review captures at 40 and 100 columns were inspected using captured screen
  cells. NO_COLOR checks passed. TERM=dumb retains reedline input; this is not a
  claim of a cursor-free plain mode.
- Workspace suite: 416 passed, 0 failed, 1 existing ignored Docker containment
  test. CLI unit portion: 98 passed.
- Formatting, strict workspace Clippy, warnings-as-errors documentation, and the
  optimized workspace build passed.

The source audit and remaining milestones are in [v2-audit.md](v2-audit.md) and
[v2-cli-plan.md](v2-cli-plan.md). These checks do not demonstrate complete v2
implementation, model task-solving quality, crash coverage at every durable
boundary or macOS/Windows terminal compatibility.

## Earlier workflow verification

Date: September 30, 2026. Platform: Linux, Rust 1.95.

## Verified behavior

The real debug executable was exercised in PTYs against the local deterministic
model fixture, in disposable workspaces without real provider credentials:

- Multiline dispatch and answered output; `/retry` restores a draft without running it.
- Isolated edit candidate, obligation/evidence review, and explicit Apply.
- Empty or incorrect Apply acknowledgment cancels; source stays untouched.
- Empty Discard acknowledgment cancels; explicit Discard removes the candidate.
- Run-prefix focus and subsequent inspection select the requested run.
- Described command palette, Tab completion, quoted task-file loading, and
  external-editor draft return.
- `less` pager exit returns to the prompt.
- Startup at 32, 40, 60, 80, and 120 columns; TERM=dumb and NO_COLOR.

Terminal screen captures were reviewed for startup, answer, review, and applied
results. The optional reproducible driver is
[`devtools/check_cli_experience.py`](../devtools/check_cli_experience.py).
It needs development-only pexpect/pyte and the local fixture port 4190.
Captures are generated under `/tmp/pactrail-cli-qa`, not committed.

Separate real-process smoke checks passed for the new one-shot human report,
`-C` workspace selection, and unchanged JSON outcome/receipt/cost fields.

## Automated gates

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`: 388 passed; one pre-existing ignored test.
  The CLI portion includes 81 unit tests and 17 integration tests.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features`
- `cargo build --workspace --release --locked`

The optimized CLI executable replaced the previous local installation;
`pactrail --help` was checked for the new workspace shorthand and session hints.

New regression tests cover grapheme/cell wrapping, code whitespace, missing
versus zero cost, bounded UTF-8 task files, executable argument parsing,
contextual completion, narrow presentation, and review cautions derived from
actual evidence and trace order.

## Limits

PTY checks were run on Linux; macOS and Windows terminal behavior was not
executed locally. Configured editors/pagers are explicit user-controlled local
programs; their command strings are parsed as argv rather than passed to a shell.
The fixture proves interface behavior, not frontier-model task-solving quality.

Existing normalized token counters can lose provider-field absence. The UI labels
these as engine-counted tokens and does not claim complete provider coverage.
The V3 proposal addresses that underlying limitation. It is a proposal, not a
shipped architecture change or a measured victory over other harnesses.
