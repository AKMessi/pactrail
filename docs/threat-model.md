# Threat model

## Trust boundaries

Pactrail treats all of the following as untrusted:

- model output and tool arguments;
- repository source and generated files;
- ordinary file contents that resemble instructions;
- provider, plugin, and tool output;
- terminal control sequences;
- concurrent changes in the source workspace.

The explicit task contract and applicable `AGENTS.md` files are instructions,
but they cannot override capability policy or path confinement.

## Defended properties

- Typed file tools cannot resolve absolute paths, platform prefixes, or parent traversal.
- Write tools cannot leave the contract's path prefixes.
- Symbolic links and special files are rejected conservatively.
- The source workspace is not written before explicit apply.
- Apply refuses a touched path changed by the user after the baseline snapshot.
- A receipt or event modified after creation fails its integrity check.
- Provider keys are read from environment variables and are never accepted as CLI values.
- Remote model endpoints require HTTPS; plain HTTP is restricted to exact loopback hosts.
- Process environments are cleared and rebuilt from a small operational allowlist.
- Process execution has time and retained-output limits and avoids shell interpolation.
- Network, secret, external-write, and process capabilities fail closed unless granted.
- Runtime policy cannot grant a capability absent from the durable task contract.

## Important limitation: native processes

`--allow-process` is an explicit trust decision. A native child process is run in
the transaction directory with a scrubbed environment, but it is not confined by
an operating-system or container sandbox. It may attempt to read other host files,
discover secrets, use the network, or mutate external state. Pactrail therefore
records process, network, secret-use, and external-write authority in the task
contract; task files that understate this authority are rejected.

Do not enable native processes for an untrusted repository. Use `pactrail doctor`
to inspect available container runtimes and prefer the forthcoming OCI backend
for hostile inputs.

## Out of scope for the current developer preview

- defending the provider itself from prompts sent by the user;
- cryptographic non-repudiation of a local receipt;
- protecting a compromised operating-system account;
- safely executing arbitrary native code without an external sandbox;
- automatic secret brokering;
- remote side effects such as pull requests or deployments.

Report suspected vulnerabilities through GitHub private vulnerability reporting,
as described in `SECURITY.md`.
