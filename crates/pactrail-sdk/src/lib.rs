//! Rust embedding facade for Pactrail.
//!
//! The SDK exposes Pactrail's provider-neutral model contract, typed tool
//! contract, governed MCP adapter, execution engine, and durable state types
//! without making downstream applications depend on the CLI crate. The facade
//! is intentionally static: applications link extensions they trust at build
//! time. It does not load repository-provided native libraries or grant dynamic
//! plugins ambient authority.
//!
//! See `docs/embedding.md` in the repository for a complete composition guide.

/// SDK surface revision used by Pactrail 0.7.
///
/// This revision tracks source-level extension compatibility independently of
/// durable task, event, receipt, checkpoint, and MCP schema versions.
pub const SDK_API_REVISION: u32 = 1;

/// Provider-neutral model extension contracts and built-in adapters.
pub mod model {
    pub use pactrail_models::{
        AnthropicConfig, AnthropicDriver, CapabilityProbeReport, CapabilitySource,
        ConversationItem, FinishReason, GeminiConfig, GeminiDriver, Message, ModelCapabilities,
        ModelDriver, ModelError, ModelRequest, ModelResponse, ModelStreamEvent,
        ModelStreamObserver, OpenAiCompatibleConfig, OpenAiCompatibleDriver, ProbeObservation,
        Role, ToolCall, ToolResult, Usage, probe_capabilities,
    };
}

/// Typed tool extension contracts, policy, approvals, and built-in tools.
pub mod tool {
    pub use pactrail_tools::{
        ApprovalResolver, DisabledProcessBackend, EditFileTool, ListFilesTool,
        NativeProcessBackend, OciProcessBackend, OciProcessConfig, OciRuntimeKind,
        OciSandboxProfile, PolicyAuditEntry, PolicyAuditLog, PolicyEngine, ProcessBackend,
        ProcessBackendDescriptor, ProcessBackendError, ProcessBackendKind, ProcessExecution,
        ProcessRequest, ReadFileTool, ReadManyFilesTool, RecallMemoryTool, RemoveFileTool,
        ReplaceTextTool, RunProcessTool, SearchChangeImpactTool, SearchCodeGraphTool, SearchTool,
        Tool, ToolAnnotations, ToolContext, ToolDescriptor, ToolError, ToolOutput, ToolRegistry,
        ToolRisk, WorkspaceChangesTool, WriteFileTool, builtin_registry,
        builtin_registry_with_process,
    };
}

/// Contracts, lifecycle, approvals, events, evidence, and receipts.
pub mod core {
    pub use pactrail_core::*;
}

/// Execution, checkpoints, progress observation, and verification discovery.
pub mod engine {
    pub use pactrail_engine::{
        CheckpointError, CheckpointIdentity, CheckpointStore, EngineError, ResumePhase,
        RunCheckpoint, RunEngine, RunObserver, RunOutcome, RunProgress, VerificationCommand,
        contract_digest, detect_verification_commands,
    };
}

/// Governed MCP manifests, snapshots, discovery, and tool registration.
pub mod mcp {
    pub use pactrail_mcp::*;
}

/// Repository context and provenance-labelled supplemental fragments.
pub mod context {
    pub use pactrail_context::*;
}

/// Provenance-aware durable memory.
pub mod memory {
    pub use pactrail_memory::*;
}

/// Hash-linked event and content-addressed artifact persistence.
pub mod store {
    pub use pactrail_store::*;
}

/// Isolated workspace transactions and apply boundary.
pub mod workspace {
    pub use pactrail_workspace::*;
}

/// Common imports for custom providers, tools, and embedded kernels.
pub mod prelude {
    pub use async_trait::async_trait;
    pub use pactrail_core::{
        ApprovalDecision, ApprovalRequest, Capability, PermissionSet, RunId, TaskContract,
    };
    pub use pactrail_engine::{RunEngine, RunObserver, RunOutcome, RunProgress};
    pub use pactrail_models::{
        ModelCapabilities, ModelDriver, ModelError, ModelRequest, ModelResponse,
        ModelStreamObserver,
    };
    pub use pactrail_store::EventStore;
    pub use pactrail_tools::{
        ApprovalResolver, PolicyEngine, Tool, ToolAnnotations, ToolContext, ToolDescriptor,
        ToolError, ToolOutput, ToolRegistry, ToolRisk,
    };
    pub use pactrail_workspace::WorkspaceTransaction;
    pub use tokio_util::sync::CancellationToken;
}
