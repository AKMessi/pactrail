# Pactrail post-upgrade real-issue regression

This is a preregistered matched-harness regression of Pactrail's architectural
upgrade at implementation revision
`74c5ccdaab89ede52bc63056e18a2f37398dfef5`. It replays the three real Rust
defects on which Pactrail 1.0.0 previously passed 0/6 trials because its loop
never transitioned from investigation to mutation.

The comparison uses Pactrail's release build from that revision and the current
OpenCode release, with the same DeepSeek model, prompt, non-thinking mode,
temperature zero, 16,384-token context, 2,048-token output cap, 16 model steps,
offline dependencies, unrestricted trusted process execution, and hidden
behavioral graders. Each declared trial is pass@1: no retry, replacement,
best-of-n selection, or prompt repair is allowed.

The primary V4 Flash order is paired and counterbalanced:

1. regex / Pactrail
2. regex / OpenCode
3. HTTP / OpenCode
4. HTTP / Pactrail
5. ripgrep / Pactrail
6. ripgrep / OpenCode

The V4 Pro robustness replication reverses harness order:

1. regex / OpenCode
2. regex / Pactrail
3. HTTP / Pactrail
4. HTTP / OpenCode
5. ripgrep / OpenCode
6. ripgrep / Pactrail

The runner checks the DeepSeek balance before every model case and refuses to
start another case below $1.08. An incomplete or failed case is retained and is
never replaced.

## Interpretation boundary

These tasks are no longer held out: their earlier failures directly motivated
the phase controller, semantic progress detector, strict patch tool, proactive
verification, and adaptive runtime. The experiment can establish whether those
changes repaired the measured failure mode and whether the upgraded harness
beats OpenCode on this exact regression set. It cannot establish broad harness
superiority.

The implementation, executable, comparator version, suite files, prices, order,
and stopping rule are frozen in `protocol.json` before the first paid request.
