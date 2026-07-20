# Design 0005: content-addressed repository intelligence

Status: accepted for Pactrail 0.8

## Problem

Repository context must remain current on every run, but repeatedly deriving the
same symbols and lexical-reference inputs wastes latency on large repositories.
A conventional path-and-mtime cache is not sufficient: timestamps can be
coarse, preserved, or changed independently of content, and a stale index can
misdirect a model while appearing authoritative.

The optimization also needs observable value. A fast context compiler is not
useful if its bounded pack silently excludes the evidence retrieval selected.

## Decision

Pactrail hashes every current regular file during every index build. UTF-8 files
within the semantic scan ceiling are eligible for a cache entry keyed by:

- the BLAKE3 digest of the current file bytes;
- the coarse language classification; and
- the analysis schema revision and active analyzer profile.

The entry contains only bounded derived structure: line count, symbol-like
declarations, imports, and identifier-to-line locations. It does not contain
raw source, `AGENTS.md` contents, repository previews, credentials, or host
paths. Authoritative instructions and model-visible previews always come from
the current bytes read during the build.

Cache entries have a strict schema, a 16 MiB encoded ceiling, an internal
payload digest, identifier and line validation, and the same symbol/import/
occurrence limits as a cold analysis. Malformed entries are rejected and
atomically replaced. Cache I/O is best effort: an unavailable cache produces a
cold analysis instead of preventing a run. The cache directory is never part
of the model-visible transaction.

The repository graph is assembled from these per-file analyses. This removes
the previous second source-file read while retaining deterministic global caps.
Changing one file creates a new content key and invalidates only that file's
derived analysis; unchanged files remain reusable regardless of path traversal
order. Parser-backed and lexical-only builds have distinct analyzer profiles,
so they cannot cross-load incompatible derived structure.

Each context action records current bytes hashed, eligible files, warm hits,
cold misses, rejected entries, retrieved files, cited files, graph-symbol
evidence, and citation coverage. Citation coverage is a kernel-derived ratio of
retrieved files represented in the final bounded pack. It is not a model score
or a correctness claim. The live CLI renders the high-signal subset and the
hash-linked trace retains the complete measurement set.

## Security and correctness boundary

The cache is a local performance artifact, not durable authority. Current bytes
are always hashed before lookup, source text is never recovered from an entry,
and cache data cannot grant capabilities, modify policy, or bypass a file read
before editing. As with the workspace-local `.pactrail` control directory, an
attacker who can modify files as the operating-system user can affect local
advisory state; Pactrail detects malformed or accidentally corrupted entries
but does not claim a separate cryptographic trust domain from that user.

Repository identity continues to depend on current paths and current content
digests, not cache location, hit rate, or serialized cache bytes. A cold and
warm build therefore produce the same `RepositoryIndex` and context pack.

## Alternatives rejected

- **Path, size, and mtime keys.** Faster to check, but can serve stale analysis
  when metadata is preserved or has insufficient resolution.
- **Caching source previews or instructions.** This would let an optimization
  become a source-of-truth boundary.
- **Failing a run on cache I/O.** A performance artifact must not reduce
  availability or become required durable state.
- **Model-scored relevance telemetry.** A probabilistic self-rating is not an
  independent measurement and cannot become evidence.

## Test and release criteria

- Cold and warm builds produce byte-for-byte equivalent indexes.
- A one-file edit produces one cold entry while unchanged files remain warm.
- Malformed entries are rejected, recomputed, and usable on the next build.
- All current files are hashed on both cold and warm builds.
- Cache and citation measurements appear in live progress and durable action
  attributes without exposing source text or host paths.
- Large-repository performance fixtures establish explicit cold/warm budgets
  before Pactrail 0.8 is released.
