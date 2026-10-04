# Web workspace refresh

## Reference and intent

Inspected DeepSeek Harness `ui-conversation` EmptyHero/HeroShell, shared input ownership, and `ui-sidebar` SidebarRoot in the local reference checkout, alongside its public repository. Borrow the centered new-session composition, quieter sidebar, and progressive configuration; retain Pactrail branding and governed review model. No source, logos, dependencies, or runtime architecture copied.

## Interaction

A centered workspace heading leads to a single task card. The brief and dispatch share the card; starter actions follow it. An initially open disclosure keeps safety choices visible and lets the user fold run setup beneath the card, so configuration is available without dominating the task. Commands remain visible with their consequences. Host acknowledgement remains required and never persisted. The existing run workspace uses a compact header, ledger, and separate Trace/Changes/Evidence/Receipt tabs. No fabricated chat, model reasoning, tools, or latent functionality.

## Boundaries

Vanilla embedded assets, loopback-only API, origin checks, CSP, textContent-only engine strings, existing pricing conversion, unknown-number semantics, state machine, streaming deduplication/focus/scroll preservation, confirmation dialogs and evidence colors remain unchanged. Existing engine/API behavior is authoritative.

## Verification and release

Run projection/design tests, browser workflow and viewport matrix with screenshots, real-engine fixture workflows, Rust web tests and exact-commit CI. Review desktop/mobile screenshots. v2.1.0 was already tagged and publishing before this request; ship the refresh as v2.1.1 without moving that tag.

## Reference source

https://github.com/deepseek-ai/deepseek-harness — inspected hero and sidebar source; no copied implementation. The benchmark/research architecture is outside the scope of this visual patch.

## Browser results

Ten design/projection unit tests passed, including both-theme contrast, CSP restrictions, honest usage, pricing conversion and evidence gating. The browser suite passed with 76 screenshots, no browser errors, 2,007 trace rows and a 50-event append of 18.9 ms. It covers reconnect scroll/selection/expansion/filter preservation, dialogs, file review, responsive geometry and editor identity/draft continuity while folding setup. Artifacts: `benchmark-results/v2.1.1-web/` (ignored) and `/tmp/pactrail-workspace-final-qa`. Desktop light, mobile dark and review screenshots were visually inspected. No engine changes. Rust, real-engine fixtures and publication are separate release gates and must complete before reporting publication successful.
