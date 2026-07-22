# ADR 0013: Stale-resistant receipt memory

## Status

Accepted for Pactrail 1.1.

## Context

An integrity hash proves that a historical receipt was not modified. It does
not prove that the files supporting that history still match the current
workspace. Ranking receipt summaries beside current human guidance without a
freshness check lets old, lexically relevant history masquerade as present
fact. Reusing an external state directory also makes implicit workspace
assumptions unsafe.

## Decision

Memory database schema two stores a bounded list of every changed path and its
post-apply BLAKE3 digest (or absence for a deletion), plus an explicit
completeness bit. The record remains bound to its applied receipt run ID and
integrity hash.

Recall keeps relevance, trust, and freshness independent:

- user-authored memory is `user_asserted` and `advisory`;
- receipt memory with a complete, exactly matching anchor set is
  `receipt_verified` and `current`;
- any path/digest mismatch is `stale`;
- legacy or over-limit receipt history is `unverified`.

The transaction resolves every anchor as a safe workspace-relative path and
streams its digest; no process or model is involved. Only advisory human memory
and current receipt memory enter model context. Stale and unverified receipt
records are counted but withheld. Retrieval validates a wider bounded pool
before applying the requested result limit so stale high-ranking entries cannot
starve current lower-ranking entries.

Schema one remains readable. Migration adds the anchor columns inside one
SQLite immediate transaction and marks existing records incomplete rather than
fabricating evidence.

## Consequences

Historical run summaries no longer silently survive source drift as current
evidence. Deleted paths are validated as absence. A directory, symbolic link,
unsafe path, or I/O failure fails recall closed instead of weakening freshness.
Human memories remain useful but never gain receipt trust. Very large applied
runs above the 2,048-anchor bound remain durable history but are not injected
into a model as current evidence.
