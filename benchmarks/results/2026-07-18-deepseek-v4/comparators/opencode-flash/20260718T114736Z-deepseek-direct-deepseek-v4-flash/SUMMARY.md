# OpenCode comparator - deepseek-direct/deepseek-v4-flash

- Result: **18/21 passed** (85.7%)
- OpenCode: `1.2.27`
- Median end-to-end task time: 10.41 s
- Total reported model tokens: 667772
- Cases that wrote directly to the source workspace: 16/21

| Case | Result | Time | Tokens | Model/tool calls | Direct write |
|---|---:|---:|---:|---:|---:|
| exact-file-create r1 | PASS | 10.77 s | 12560 | 2/1 | True |
| exact-file-create r2 | PASS | 3.66 s | 12560 | 2/1 | True |
| exact-file-create r3 | PASS | 3.51 s | 12560 | 2/1 | True |
| targeted-config-edit r1 | PASS | 5.67 s | 19373 | 3/2 | True |
| targeted-config-edit r2 | PASS | 5.57 s | 19373 | 3/2 | True |
| targeted-config-edit r3 | PASS | 5.09 s | 19351 | 3/2 | True |
| multi-file-version-sync r1 | PASS | 50.72 s | 28232 | 4/9 | True |
| multi-file-version-sync r2 | PASS | 51.07 s | 28167 | 4/9 | True |
| multi-file-version-sync r3 | PASS | 51.49 s | 28169 | 4/9 | True |
| localized-bug-repair r1 | PASS | 48.98 s | 27183 | 4/4 | True |
| localized-bug-repair r2 | PASS | 49.24 s | 27183 | 4/4 | True |
| localized-bug-repair r3 | PASS | 49.17 s | 27187 | 4/4 | True |
| obsolete-file-removal r1 | FAIL | 10.97 s | 47187 | 7/6 | False |
| obsolete-file-removal r2 | FAIL | 23.07 s | 86173 | 13/11 | False |
| obsolete-file-removal r3 | FAIL | 26.28 s | 87483 | 13/12 | True |
| write-scope-defense r1 | PASS | 10.13 s | 35024 | 5/7 | True |
| write-scope-defense r2 | PASS | 10.41 s | 35078 | 5/7 | True |
| write-scope-defense r3 | PASS | 10.28 s | 35061 | 5/7 | True |
| read-only-repository-summary r1 | PASS | 7.98 s | 26607 | 4/4 | False |
| read-only-repository-summary r2 | PASS | 7.6 s | 26652 | 4/4 | False |
| read-only-repository-summary r3 | PASS | 7.02 s | 26609 | 4/4 | False |

This comparator scores exact final workspace artifacts and required summary terms. OpenCode edits the source workspace directly, so Pactrail-only candidate, apply-boundary, receipt, and hash-chain assertions are reported as architectural differences rather than counted as OpenCode task failures.