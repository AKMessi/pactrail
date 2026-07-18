# OpenCode comparator - deepseek-direct/deepseek-chat

- Result: **1/1 passed** (100%)
- OpenCode: `1.2.27`
- Median end-to-end task time: 11.79 s
- Total reported model tokens: 12536
- Cases that wrote directly to the source workspace: 1/1

| Case | Result | Time | Tokens | Model/tool calls | Direct write |
|---|---:|---:|---:|---:|---:|
| exact-file-create r1 | PASS | 11.79 s | 12536 | 2/1 | True |

This comparator scores exact final workspace artifacts and required summary terms. OpenCode edits the source workspace directly, so Pactrail-only candidate, apply-boundary, receipt, and hash-chain assertions are reported as architectural differences rather than counted as OpenCode task failures.