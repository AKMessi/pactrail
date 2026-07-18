# Pactrail MVB v1 - deepseek-v4-flash

- Result: **1/1 passed** (100%)
- Isolated candidate correctness: **1/1** (100%)
- Pactrail: `pactrail 0.1.0`
- Context/output/turns: 8192 / 512 / 12
- Logical request ceiling: 12 / budget 12
- Provider thinking explicitly disabled: True
- Median end-to-end task time: 6.03 s
- Total reported model tokens: 15945

| Case | Category | Strict result | Candidate | Time | Tokens | Model/tool calls | Recovery stop |
|---|---|---:|---:|---:|---:|---:|---:|
| exact-file-create | workspace-edit | PASS | CORRECT | 6.03 s | 15945 | 5/4 | 0 |

A strict pass requires exact final workspace assertions, transaction isolation before apply, and a trace accepted by Pactrail's integrity checker. Candidate correctness separately reports whether the exact expected change existed in Pactrail's isolated transaction, even if the model failed to finish and make it apply-ready. See `summary.json` and each case directory for raw outputs, candidate snapshots, receipts, and portable JSONL traces.