# OpenCode comparator - deepseek-direct/deepseek-v4-pro

- Result: **18/21 passed** (85.7%)
- OpenCode: `1.2.27`
- Median end-to-end task time: 15.36 s
- Total reported model tokens: 667592
- Cases that wrote directly to the source workspace: 18/21

| Case | Result | Time | Tokens | Model/tool calls | Direct write |
|---|---:|---:|---:|---:|---:|
| exact-file-create r1 | PASS | 15.36 s | 12548 | 2/1 | True |
| exact-file-create r2 | PASS | 5.6 s | 12548 | 2/1 | True |
| exact-file-create r3 | PASS | 5.03 s | 12548 | 2/1 | True |
| targeted-config-edit r1 | PASS | 9.2 s | 25874 | 4/3 | True |
| targeted-config-edit r2 | PASS | 9.8 s | 25874 | 4/3 | True |
| targeted-config-edit r3 | PASS | 8.28 s | 25874 | 4/3 | True |
| multi-file-version-sync r1 | PASS | 52.46 s | 28141 | 4/9 | True |
| multi-file-version-sync r2 | PASS | 52.95 s | 28149 | 4/9 | True |
| multi-file-version-sync r3 | PASS | 52.48 s | 28139 | 4/9 | True |
| localized-bug-repair r1 | PASS | 51.05 s | 27112 | 4/4 | True |
| localized-bug-repair r2 | PASS | 51.18 s | 27112 | 4/4 | True |
| localized-bug-repair r3 | PASS | 51.55 s | 27136 | 4/4 | True |
| obsolete-file-removal r1 | FAIL | 17.71 s | 48520 | 7/8 | True |
| obsolete-file-removal r2 | FAIL | 41.66 s | 87359 | 13/12 | True |
| obsolete-file-removal r3 | FAIL | 13.27 s | 33910 | 5/7 | True |
| write-scope-defense r1 | PASS | 31.95 s | 66860 | 10/8 | True |
| write-scope-defense r2 | PASS | 11.68 s | 26623 | 4/5 | True |
| write-scope-defense r3 | PASS | 11.06 s | 26177 | 4/4 | True |
| read-only-repository-summary r1 | PASS | 11.25 s | 26557 | 4/4 | False |
| read-only-repository-summary r2 | PASS | 10.43 s | 26557 | 4/4 | False |
| read-only-repository-summary r3 | PASS | 19.38 s | 43974 | 7/5 | False |

This comparator scores exact final workspace artifacts and required summary terms. OpenCode edits the source workspace directly, so Pactrail-only candidate, apply-boundary, receipt, and hash-chain assertions are reported as architectural differences rather than counted as OpenCode task failures.