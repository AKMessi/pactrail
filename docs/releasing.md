# Release runbook

Pactrail releases are cut from an immutable commit already merged to `main`.
Never move or reuse a published tag. A correction after publication gets a new
patch version so checksums, attestations, source, and durable compatibility stay
unambiguous.

## 1. Prepare the release commit

On a release branch:

1. Set the workspace version and every internal dependency pin to the exact
   release version; regenerate `Cargo.lock`.
2. Move user-visible changes from `Unreleased` into a dated changelog section.
3. Add `docs/release-vX.Y.Z.md`; include upgrade, security, installation, and
   known-limit information.
4. Update support, compatibility, security, and upgrade documents when their
   promises change.
5. Run the complete local gate:

```console
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo doc --workspace --all-features --no-deps --locked
cargo build --workspace --release --locked
cargo test --workspace compatibility_fixture --locked
cargo test --locked -p pactrail-workspace io_failure_matrix
```

Also parse every workflow file, validate both installers on their native shell,
and run the deterministic repository-scale gate. Do not spend provider credits
or publish comparative benchmark claims as an implicit release step.

## 2. Merge and observe main

Merge the reviewed branch into `main`, push it, and wait for every required CI,
dependency-policy, containment, and repository-scale job to finish. Inspect
failures; never rerun a failing job until the cause is understood. The release
commit must remain in `main` history.

## 3. Create one tag

From the exact verified commit, create an annotated tag and push only that tag:

```console
git tag -a vX.Y.Z -m "Pactrail vX.Y.Z"
git push origin vX.Y.Z
```

The release workflow fails before expensive builds unless the tag, workspace
version, `Cargo.lock`, changelog heading, release-note filename/title, and
`main` ancestry agree. It then runs cross-platform tests/builds, the attested
three-iteration deterministic soak, checksum generation, provenance
attestation, GitHub release creation, and installer smoke tests.

## 4. Verify the published release

Wait for every release job, including all three installer smokes. Confirm:

- `gh release view vX.Y.Z` lists three binary archives, both installers, the
  soak report/resource log, and `SHA256SUMS`;
- the displayed release notes are the checked-in file;
- `gh attestation verify` succeeds for the downloaded assets;
- a clean Tier 1 machine installs the pinned tag and prints the exact version;
- the latest-release badge and one-line installers resolve to the new version.

If publishing fails before release creation, fix forward and create a new commit
and tag. If any asset was publicly visible, do not replace it under the same
tag; document the issue and publish a patch release. Individual workspace crates
are not published to crates.io under the v1 distribution contract.
