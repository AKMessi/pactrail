# Design 0012: strict unified patch mutation

Status: implemented for the next 1.x release

## Problem

Exact text replacement is safe and efficient when a model knows the precise
substring. Full-file writes are reliable for small files. Neither is ideal for
line-oriented changes with several nearby additions and deletions: exact
replacement duplicates large source spans in JSON, while a complete write
increases token cost and risks dropping unseen content.

Calling `git apply`, `patch`, or a shell would make ordinary editing depend on
process authority and platform-specific executables. Fuzzy patch application is
also unsuitable for a transactional harness because a plausible nearby match is
not evidence that the intended region was changed.

## Decision

Pactrail provides a native `apply_patch` tool for one file per call. Its input
is a strict unified diff with exactly two file headers followed by one or more
hunks. It supports:

- update headers with the same workspace-relative old/new path;
- additions from `/dev/null`;
- deletions to `/dev/null`;
- standard old/new ranges, including zero-count insertion boundaries;
- context, removal, addition, and `No newline at end of file` lines; and
- an optional complete BLAKE3 digest of the current source file.

Renames are deliberately excluded; models use a bounded read, write, and remove
sequence so each filesystem effect stays explicit.

## Exact application

Patch bytes, line count, hunk count, source size, and resulting size have hard
limits. Every hunk header count must equal its body. Old ranges must be ordered
and non-overlapping. New ranges must equal the actual number of output lines
preceding the hunk. Every context and removal line must exactly equal the source
at the declared line; there is no offset search, whitespace relaxation, or
fuzz factor.

The complete patch is parsed and applied to an in-memory UTF-8 document before
the transaction receives a write or delete. A mismatch returns the hunk number,
line number, and bounded expected/actual text. A stale optional digest fails
before application. No-op results are rejected.

## Text fidelity

An existing file must use consistent LF or CRLF endings. Patch transport may use
LF or CRLF independently. Updated content preserves the source convention;
added files use LF. A final no-newline marker is honored only at the end of the
new file and malformed mid-hunk use is rejected. Mixed source endings fail
instead of silently normalizing unrelated lines.

## Policy and traces

File paths pass through the same safe relative-path and write-scope boundary as
every other workspace tool. Absolute paths, drive prefixes, parent traversal,
symlinks, unsupported files, and out-of-scope writes fail closed. The tool never
starts Git, a shell, or any process.

Success returns operation, hunk and line counts, result bytes/digest, and the
standard bounded post-edit current-source evidence. The engine independently
reconciles changed paths, effect-fences the call, and binds it into the
transaction receipt.

## Rejected alternatives

- **External `patch` or `git apply`:** adds process authority and platform state.
- **Fuzzy offsets:** can modify a plausible but unintended repeated block.
- **Multi-file patch calls:** validation is easy, but an I/O failure between
  file writes would require a second candidate-level transaction protocol.
- **Silently normalize line endings:** creates unrelated whole-file churn.
- **Accept Git metadata and renames:** expands the parser and effect model
  without improving the core line-edit primitive.

## Verification

Tests cover multi-hunk updates, zero-count insertion, add/delete, CRLF, explicit
no-final-newline output, context mismatch, stale digests, traversal, rename and
header rejection, mixed line endings, unchanged-candidate failure, effect-fence
and receipt integration, descriptor budgets, and arbitrary bounded parser input.
The full workspace, strict Clippy, rustdoc, and platform release gates remain
authoritative.
