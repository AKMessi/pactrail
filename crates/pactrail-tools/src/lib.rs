//! Typed, policy-aware tools for Pactrail.

mod builtins;
mod policy;
mod process;
mod registry;

pub use builtins::{
    ListFilesTool, ReadFileTool, RemoveFileTool, ReplaceTextTool, SearchTool, WriteFileTool,
};
pub use policy::PolicyEngine;
pub use process::RunProcessTool;
pub use registry::{Tool, ToolContext, ToolDescriptor, ToolError, ToolOutput, ToolRegistry};

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
    registry.register(ListFilesTool)?;
    registry.register(SearchTool)?;
    registry.register(WriteFileTool)?;
    registry.register(ReplaceTextTool)?;
    registry.register(RemoveFileTool)?;
    registry.register(RunProcessTool)?;
    Ok(registry)
}
