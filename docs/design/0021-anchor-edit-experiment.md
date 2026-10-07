# Design 0021: revision-bound anchor editing experiment

Status: specification only. No production tool is registered and no default has
changed. Design 0020's completed model-baseline gate still applies. This document
defines the smallest experiment rather than presuming an editing improvement.

## Hypothesis and control

A model can replace a bounded source region without reproducing its existing
text. Compare existing exact editing with anchored editing on the same admitted
development issues, models, total limits, authority and external graders. Only
tool documentation and available edit protocol differ. Freeze a new protocol
before either scored arm. Confirmation tasks cannot guide design iterations.

Primary outcomes: strict completion and targeted/regression correctness. Secondary:
edit failures, generated tokens, repair attempts, turns, patch scope and latency.
A smaller request is not proof of fewer billed tokens or improved task success.
Retain malformed calls, stale references and aborted trials in their original arms.

## Proposed minimal interface

Two experimental tools: `read_anchored` and `anchor_edit`. Existing registry stays
unchanged until an explicit experimental opt-in exists. Tools operate through
`ToolContext`, normal FileRead/FileWrite policy, workspace resolution and the
candidate transaction. There is no alternate writer or source-tree access.

A read returns path, opaque revision, bounded numbered lines and content-derived
anchors. The revision binds schema, durable run identity, workspace-relative path
and full exact-byte file digest. A tool needs a run-scoped context; SDK calls without
one fail explicitly rather than producing portable unbound edit credentials.

An edit supplies path, revision, inclusive start/end anchors and replacement.
Each anchor contains ordinal plus a content-derived hash. Ordinal alone is never
accepted. Revision equality, current line bytes, both anchors, range ordering and
all resource bounds must pass before constructing the replacement. No fuzzy
match, stale relocation or automatic fallback is allowed. An unrelated write to
the same file invalidates the revision. File revisions are deliberately narrower
than whole-repository revisions; edits to other files do not invalidate a read.

Use domain-separated BLAKE3 over exact bytes. Start with a 16-hex anchor prefix.
During reading, detect prefixes shared by unequal line bytes and extend the hash
until unambiguous, up to the full digest; repeated identical lines remain
unambiguous through ordinal plus revision. Reject ambiguous externally supplied
short prefixes. Truncated output must never create an anchor for omitted bytes.

## Byte and newline semantics

Index original UTF-8 with byte offsets from `split_inclusive('\n')`. CRLF belongs
to the exact line bytes, as does the presence/absence of the final newline. Do not
reconstruct untouched lines through `lines().join(...)`. Replacement bytes are
explicit: preserve them exactly, with no implicit newline normalization. Tool
instructions must state that replacing a newline-terminated region normally
requires a terminating newline in the replacement. Unicode boundaries are
verified before slicing. Reject non-UTF-8 source and replacement.

Initially support replacement and deletion of existing inclusive ranges only.
Insertion uses an explicitly empty replacement boundary in a later separately
specified operation; do not overload a reversed range. Empty/new files continue
through existing `write_file`. This avoids ambiguous EOF/empty-file sentinels.

## Bounds, effects and concurrency

Use the existing eight-MiB mutation ceiling, a smaller bounded read window and a
bounded total output budget. Scan offsets without allocating strings per line.
Check replacement and resulting byte limits before allocation. A giant line may
be refused with an actionable error; never return an editable partial line.

Mutations remain serialized by the engine's existing authority schedule. This
experiment does not support concurrent writers or a privileged compare-and-swap
API. Revalidate current bytes immediately before the existing atomic transaction
write; approved native commands must not overlap a mutation. External candidate
mutation remains outside the supported execution contract and fails recovery
validation. Document this assumption instead of claiming race-proof host edits.

Return before/after digest, revision, changed range and existing bounded mutation
feedback. These attributes flow through the normal action record and effect
fences. Include normal `fs.write:<path>` provenance. Anchor observations are
navigation data, not verification evidence. Crash/replay safety continues to use
existing tool intent/completion fencing; an uncertain write must not replay.

## Offline adversarial qualification

Before model evaluation test:

- repeated identical lines and forced short-hash collision fixtures;
- stale file digest, wrong run, wrong path and invalid ordinal/hash combinations;
- Unicode, CRLF, mixed newlines and missing final newline;
- empty files, giant single lines, maximum ranges and output/result overflow;
- traversal, symlinks, forbidden paths and read-only mutation attempts;
- poisoned source text that looks like anchors or instructions;
- preservation of every byte outside the edited region;
- source isolation and recorded before/after provenance;
- checkpoint/effect recovery and single-agent/default registry compatibility.

Property tests should compare edit results with exact byte splicing and reject
malformed anchors without panic or large allocation. A test-only shortened hash
can exercise collision resolution without weakening the production hash.

## Retention gate

PROMOTE only with material edit reliability/token benefit and no correctness or
authority regression on a frozen confirmation comparison. KEEP EXPERIMENTAL if
uncertainty remains. REMOVE if the protocol worsens performance or duplicates
exact editing without useful benefit. No A/B result exists yet; this specification
is neither implementation nor evidence that anchors are better.
