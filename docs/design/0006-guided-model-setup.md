# Guided model setup

## Problem and design

Provider/endpoint/model/key-variable commands impose unnecessary first-use work.
A single guided CLI flow supplies protocol presets, hidden key input and bounded
catalog search; advanced inputs remain compatible. Entering a first task invokes
setup without losing the task. No inference, downloads or tool actions occur in
setup. There is no OAuth/browser login claim for providers that require API keys.

## Authority and persistence

Only a selected provider's environment key is used. Reading a workspace `.env`
requires explicit consent and selects one variable; repository environment is
never imported wholesale. Temporary credentials are removed on cancellation.
Unix saved credentials reuse the bounded atomic no-follow local-store primitive
and are endpoint-bound; they never derive execution authority or enter runtime
settings serialization. Windows encrypts saved keys using user-scoped DPAPI through the system
PowerShell host, profiles disabled and secrets sent only over pipes. Failure
never falls back to plaintext. No portable ACL claim is made. Environment credentials retain existing explicit user routing
semantics. Process backend and permissions are unchanged.

No event/receipt/checkpoint/settings schema changes. Saved-key references occupy
the existing key-variable field. A missing or mismatched saved key fails clearly;
recovery cannot silently substitute a credential for a different endpoint.
Browser bootstrap exposes only references, not values. Existing browser-local
preferences are retained.

## Verification

Strict all-target Clippy and workspace tests; real-binary PTY workflows for hidden
input, cancellation, restart, catalog search/manual fallback, key removal,
endpoint mismatch, preserved permissions and browser defaults. Existing Ledger
terminal and browser suites remain release gates. No paid model calls required.
