# Pactrail MVB v1 - ternary-bonsai-27b-q2

- Result: **1/1 passed** (100%)
- Isolated candidate correctness: **1/1** (100%)
- Pactrail: `pactrail 0.1.0`
- Context/output/turns: 8192 / 512 / 8
- Logical request ceiling: 8 / budget 8
- Median end-to-end task time: 216.39 s
- Total reported model tokens: 6187

| Case | Category | Strict result | Candidate | Time | Tokens | Model/tool calls | Recovery stop |
|---|---|---:|---:|---:|---:|---:|---:|
| exact-file-create | workspace-edit | PASS | CORRECT | 216.39 s | 6187 | 2/1 | 0 |

A strict pass requires exact final workspace assertions, transaction isolation before apply, and a trace accepted by Pactrail's integrity checker. Candidate correctness separately reports whether the exact expected change existed in Pactrail's isolated transaction, even if the model failed to finish and make it apply-ready. See `summary.json` and each case directory for raw outputs, candidate snapshots, receipts, and portable JSONL traces.