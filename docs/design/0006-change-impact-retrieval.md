# Design 0006: bounded change-impact retrieval

Status: accepted for Pactrail 0.8

## Problem

Filename and symbol search can locate the obvious edit site, but a coding agent
also needs to inspect nearby consumers and definitions before changing it. A
model can guess those relationships, but its guess is neither complete nor
independent evidence. Full type-resolved dependency analysis is language- and
toolchain-specific and cannot be a hidden runtime requirement.

## Decision

Pactrail derives a deterministic one-hop impact view from its bounded repository
graph. Direct task matches become seed files. For each seed, the query finds:

- files with references to project symbols defined by the seed; and
- files defining project symbols referenced by the seed.

Each related path carries a stable score and explicit reasons naming the symbol
relationship and its lexical, language-server, or corroborated provenance.
Results are capped by caller-supplied bounds under hard kernel ceilings, sorted
by score and portable path, and propagate graph truncation. They are labelled
navigation evidence, never type-resolved dependency or proof of runtime impact.

The initial context compiler adds bounded impact entries after symbol evidence
and before topology. The `search_change_impact` tool rebuilds from the current
isolated candidate on every invocation, so an edit in a prior tool turn cannot
leave the model navigating a stale graph. The tool is read-only,
parallel-safe, requires `file_read`, and directs the model to inspect cited
source before editing.

Context telemetry records related paths that fit the pack. The hash-linked tool
action separately records its descriptor, argument digest, bounds, result size,
truncation, and observed repository-read effect through the standard Tool
Kernel.

## Alternatives rejected

- **Let the model infer impact from filenames.** This is expensive for small
  models and cannot be measured independently.
- **Describe the lexical graph as a call graph.** Identical identifiers can be
  unrelated; the UI and protocol must preserve that uncertainty.
- **Require an LSP.** An explicit optional snapshot can enrich references, but
  repository navigation must work deterministically without a server.
- **Cache the tool result across edits.** Candidate-aware inspection is more
  important than avoiding a bounded local rebuild.

## Test and release criteria

- Definition seeds find bounded referencing files.
- Consumer seeds find bounded defining files.
- Ordering, reasons, caps, and truncation are deterministic.
- The model-visible descriptor and output preserve reference provenance and
  state the navigation-evidence limitation.
- A candidate edit is reflected by the next query.
- Initial context and durable telemetry expose impact without treating it as a
  correctness grade.
