# Space Bunny first scored trial

The first declared pass@1 case under
[`issue-replay-space-bunny-v1`](../../issue-replay-space-bunny-v1/README.md)
failed: Pactrail completed 0/1 strict tasks. The model made no completed
turns and no tool calls. OpenRouter emitted two identical `tool_calls` finish
markers in one streamed response; Pactrail rejected the second marker as
malformed and stopped. The exact failed result is retained. It is not removed
or replaced by a later run.

The run preserved source isolation and its portable trace verified. The
behavioral grader failed, and no candidate was ready to apply. This result
identifies a provider-stream compatibility defect, not evidence about Space
Bunny's ability to solve the Rust issue.

The selected raw result, trace, test output, and runner summary are in this
directory with `SHA256SUMS`. The full unchanged upstream source tree is omitted
because the pinned suite can reconstruct it; the patch artifact is empty. The
subsequent adapter fix is revision `508eaba` and is evaluated under a separate
protocol so this pass@1 failure remains visible.
