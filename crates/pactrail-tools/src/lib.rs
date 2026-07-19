//! Typed, policy-aware tools for Pactrail.

mod advanced;
mod builtins;
mod policy;
mod process;
mod registry;
mod structural;

pub use builtins::{
    ListFilesTool, ReadFileTool, RemoveFileTool, ReplaceTextTool, SearchTool, WriteFileTool,
};
pub use policy::PolicyEngine;
pub use process::RunProcessTool;
pub use registry::{
    Tool, ToolAnnotations, ToolContext, ToolDescriptor, ToolError, ToolOutput, ToolRegistry,
    ToolRisk,
};
pub use structural::SearchCodeGraphTool;

/// Builds the production default set of local coding tools.
///
/// Process execution is registered but still requires an explicit policy grant.
///
/// # Errors
///
/// Returns an error if two built-in tools accidentally use the same name.
pub fn builtin_registry() -> Result<ToolRegistry, ToolError> {
    let mut registry = ToolRegistry::new();
    registry.register(ReadFileTool)?;
    registry.register(ReadManyFilesTool)?;
    registry.register(ListFilesTool)?;
    registry.register(SearchTool)?;
    registry.register(SearchCodeGraphTool)?;
    registry.register(WriteFileTool)?;
    registry.register(ReplaceTextTool)?;
    registry.register(EditFileTool)?;
    registry.register(RemoveFileTool)?;
    registry.register(WorkspaceChangesTool)?;
    registry.register(RecallMemoryTool)?;
    registry.register(RunProcessTool)?;
    Ok(registry)
}
pub use advanced::{EditFileTool, ReadManyFilesTool, RecallMemoryTool, WorkspaceChangesTool};
