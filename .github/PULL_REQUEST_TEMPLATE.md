## Outcome

<!-- What user-visible behavior or invariant changes? -->

## Design and trust boundary

<!-- Capabilities, paths, processes, network, secrets, memory, persistence, provider, or external effects. Use "none" where applicable. -->

## Failure and recovery

<!-- How do denial, interruption, malformed input, partial completion, and retry behave? -->

## Verification

<!-- Exact commands and manual checks performed. -->

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [ ] `cargo test --workspace --all-features --locked`
- [ ] User-facing docs and `CHANGELOG.md` are updated, or not applicable.
- [ ] No credentials, private source, personal data, `.pactrail` state, model files, or generated artifacts are included.
- [ ] Commits include DCO sign-off (`Signed-off-by`).
