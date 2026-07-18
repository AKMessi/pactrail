# Real Issue Replay v1

This suite compares coding-agent harnesses on real repository states and real
historical defects. It is intentionally separate from Pactrail's deterministic
MVB smoke suite.

Each task starts at the commit immediately before its reference fix. The agent
receives the public issue behavior but not the reference patch or hidden tests.
After the run, the evaluator overlays the test file from the pinned reference
commit into a disposable grading workspace, runs the targeted regression test,
and runs a broader deterministic regression command. Exact patch similarity is
not scored.

The three preregistered tasks cover:

- a signed-buffer edge case in `tokio-rs/bytes`;
- multi-pattern CLI validation in `sharkdp/fd`;
- durable transaction recovery in Pactrail itself.

The two tiny setup patches only neutralize known Windows-only failures in
unrelated benchmark/CLI tests. They are part of the synthetic baseline commit,
are disclosed in the manifest, and are overwritten by the hidden grader where
relevant. The fd grader independently skips the same two unrelated Windows
cases. Bytes disables nightly-only auto-discovered benches so Pactrail's fixed
`cargo test --workspace --all-targets` verification command can run on stable.

## Matched controls

- DeepSeek V4 Flash and V4 Pro through the direct API;
- temperature `0`, thinking explicitly disabled;
- 16,384-token context, 1,024-token output, 12 steps/turns;
- pass@1 with one fresh workspace and session per task;
- arbitrary process execution enabled for both harnesses;
- an offline task policy, a single synthetic baseline Git commit, and no remote;
- identical task text and behavioral graders;
- no retries, best-of-n selection, or post-hoc replacement;
- a 600-second wall limit and a $1.05 account-balance floor.

Pactrail's candidate isolation, apply boundary, receipt, and hash-chain are
measured separately. OpenCode is not marked wrong for lacking those concepts;
functional task score comes only from the hidden target test and broader
regression command.

The manifest is the preregistration record. Commit it before any scored model
call, and publish raw JSONL, model/tool usage, patches, grader output, source
locks, balance snapshots, and checksums with the final report.
