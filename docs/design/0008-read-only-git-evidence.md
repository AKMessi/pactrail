# Design 0008: bounded read-only Git evidence

Status: accepted for Pactrail 0.8

## Problem

Coding agents need repository status, diffs, and local history to understand
intent and avoid overwriting work. Treating `git` as an unrestricted process
tool would also expose configuration includes, filters, hooks, credential
helpers, remote transports, submodule helpers, environment, and every other
authority of the host executable. Git porcelain also conflates state that
Pactrail must keep distinct: the source repository and its isolated candidate
transaction are different trust domains.

## Decision

Pactrail owns a dedicated `pactrail-git` evidence crate. It uses Gitoxide with
default features disabled and only object hashing, index, revision walking,
SHA-1, and SHA-256 support enabled. Pactrail enables none of Gitoxide's command,
network-client, credential, status, or attribute/filter-pipeline features. The
top-level Gitoxide dependency still compiles internal protocol and transport
primitives; Pactrail's inspector is private, exposes none of those types, and
contains no call site for commands, connections, remotes, credential helpers,
hooks, filters, or mutation.

An inspector opens `.git` only when it is a real directory at the exact
authorized source-workspace root. Gitdir pointer files, linked worktrees,
submodule worktrees, and symbolic links are rejected because they can redirect
metadata reads outside that root. Common-directory pointers, object alternates,
HTTP alternates, and redirected critical metadata paths are also rejected.
Parent discovery is forbidden. Open configuration is isolated and strict,
untrusted repositories fail closed, object allocation is capped, and all paths
from Git objects or the index pass through Pactrail's safe relative-path parser
before filesystem access.

The public evidence operations are:

- `status`: separately compares HEAD to index and index to raw worktree bytes,
  reports conflict stages and ignored-aware untracked paths, and returns the
  Pactrail candidate as a separate transaction record at the tool layer;
- `diff`: renders a bounded UTF-8 raw HEAD-to-source-worktree unified diff,
  listing binary and over-budget files instead of embedding them; and
- `history`: walks bounded newest-first commits from HEAD, retains IDs, parent
  IDs, author names, timestamps, and one-line summaries, and omits email.

The comparison is intentionally raw. Pactrail does not execute or emulate clean
filters, textconv, hooks, filesystem monitors, submodule commands, or remote
state. This can differ from command-line porcelain and is labelled as navigation
evidence. Submodules, symbolic links, unknown modes, oversized files, exhausted
hash budgets, and truncated traversals are explicit rather than inferred clean.

## Bounds and failure behavior

- HEAD tree, index, and worktree traversals each stop at a hard entry ceiling.
- HEAD traversal uses an early-cancelling visitor instead of an unbounded
  convenience collector.
- Per-object, per-status-file, aggregate status hashing, per-diff-file,
  diff-file-count, diff-output, history-count, author, and summary bytes are
  capped independently.
- Unsafe or non-Unicode Git paths, malformed objects/indexes, repository trust
  failures, and unexpected I/O fail the call.
- Expected resource exhaustion produces `unscanned`, `omitted_files`, or
  `result_truncated` evidence and never a fabricated clean state.

All three built-ins require `file_read`, are read-only/idempotent/parallel-safe,
and emit effect labels for the durable trace. Adding commit, branch, stash,
remote, hosting, or source-mutation operations requires a separate design and
capabilities; it cannot be smuggled into this crate.

## Alternatives rejected

- **Run Git through the process tool.** Read navigation would inherit excessive
  host and repository-config authority and disappear when processes are
  disabled.
- **Enable Gitoxide's full status pipeline.** Attribute/filter support can
  invoke external filters and expands the boundary beyond deterministic reads.
- **Discover a parent repository.** A workspace-scoped read could then inspect
  siblings the task never authorized.
- **Report candidate edits as worktree changes.** That erases Pactrail's review
  boundary and encourages models to reason about the wrong tree.
- **Guess on resource exhaustion.** Unknown state must remain unknown.

## Test and release criteria

- Fixture repositories cover clean, staged, unstaged, untracked, empty-file,
  history, path-filter, and exact-root behavior.
- Tool tests prove source and candidate evidence remain separate and all three
  descriptors are registered.
- Boundary tests cover unsafe paths, conflicts, binary/oversized files, unborn
  HEAD, traversal/result truncation, malformed inputs, and policy denial.
- The full workspace passes formatting, lint, tests, documentation, dependency
  policy, and supported-platform CI before release.
