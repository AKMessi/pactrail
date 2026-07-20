use async_trait::async_trait;
use pactrail_context::RepositoryIndex;
use pactrail_core::Capability;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::builtins::{descriptor, input};
use crate::{Tool, ToolContext, ToolDescriptor, ToolError, ToolOutput};

const DEFAULT_MAX_SYMBOLS: usize = 12;
const MAX_SYMBOLS: usize = 32;
const DEFAULT_MAX_REFERENCES: usize = 20;
const MAX_REFERENCES: usize = 128;
const MAX_QUERY_BYTES: usize = 512;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchCodeGraphInput {
    /// Function, class, type, trait, or other project-defined symbol to locate.
    query: String,
    /// Maximum matching project-defined symbols to return.
    #[serde(default = "default_max_symbols")]
    max_symbols: usize,
    /// Maximum lexical reference locations retained for each matched symbol.
    #[serde(default = "default_max_references")]
    max_references_per_symbol: usize,
}

const fn default_max_symbols() -> usize {
    DEFAULT_MAX_SYMBOLS
}

const fn default_max_references() -> usize {
    DEFAULT_MAX_REFERENCES
}

/// Searches a deterministic, current repository-wide symbol evidence graph.
pub struct SearchCodeGraphTool;

#[async_trait]
impl Tool for SearchCodeGraphTool {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor::<SearchCodeGraphInput>(
            "search_code_graph",
            "Find project-defined symbols and bounded lexical references across the current isolated workspace. Use it to navigate relationships, then read cited source before editing; references are not runtime call-graph proof.",
            Capability::FileRead,
        )
    }

    async fn execute(
        &self,
        context: &ToolContext<'_>,
        value: Value,
    ) -> Result<ToolOutput, ToolError> {
        let request: SearchCodeGraphInput = input(value, "search_code_graph")?;
        validate_request(&request)?;
        context.authorize(&Capability::FileRead, ".", "search_code_graph")?;

        // Build from the candidate on every invocation. This deliberately
        // favors correctness over a stale cache: an edit made in the preceding
        // tool turn is immediately visible to structural navigation.
        let index = RepositoryIndex::build(context.workspace.workspace_root())?;
        let result = index.graph.query(
            request.query.trim(),
            request.max_symbols,
            request.max_references_per_symbol,
        );
        let definition_count = result
            .symbols
            .iter()
            .map(|symbol| symbol.definitions.len())
            .sum::<usize>();
        let reference_count = result
            .symbols
            .iter()
            .map(|symbol| symbol.references.len())
            .sum::<usize>();
        let truncated = result.result_truncated
            || result.graph_truncated
            || result
                .symbols
                .iter()
                .any(|symbol| symbol.references_truncated);

        Ok(ToolOutput {
            content: json!({
                "method": "deterministic project definitions plus bounded lexical identifier references",
                "warning": "navigation evidence only; read cited source before editing and do not infer runtime call flow from lexical references",
                "repository_digest": index.digest,
                "result": result,
            }),
            summary: format!(
                "found {definition_count} definition(s) and {reference_count} bounded reference location(s)"
            ),
            observed_effects: vec!["repository.graph.search".to_owned()],
            succeeded: true,
            truncated,
        })
    }
}

fn validate_request(request: &SearchCodeGraphInput) -> Result<(), ToolError> {
    let query = request.query.trim();
    if query.is_empty() || query.len() > MAX_QUERY_BYTES || query.chars().any(char::is_control) {
        return Err(ToolError::InvalidRange(format!(
            "query must contain 1 to {MAX_QUERY_BYTES} bytes and no control characters"
        )));
    }
    if !(1..=MAX_SYMBOLS).contains(&request.max_symbols) {
        return Err(ToolError::InvalidRange(format!(
            "max_symbols must be between 1 and {MAX_SYMBOLS}"
        )));
    }
    if !(1..=MAX_REFERENCES).contains(&request.max_references_per_symbol) {
        return Err(ToolError::InvalidRange(format!(
            "max_references_per_symbol must be between 1 and {MAX_REFERENCES}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::PolicyEngine;
    use pactrail_workspace::WorkspaceTransaction;

    fn fixture() -> (tempfile::TempDir, tempfile::TempDir, WorkspaceTransaction) {
        let source = tempfile::tempdir().unwrap_or_else(|error| unreachable!("source: {error}"));
        fs::create_dir(source.path().join("src"))
            .unwrap_or_else(|error| unreachable!("src: {error}"));
        fs::write(
            source.path().join("src/receipt.rs"),
            "pub struct Receipt;\n",
        )
        .unwrap_or_else(|error| unreachable!("definition: {error}"));
        fs::write(
            source.path().join("src/use_receipt.rs"),
            "pub fn consume(value: Receipt) {}\n",
        )
        .unwrap_or_else(|error| unreachable!("reference: {error}"));
        let control = tempfile::tempdir().unwrap_or_else(|error| unreachable!("control: {error}"));
        let transaction = WorkspaceTransaction::create(
            source.path(),
            control.path().join("run"),
            &[".".to_owned()],
        )
        .unwrap_or_else(|error| unreachable!("transaction: {error}"));
        (source, control, transaction)
    }

    #[tokio::test]
    async fn searches_the_current_candidate_graph() {
        let (_source, _control, transaction) = fixture();
        let policy = PolicyEngine::local_default();
        let context = ToolContext::new(&transaction, &policy, None);

        let initial = SearchCodeGraphTool
            .execute(&context, json!({"query":"Receipt"}))
            .await
            .unwrap_or_else(|error| unreachable!("graph search: {error}"));
        assert_eq!(initial.content["result"]["symbols"][0]["name"], "Receipt");
        assert_eq!(
            initial.content["result"]["symbols"][0]["references"][0]["path"],
            "src/use_receipt.rs"
        );

        transaction
            .write_file("src/new_use.rs", b"pub fn inspect(value: Receipt) {}\n")
            .unwrap_or_else(|error| unreachable!("candidate edit: {error}"));
        let refreshed = SearchCodeGraphTool
            .execute(&context, json!({"query":"Receipt"}))
            .await
            .unwrap_or_else(|error| unreachable!("refreshed graph search: {error}"));
        assert!(
            refreshed.content["result"]["symbols"][0]["references"]
                .as_array()
                .is_some_and(|references| references
                    .iter()
                    .any(|reference| { reference["path"] == "src/new_use.rs" }))
        );
    }
}
