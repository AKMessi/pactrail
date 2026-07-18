# Pactrail MVB v1 - ternary-bonsai-27b-q2

- Result: **0/1 passed** (0%)
- Isolated candidate correctness: **0/1** (0%)
- Pactrail: `pactrail 0.1.0`
- Context/output/turns: 8192 / 512 / 8
- Logical request ceiling: 8 / budget 8
- Median end-to-end task time: 300.1 s
- Total reported model tokens: 0

| Case | Category | Strict result | Candidate | Time | Tokens | Model/tool calls | Recovery stop |
|---|---|---:|---:|---:|---:|---:|---:|
| exact-file-create | workspace-edit | FAIL | INCORRECT | 300.1 s | 0 | 0/0 | 0 |

A strict pass requires exact final workspace assertions, transaction isolation before apply, and a trace accepted by Pactrail's integrity checker. Candidate correctness separately reports whether the exact expected change existed in Pactrail's isolated transaction, even if the model failed to finish and make it apply-ready. See `summary.json` and each case directory for raw outputs, candidate snapshots, receipts, and portable JSONL traces.