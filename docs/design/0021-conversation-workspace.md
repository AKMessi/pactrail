# Conversation workspace

The user supplied a DeepSeek Harness screenshot showing a workspace tree, compact title, Chat/Trajectory navigation, a readable conversation, bottom input and usage footer. This follows that reference on Pactrail run pages rather than just the new-task composer.

Conversation renders actual contract goal, browser-stored summary or explicit missing/live-state text. Trace remains the durable execution view, and Changes/Evidence/Receipt remain separate governed review views. Legacy Answer URLs are accepted as Conversation aliases. Review retains its default Changes view.

The next-task composer prepares a new draft and opens setup; it cannot imply same-run continuation or hidden context that the web API does not provide. Draft is browser-local. Polling updates stable status/result nodes and never rebuilds the input or unchanged summary. No model-provided HTML, authority, API, provider or schema change.

The v2.1.1 tag workflow preceded this screenshot; ship this closer match as v2.1.2 without rewriting either previous tag. Verification requires design/projection tests, full browser suite with both-theme screenshots, real-engine fixtures, strict Rust gates and exact-commit CI before tagging.

Browser verification passed: ten design/projection tests, 76 screenshots, 2,007 trace rows, 13.2 ms per 50-event append, zero browser errors, next-task preparation and unchanged-editor/draft preservation. Desktop light and mobile dark Conversation screenshots were visually reviewed. Retained evidence is under `benchmark-results/v2.1.2-web/` (ignored). Rust, engine fixtures and exact-commit CI remain publication gates.
