# Rust embedding API

Pactrail's SDK is a static Rust composition surface for applications that want
the real Pactrail kernel without the bundled CLI. It is not a native plugin
loader: the host chooses and links model and tool implementations at build time,
so a repository cannot cause arbitrary extension code to load.

The facade crate is `pactrail-sdk`. During the pre-1.0 series it is consumed
from the repository or a pinned Git revision; crates.io publication and the
SemVer stability window are v1 release work. `SDK_API_REVISION` is currently 3.

## Custom model provider

Implement `ModelDriver` to translate a provider into Pactrail's ordered,
provider-neutral request and complete-response types:

```rust,no_run
use pactrail_sdk::model::{FinishReason, Usage};
use pactrail_sdk::prelude::*;

struct Provider {
    capabilities: ModelCapabilities,
}

#[async_trait]
impl ModelDriver for Provider {
    fn name(&self) -> &str { "my-provider" }
    fn model(&self) -> &str { "my-model" }
    fn capabilities(&self) -> &ModelCapabilities { &self.capabilities }

    async fn invoke(&self, request: &ModelRequest) -> Result<ModelResponse, ModelError> {
        // Perform bounded transport and normalize one complete turn here.
        let _ = request;
        Ok(ModelResponse {
            text: "done".into(),
            tool_calls: vec![],
            finish_reason: FinishReason::Complete,
            usage: Usage::default(),
            provider_request_id: None,
            extensions: serde_json::Map::new(),
        })
    }
}
```

Drivers own protocol parsing, response-size limits, credentials, and transport
timeouts. They return typed tool calls but never execute them. Implement
`invoke_with_observer` only when the provider has a true bounded streaming
transport; the final `ModelResponse` remains the sole execution authority.

## Custom tool

Implement `Tool` with a stable JSON Schema descriptor and enforce the declared
capability inside `execute` before observing or changing anything:

```rust,no_run
use pactrail_sdk::prelude::*;
use serde_json::{Value, json};

struct WorkspaceFact;

#[async_trait]
impl Tool for WorkspaceFact {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "workspace_fact".into(),
            description: "Return one deterministic workspace fact.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            required_capability: Capability::FileRead,
            annotations: ToolAnnotations::READ_ONLY,
        }
    }

    async fn execute(
        &self,
        context: &ToolContext<'_>,
        _input: Value,
    ) -> Result<ToolOutput, ToolError> {
        context.authorize(&Capability::FileRead, ".", "workspace_fact")?;
        Ok(ToolOutput {
            content: json!({"fact": "bounded result"}),
            summary: "returned one fact".into(),
            observed_effects: vec!["fs.read:.".into()],
            succeeded: true,
            truncated: false,
        })
    }
}
```

Register extensions in a `ToolRegistry`, build a `PolicyEngine` from the exact
task permissions, then construct `RunEngine`. Production hosts should also add
an isolated `WorkspaceTransaction`, `EventStore`, `CheckpointStore`, bounded
context fragments, cancellation, and an approval resolver/observer. The SDK
reexports those types by subsystem rather than introducing a second, weaker
execution path.

The compile-time compatibility fixture at
`crates/pactrail-sdk/tests/embedding_api.rs` implements both extension types and
assembles the real kernel. MCP extensions use the same `ToolRegistry` through
`pactrail_sdk::mcp::register_snapshot`; they do not bypass policy or traces.

## Optional language-server evidence

The context module exposes `LspReferenceSnapshot` and
`RepositoryIndex::apply_lsp_references`. An embedding host may operate its own
language server boundary, normalize bounded reference locations, create a
snapshot for the exact current repository digest, and explicitly merge it
before compiling a `ContextPack`. Pactrail validates snapshot integrity, known
symbols, current paths, and line bounds and retains lexical, language-server,
or corroborated provenance.

This API does not start or communicate with a language server. Process/network
authority, protocol lifecycle, timeouts, cancellation, and executable trust
remain the embedding host's responsibility. Invalid or stale snapshots fail
before graph mutation.

## Read-only Git evidence

The `git` module exposes `GitInspector` and its typed status, diff, history, and
error records. `tool` exposes the three corresponding built-ins. The inspector
opens only a repository with a real `.git` directory rooted exactly at the
supplied workspace. It enables none of Gitoxide's command, network-client,
credential, status/filter-pipeline, or remote-operation features, and its
private implementation exposes and calls none of those operations. Worktree
files, tree traversal, object size, output, and history all have hard bounds.

Status deliberately reports HEAD-to-index, index-to-raw-worktree, and
Pactrail-candidate state separately. The raw comparison does not apply Git
clean filters and never executes hooks, textconv, submodule helpers, filesystem
monitors, or external commands. Results say when a path is unscanned or output
is truncated; embedders must preserve that uncertainty instead of converting
it to a clean result.
