# Design 0015: stable tool catalog and progress-led execution

Status: accepted for the post-1.0 controller repair

## Evidence

The preregistered post-upgrade real-issue replay completed zero of six Pactrail
trials. In all six, phase narrowing removed read and search tools before the
model had enough evidence to edit. Twenty subsequent requests were rejected and
no production mutation call was made. The isolated candidate and trace
boundaries held, but the controller prevented task completion.

## Decision

Every model turn advertises the configured tool descriptors in a stable order.
The kernel still rejects unregistered names and independently enforces schema,
capability, approval, workspace, and effect policy. Phase transitions guide the
model toward an edit, targeted missing fact, validation, or final answer; they
do not revoke ordinary evidence tools. Existing turn, token, tool-call, output,
semantic-stall, and repeated-call limits bound exploration.

This avoids reserializing a different tool schema at each phase and preserves
the opportunity for provider prefix-cache reuse. The security boundary remains
the kernel and process backend, not the text of a phase prompt.

## Recovery and compatibility

Phase and semantic progress are reconstructed from the checkpointed
conversation as before. Resume sees the same tool catalog and never replays an
uncertain effect. No durable format changes. Previously recorded traces and
receipts retain their historical meaning.

## Verification

Scripted-model tests keep search available after entering implementation,
verify it executes rather than receiving a phase rejection, and assert that
tool names remain stable across turns. The held-out paid replay is deferred
until the broader architecture upgrade is complete; no correctness claim is
inferred from the offline regression test alone.
