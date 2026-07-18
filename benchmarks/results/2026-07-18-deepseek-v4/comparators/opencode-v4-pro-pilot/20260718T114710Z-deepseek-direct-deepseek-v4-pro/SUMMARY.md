# OpenCode comparator - deepseek-direct/deepseek-v4-pro

- Result: **1/1 passed** (100%)
- OpenCode: `1.2.27`
- Median end-to-end task time: 12.18 s
- Total reported model tokens: 12556
- Cases that wrote directly to the source workspace: 1/1

| Case | Result | Time | Tokens | Model/tool calls | Direct write |
|---|---:|---:|---:|---:|---:|
| exact-file-create r1 | PASS | 12.18 s | 12556 | 2/1 | True |

This comparator scores exact final workspace artifacts and required summary terms. OpenCode edits the source workspace directly, so Pactrail-only candidate, apply-boundary, receipt, and hash-chain assertions are reported as architectural differences rather than counted as OpenCode task failures.