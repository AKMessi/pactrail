# Design 0007: parser-backed structure and optional LSP evidence

Status: accepted for Pactrail 0.8

## Problem

The bounded lexical analyzer is portable and predictable, but line prefixes
miss valid declarations with attributes, modifiers, nesting, or multi-line
syntax. Language servers can provide richer reference evidence, but silently
starting repository-selected executables would add process, environment,
workspace, protocol, lifecycle, and replay authority to context construction.

## Decision

### Embedded structural parser

The default `pactrail-context` build embeds the official Tree-sitter runtime and
official grammars for Rust, Python, JavaScript, and TypeScript/TSX. It walks the
concrete syntax tree to derive bounded declarations and import statements.
Parser output remains navigation metadata; it never becomes verification or
policy authority.

Parsing has independent ceilings:

- at most 1 MiB of source enters Tree-sitter;
- parser progress callbacks have a deterministic work limit;
- at most 250,000 named nodes are visited;
- existing per-file symbol and import ceilings still apply; and
- parse-error presence is retained in file and trace telemetry.

Unsupported languages, larger files, parser cancellation, and node-budget
exhaustion use the existing lexical analyzer. Compiling
`pactrail-context --no-default-features` removes every Tree-sitter dependency
and uses the same lexical fallback for all languages. Both configurations have
dedicated tests.

The content-addressed cache key and strict entry payload include an analyzer
profile. A lexical-only build can never consume parser-derived cache data, and
changing grammar/analyzer behavior requires a new profile or cache schema.

### Optional language-server references

Pactrail does not start an LSP during indexing. An operator or static embedder
may separately create an `LspReferenceSnapshot` for the exact current
repository digest and pass it to `RepositoryIndex::apply_lsp_references`.
Snapshots are canonical, bounded, strict-schema, and integrity-bound. Provider
names, symbols, paths, and line locations are validated before any mutation.

The merge accepts only known project symbols, current indexed paths, and valid
line bounds. A lexical location also reported by the language server becomes
`corroborated`; a new location remains `language_server`. Reference provenance
is visible in graph results and initial context. The snapshot digest is folded
into the final context identity so checkpoint reuse cannot confuse two evidence
sets. One index accepts at most one external overlay in 0.8.

This API ingests evidence only. It grants no process or network capability,
does not define an LSP transport, and cannot cause server startup. A future CLI
adapter must separately govern executable identity, initialization,
synchronization, timeout, cancellation, output bounds, and effect tracing.

## Security and failure behavior

Repository text and parser output are untrusted advisory data. Tree-sitter is
an in-process native dependency, so source and work ceilings reduce exposure but
do not create a sandbox. Dependency policy and cross-platform tests cover every
enabled grammar. A parse failure falls back locally; it never weakens path,
policy, transaction, or apply checks.

An invalid or stale LSP snapshot fails before graph mutation. The external
producer is named, its snapshot is digest-bound, and its references cannot
invent files, symbols, or out-of-range locations. Even corroborated references
remain navigation hints and must be followed by a current source read.

## Alternatives rejected

- **Regex growth for every language.** Modifier and nesting variants become an
  unmaintainable approximation of grammar behavior.
- **Make Tree-sitter mandatory.** Minimal embedders and constrained builds need
  a dependency-light deterministic mode.
- **Start an LSP automatically.** Context compilation must not acquire hidden
  process, network, or secret authority.
- **Replace lexical references with LSP results.** Language servers can be
  unavailable, stale, incomplete, or wrong; provenance and fallback must remain
  visible.
- **Trust arbitrary paths from a server.** Every location must map back to the
  exact indexed repository before it can enter model context.

## Test and release criteria

- Parser fixtures cover modifiers/nesting and syntax errors for supported
  grammars.
- Feature-disabled fixtures prove the lexical-only build compiles and passes.
- Oversized input and exhausted parser work fall back deterministically.
- Cold/warm cache equivalence holds within each analyzer profile, and profiles
  cannot cross-load.
- LSP snapshot tampering, stale digests, unknown symbols/paths, invalid lines,
  duplicate overlays, and bounds fail without partial graph mutation.
- Lexical, language-server, and corroborated provenance remains visible through
  graph queries, context, SDK types, and documentation.
