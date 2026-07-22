# Compatibility contracts

Pactrail treats every persisted or externally consumed shape as an explicit
contract. A Rust type being deserializable is not, by itself, a compatibility
promise.

Run the compile-time inventory from any installation:

```console
pactrail compatibility
pactrail compatibility --json
```

The JSON envelope is intended for upgrade automation. It names each format's
owning crate, current schema, oldest readable schema, migration strategy, and
whether the data is authoritative durable state or safe-to-rebuild derived
state. The inventory is generated from the same constants used by runtime
readers and is pinned by a checked-in compatibility fixture.

Every historical schema in a declared readable range has an immutable fixture
under `tests/fixtures/compatibility/historical`. Run the complete historical
reader and migration suite with one command:

```console
cargo test --workspace compatibility_fixture
```

The fixture manifest is checked against the compile-time inventory, so adding
a readable schema without a corresponding artifact fails the suite. Each
artifact is then exercised by its owning production reader: event envelopes
are hash-verified and projected, event databases are opened read-only and
atomically migrated, memory schema one is read without mutation and upgraded
transactionally without inventing receipt anchors, and settings are migrated
through the normal crash-safe persistence path. A manifest-only placeholder
therefore cannot satisfy the compatibility gate.

Audit the actual workspace state and interactive settings before an upgrade:

```console
pactrail migrate
pactrail migrate --json
```

For one read-only operational report that combines this state audit with active
CLI deprecations and exact next steps, use `pactrail upgrade [--json]`. See the
[upgrade guide](upgrading.md).

These forms are read-only. They inspect schema headers without creating a
database, verify current event chains, transaction/run bindings, receipts,
checkpoint artifacts, and offline MCP snapshots, then report every pending or
incompatible component. Apply only known migrations explicitly:

```console
pactrail migrate --apply
```

Apply mode completes the whole compatibility preflight before changing a
component, refuses to run while a local run lock is active, and re-audits the
result. Each SQLite schema change is committed in one database transaction;
settings use the same fsync-and-rename persistence path as normal interactive
updates. Unknown versions stop the preflight, so no supported component is
changed in that invocation.

`PACTRAIL_CONFIG_DIR` may point to an absolute alternate settings directory for
portable installations and hermetic CI. It does not change workspace state;
`--state-dir` remains the independent workspace-state override.

## Strategies

- `exact_version`: only the named schema is accepted. Unknown older or newer
  data fails closed; it is never guessed into a current shape.
- `read_compatible`: the current reader verifies a bounded range of historical
  versions without rewriting their integrity-bound bytes.
- `migrate_atomically`: a known historical version is upgraded through a
  crash-safe local transaction before normal use.
- `rebuild_derived`: a mismatch invalidates derived data, which may be rebuilt
  from authoritative workspace bytes. It never changes source files.

All future schema versions fail closed. Pactrail never downgrades durable state,
and it never obtains new process, network, MCP, write, or secret authority while
migrating a known format.

## Stable 1.x contract

Pactrail 1.x follows Semantic Versioning for the surfaces below:

- documented non-interactive commands, flags, exit behavior, and versioned
  JSON envelopes;
- documented interactive commands and their durable effects;
- task, event, receipt, trace, checkpoint, MCP snapshot, settings, and other
  formats listed by `pactrail compatibility --json`;
- the provider-neutral model/tool contracts reexported by `pactrail-sdk`; and
- the authority, isolation, review, apply, and recovery guarantees described in
  the threat model and architecture documents.

Human-oriented colors, spacing, progress animation, prose, and diagnostic
wording may improve in any 1.x release. Scripts must use JSON modes and stable
exit codes, not scrape terminal output. New optional JSON fields, enum variants
marked non-exhaustive, commands, tools, and provider capabilities may be added
in a minor release. Removing or changing a documented field or behavior
requires a major release unless the old behavior is unsafe.

The stable Rust contract is the `pactrail-sdk` facade, not every public item in
the workspace's implementation crates. Downstream embedders should depend on an
exact Pactrail tag or compatible `1.x` release and check `SDK_API_REVISION` when
their integration needs a specific extension surface.

Every 1.x durable-schema change must include:

1. an updated entry in `pactrail compatibility --json`;
2. a checked-in current or historical fixture exercised by its production
   reader;
3. tests for the oldest supported reader and an unknown-future rejection;
4. documented atomicity, rollback, and authority behavior; and
5. a release-note migration section.

Pactrail 1.x will continue to read or atomically migrate every safe format
shipped by 1.0 for the lifetime of the 1.x line. A schema may stop being
accepted only in 2.0 after a deprecation notice in at least one prior 1.x minor
release. Security fixes may reject a previously accepted unsafe format
immediately; the release notes must name the exception and the safe recovery
path.

The manifest describes format compatibility, not binary downgrade safety. Do
not open state with an older Pactrail binary after a newer binary has migrated
it unless that older release explicitly declares the resulting schema readable.

## Distribution and support

GitHub release binaries, their checksum manifest, provenance attestations, and
source installation from an immutable tag are the v1 distribution contract.
Publishing individual workspace crates to crates.io is not part of the 1.0
contract. See the [support matrix](support.md), [upgrade guide](upgrading.md),
and [security policy](../SECURITY.md) for platform tiers and maintenance
windows.
