# Space Bunny architecture regression, revision 3

This is the six-trial pass@1 run frozen by
[`protocol-v3.json`](../../issue-replay-space-bunny-v1/protocol-v3.json).
The three known Rust defects were each attempted once by Pactrail at `d4d9579`
(the frozen release binary has SHA256
`9c512a20c9041c263c5673a2fe296ac29d66eeab4a9660cfa09d2fc3e69c2fec`)
and once by OpenCode 1.18.32 with the same free OpenRouter model and declared
limits. Every trial is retained in `raw/`; `comparison.json` is the
machine-readable comparison, and `SHA256SUMS` covers the selected raw files.

| Harness | Functional | Strict | Reported tokens | Tool calls | Agent time |
| --- | ---: | ---: | ---: | ---: | ---: |
| Pactrail | 0/3 | 0/3 | 502,433 | 58 | 157.64 s |
| OpenCode | 0/3 | 0/3 | 508,767 | 62 | 316.45 s |

Neither harness edited source or passed a hidden behavioral grader. Pactrail's
periodic action deadlines were delivered in all three cases, but did not yield
an edit. Its regex run exhausted 16 model turns. The HTTP and ripgrep runs ended
on a `finish_reason=length` response with no visible text or tool calls after
using the full 2,048-token output cap. These were output-limit/protocol exits,
not provider transport errors. The corrected runner records `provider_errors=0`
and excludes generated, ignored `Cargo.lock` files from changed paths.

Pactrail preserved source isolation and verified its trace in all three runs.
Its lower wall time and slightly fewer reported tokens do not establish a
coding-task efficiency win when both harnesses solved 0/3. The model's actual
reasoning mode is also uncertain: Pactrail requested the provider-specific
`thinking.type=disabled` extension, which may not control OpenRouter's
`reasoning` parameter. That setting needs verification before drawing a
reasoning-mode comparison or changing the protocol again.

This suite measures three known regressions and cannot establish broad
coding-agent superiority. It identifies a remaining action-loop problem and
an output-cap failure mode that need separate fixes and fresh evaluation.
