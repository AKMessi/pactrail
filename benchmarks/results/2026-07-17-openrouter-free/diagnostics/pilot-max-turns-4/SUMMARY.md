# Pactrail MVB v1 - poolside/laguna-m.1:free

- Result: **0/1 passed** (0%)
- Pactrail: `pactrail 0.1.0`
- Context/output/turns: 32768 / 2048 / 4
- Logical request ceiling: 4 / budget 4
- Median end-to-end task time: 7.98 s
- Total reported model tokens: 13131

| Case | Category | Result | Time | Tokens | Model/tool calls | Recovery stop |
|---|---|---:|---:|---:|---:|---:|
| targeted-config-edit | precision-edit | FAIL | 7.98 s | 13131 | 4/4 | 0 |

A pass requires exact final workspace assertions, transaction isolation before apply, and a trace accepted by Pactrail's integrity checker. See `summary.json` and each case directory for raw outputs, receipts, and portable JSONL traces.