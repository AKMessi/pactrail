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
atomically migrated, and settings are migrated through the normal crash-safe
persistence path. A manifest-only placeholder therefore cannot satisfy the
compatibility gate.

Audit the actual workspace state and interactive settings before an upgrade:

```console
pactrail migrate
pactrail migrate --json
```

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

## Change rules before 1.0

Until the stable 1.0 contract is declared, a minor release may introduce a new
schema. Such a change must include:

1. an updated entry in `pactrail compatibility --json`;
2. a checked-in current or historical fixture exercised by its production
   reader;
3. tests for the oldest supported reader and an unknown-future rejection;
4. documented atomicity, rollback, and authority behavior; and
5. a release-note migration section.

Removing a readable schema requires a deprecation notice in at least one prior
minor release. Security fixes may reject a previously accepted unsafe format
immediately; the release notes must say why.

The manifest describes format compatibility, not binary downgrade safety. Do
not open state with an older Pactrail binary after a newer binary has migrated
it unless that older release explicitly declares the resulting schema readable.
