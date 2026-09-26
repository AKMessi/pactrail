# Space Bunny low-reasoning calibration

This one Pactrail pass@1 trial was frozen in
[`protocol-v4-calibration.json`](../../issue-replay-space-bunny-v1/protocol-v4-calibration.json)
before the first model call. It is a calibration on the known regex regression,
not a paired comparison or a measure of broad coding ability. `raw/` retains
the full selected result, patch, trace, and test outputs; `SHA256SUMS` covers
those files.

Pactrail requested the model's supported `low` reasoning effort, 32,768
context tokens, 8,192 output tokens, and 24 turns. It produced a focused
one-file, 10-line-addition patch in 11 model calls and 10 tool calls. The
candidate was ready to apply, source isolation and trace integrity held, and
the applied candidate matched the patch. It reported 172,108 tokens and ran
for 322.56 seconds.

**Functional and strict result: fail.** The hidden targeted test found a
second out-of-bounds slice at line 2582 in the same file. The candidate fixed
one copy path but did not cover the full public API behavior. The model's own
summary claimed its tests passed; the hidden grader contradicts a complete
fix. This result shows movement from the earlier read-only runs to an actual
candidate, while exposing a verification and completeness gap. It cannot be
compared directly with revision 3 because reasoning effort, context, output
cap, and turn limit all changed.
