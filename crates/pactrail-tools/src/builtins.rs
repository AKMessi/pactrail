use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use ignore::WalkBuilder;
use pactrail_core::Capability;
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Tool, ToolContext, ToolDescriptor, ToolError, ToolOutput};

const MAX_READ_BYTES: u64 = 1024 * 1024;
const MAX_SEARCH_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_EDIT_BYTES: u64 = 8 * 1024 * 1024;

fn descriptor<T: JsonSchema>(
    name: &str,
    description: &str,
    required_capability: Capability,
) -> ToolDescriptor {
    ToolDescriptor {
        name: name.to_owned(),
        description: description.to_owned(),
        input_schema: serde_json::to_value(schema_for!(T)).unwrap_or_else(|_| json!({})),
        required_capability,
    }
}

fn input<T: for<'de> Deserialize<'de>>(value: Value, tool: &'static str) -> Result<T, ToolError> {
    serde_json::from_value(value).map_err(|source| ToolError::InvalidInput { tool, source })
}

fn success(content: Value, summary: impl Into<String>, effects: Vec<String>) -> ToolOutput {
    ToolOutput {
        content,
        summary: summary.into(),
        observed_effects: effects,
        succeeded: true,
        truncated: false,
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadFileInput {
    path: String,
    start_line: Option<usize>,
    end_line: Option<usize>,
}

/// Reads bounded UTF-8 content from the isolated workspace.
pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor::<ReadFileInput>(
            "read_file",
            "Read a UTF-8 file or inclusive line range from the isolated workspace.",
            Capability::FileRead,
        )
    }

    async fn execute(
        &self,
        context: &ToolContext<'_>,
        value: Value,
    ) -> Result<ToolOutput, ToolError> {
        let request: ReadFileInput = input(value, "read_file")?;
        context.authorize(&Capability::FileRead, request.path.clone(), "read_file")?;
        let path = context.workspace.resolve_read(&request.path)?;
        let bytes = read_bounded(&path, MAX_READ_BYTES)?;
        let text = String::from_utf8(bytes).map_err(|_| ToolError::NonUtf8(path.clone()))?;
        let total_lines = text.lines().count();
        let start = request.start_line.unwrap_or(1);
        let end = request.end_line.unwrap_or(total_lines.max(1));
        if start == 0 || end < start {
            return Err(ToolError::InvalidRange(format!(
                "line range must be 1-based and ordered, got {start}..={end}"
            )));
        }
        let selected = text
            .lines()
            .enumerate()
            .filter(|(index, _)| {
                let line = index + 1;
                line >= start && line <= end
            })
            .map(|(_, line)| line)
            .collect::<Vec<_>>()
            .join("\n");
        let returned_end = end.min(total_lines);
        Ok(success(
            json!({
                "path": request.path,
                "start_line": start,
                "end_line": returned_end,
                "total_lines": total_lines,
                "content": selected,
            }),
            format!(
                "read {} lines from {}",
                returned_end.saturating_sub(start).saturating_add(1),
                request.path
            ),
            vec![format!("fs.read:{}", request.path)],
        ))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListFilesInput {
    path: Option<String>,
    #[serde(default = "default_list_limit")]
    max_entries: usize,
}

const fn default_list_limit() -> usize {
    500
}

/// Lists files in stable lexical order.
pub struct ListFilesTool;

#[async_trait]
impl Tool for ListFilesTool {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor::<ListFilesInput>(
            "list_files",
            "List non-ignored regular files below a workspace-relative directory.",
            Capability::FileRead,
        )
    }

    async fn execute(
        &self,
        context: &ToolContext<'_>,
        value: Value,
    ) -> Result<ToolOutput, ToolError> {
        let request: ListFilesInput = input(value, "list_files")?;
        if request.max_entries == 0 || request.max_entries > 10_000 {
            return Err(ToolError::InvalidRange(
                "max_entries must be between 1 and 10000".to_owned(),
            ));
        }
        let relative = request.path.unwrap_or_else(|| ".".to_owned());
        context.authorize(&Capability::FileRead, relative.clone(), "list_files")?;
        let start = resolve_directory(context, &relative)?;
        let mut files = BTreeSet::new();
        let mut truncated = false;
        for item in WalkBuilder::new(&start)
            .hidden(false)
            .git_ignore(true)
            .sort_by_file_path(Ord::cmp)
            .build()
        {
            let entry = item.map_err(|source| ToolError::Io {
                path: start.clone(),
                source: std::io::Error::other(source),
            })?;
            if entry.file_type().is_some_and(|kind| kind.is_file()) {
                files.insert(portable_relative(
                    context.workspace.workspace_root(),
                    entry.path(),
                )?);
                if files.len() > request.max_entries {
                    files.pop_last();
                    truncated = true;
                }
            }
        }
        let count = files.len();
        Ok(ToolOutput {
            content: json!({ "files": files }),
            summary: format!("listed {count} files below {relative}"),
            observed_effects: vec![format!("fs.list:{relative}")],
            succeeded: true,
            truncated,
        })
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchInput {
    query: String,
    path: Option<String>,
    #[serde(default = "default_search_limit")]
    max_results: usize,
    #[serde(default)]
    case_sensitive: bool,
}

const fn default_search_limit() -> usize {
    100
}

#[derive(Debug, Serialize)]
struct SearchMatch {
    path: String,
    line: usize,
    text: String,
}

/// Performs deterministic bounded text search without invoking a shell.
pub struct SearchTool;

#[async_trait]
impl Tool for SearchTool {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor::<SearchInput>(
            "search",
            "Search UTF-8 workspace files for a literal string and return cited matching lines.",
            Capability::FileRead,
        )
    }

    async fn execute(
        &self,
        context: &ToolContext<'_>,
        value: Value,
    ) -> Result<ToolOutput, ToolError> {
        let request: SearchInput = input(value, "search")?;
        if request.query.is_empty() {
            return Err(ToolError::InvalidRange("query cannot be empty".to_owned()));
        }
        if request.max_results == 0 || request.max_results > 5_000 {
            return Err(ToolError::InvalidRange(
                "max_results must be between 1 and 5000".to_owned(),
            ));
        }
        let relative = request.path.unwrap_or_else(|| ".".to_owned());
        context.authorize(&Capability::FileRead, relative.clone(), "search")?;
        let start = resolve_directory(context, &relative)?;
        let needle = if request.case_sensitive {
            request.query.clone()
        } else {
            request.query.to_lowercase()
        };
        let mut matches = Vec::new();
        let mut truncated = false;
        'files: for item in WalkBuilder::new(&start)
            .hidden(false)
            .git_ignore(true)
            .sort_by_file_path(Ord::cmp)
            .build()
        {
            let entry = item.map_err(|source| ToolError::Io {
                path: start.clone(),
                source: std::io::Error::other(source),
            })?;
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let metadata = entry.metadata().map_err(|source| ToolError::Io {
                path: entry.path().to_path_buf(),
                source: std::io::Error::other(source),
            })?;
            if metadata.len() > MAX_SEARCH_FILE_BYTES {
                continue;
            }
            let file = File::open(entry.path()).map_err(|source| ToolError::Io {
                path: entry.path().to_path_buf(),
                source,
            })?;
            let bounded = file.take(MAX_SEARCH_FILE_BYTES + 1);
            for (index, line) in BufReader::new(bounded).lines().enumerate() {
                let Ok(line) = line else { continue };
                let haystack = if request.case_sensitive {
                    line.clone()
                } else {
                    line.to_lowercase()
                };
                if haystack.contains(&needle) {
                    matches.push(SearchMatch {
                        path: portable_relative(context.workspace.workspace_root(), entry.path())?,
                        line: index + 1,
                        text: line,
                    });
                    if matches.len() == request.max_results {
                        truncated = true;
                        break 'files;
                    }
                }
            }
        }
        Ok(ToolOutput {
            content: serde_json::to_value(&matches).map_err(ToolError::Serialization)?,
            summary: format!("found {} matches for {:?}", matches.len(), request.query),
            observed_effects: vec![format!("fs.search:{relative}")],
            succeeded: true,
            truncated,
        })
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WriteFileInput {
    path: String,
    content: String,
}

/// Writes UTF-8 content to the isolated transaction.
pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor::<WriteFileInput>(
            "write_file",
            "Create or replace one UTF-8 file inside the task's write scope.",
            Capability::FileWrite,
        )
    }

    async fn execute(
        &self,
        context: &ToolContext<'_>,
        value: Value,
    ) -> Result<ToolOutput, ToolError> {
        let request: WriteFileInput = input(value, "write_file")?;
        let bytes = u64::try_from(request.content.len()).unwrap_or(u64::MAX);
        if bytes > MAX_EDIT_BYTES {
            return Err(ToolError::InvalidRange(format!(
                "content is {bytes} bytes; write limit is {MAX_EDIT_BYTES}"
            )));
        }
        context.authorize(&Capability::FileWrite, request.path.clone(), "write_file")?;
        context
            .workspace
            .write_file(&request.path, request.content.as_bytes())?;
        let digest = blake3::hash(request.content.as_bytes())
            .to_hex()
            .to_string();
        Ok(success(
            json!({ "path": request.path, "digest": digest, "bytes": request.content.len() }),
            "wrote workspace file",
            vec![format!("fs.write:{}", request.path)],
        ))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReplaceTextInput {
    path: String,
    old: String,
    new: String,
    #[serde(default = "default_replacement_count")]
    expected_replacements: usize,
}

const fn default_replacement_count() -> usize {
    1
}

/// Applies an exact, count-checked text replacement.
pub struct ReplaceTextTool;

#[async_trait]
impl Tool for ReplaceTextTool {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor::<ReplaceTextInput>(
            "replace_text",
            "Replace exact text only when the expected occurrence count matches.",
            Capability::FileWrite,
        )
    }

    async fn execute(
        &self,
        context: &ToolContext<'_>,
        value: Value,
    ) -> Result<ToolOutput, ToolError> {
        let request: ReplaceTextInput = input(value, "replace_text")?;
        if request.old.is_empty() || request.expected_replacements == 0 {
            return Err(ToolError::InvalidRange(
                "old text and expected_replacements must be non-empty".to_owned(),
            ));
        }
        context.authorize(&Capability::FileRead, request.path.clone(), "replace_text")?;
        context.authorize(&Capability::FileWrite, request.path.clone(), "replace_text")?;
        let path = context.workspace.resolve_read(&request.path)?;
        let bytes = read_bounded(&path, MAX_EDIT_BYTES)?;
        let text = String::from_utf8(bytes).map_err(|_| ToolError::NonUtf8(path.clone()))?;
        let actual = text.matches(&request.old).count();
        if actual != request.expected_replacements {
            return Err(ToolError::ReplacementCount {
                expected: request.expected_replacements,
                actual,
            });
        }
        let removed = request
            .old
            .len()
            .checked_mul(actual)
            .ok_or_else(|| ToolError::InvalidRange("replacement size overflowed".to_owned()))?;
        let added = request
            .new
            .len()
            .checked_mul(actual)
            .ok_or_else(|| ToolError::InvalidRange("replacement size overflowed".to_owned()))?;
        let resulting_bytes = text
            .len()
            .checked_sub(removed)
            .and_then(|size| size.checked_add(added))
            .ok_or_else(|| ToolError::InvalidRange("replacement size overflowed".to_owned()))?;
        if u64::try_from(resulting_bytes).unwrap_or(u64::MAX) > MAX_EDIT_BYTES {
            return Err(ToolError::InvalidRange(format!(
                "replacement would exceed the {MAX_EDIT_BYTES}-byte edit limit"
            )));
        }
        let replacement = text.replace(&request.old, &request.new);
        context
            .workspace
            .write_file(&request.path, replacement.as_bytes())?;
        Ok(success(
            json!({ "path": request.path, "replacements": actual }),
            format!("replaced {actual} exact occurrence(s)"),
            vec![format!("fs.write:{}", request.path)],
        ))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RemoveFileInput {
    path: String,
}

/// Removes a regular file from the isolated transaction.
pub struct RemoveFileTool;

#[async_trait]
impl Tool for RemoveFileTool {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor::<RemoveFileInput>(
            "remove_file",
            "Remove one regular file inside the task's write scope.",
            Capability::FileWrite,
        )
    }

    async fn execute(
        &self,
        context: &ToolContext<'_>,
        value: Value,
    ) -> Result<ToolOutput, ToolError> {
        let request: RemoveFileInput = input(value, "remove_file")?;
        context.authorize(&Capability::FileWrite, request.path.clone(), "remove_file")?;
        context.workspace.remove_file(&request.path)?;
        Ok(success(
            json!({ "path": request.path }),
            "removed workspace file",
            vec![format!("fs.delete:{}", request.path)],
        ))
    }
}

fn resolve_directory(context: &ToolContext<'_>, relative: &str) -> Result<PathBuf, ToolError> {
    let path = if relative == "." {
        context.workspace.workspace_root().to_path_buf()
    } else {
        context.workspace.resolve_read(relative)?
    };
    if !path.is_dir() {
        return Err(ToolError::Io {
            path,
            source: std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                "search root is not a directory",
            ),
        });
    }
    Ok(path)
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, ToolError> {
    let file = File::open(path).map_err(|source| ToolError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let capacity = usize::try_from(limit.min(64 * 1024)).unwrap_or_default();
    let mut bytes = Vec::with_capacity(capacity);
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| ToolError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
        return Err(ToolError::InvalidRange(format!(
            "file exceeds the {limit}-byte read limit"
        )));
    }
    Ok(bytes)
}

fn portable_relative(root: &Path, path: &Path) -> Result<String, ToolError> {
    let relative = path.strip_prefix(root).map_err(|_| ToolError::Io {
        path: path.to_path_buf(),
        source: std::io::Error::other("path escaped workspace root"),
    })?;
    let components: Result<Vec<_>, _> = relative
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| ToolError::Io {
                    path: PathBuf::from(relative),
                    source: std::io::Error::other("path is not Unicode"),
                })
        })
        .collect();
    components.map(|items| items.join("/"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::PolicyEngine;
    use pactrail_workspace::WorkspaceTransaction;

    fn fixture() -> (tempfile::TempDir, tempfile::TempDir, WorkspaceTransaction) {
        let source = tempfile::tempdir().unwrap_or_else(|error| unreachable!("source: {error}"));
        fs::write(
            source.path().join("hello.txt"),
            "hello world\nsecond line\n",
        )
        .unwrap_or_else(|error| unreachable!("fixture: {error}"));
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
    async fn replace_requires_exact_count() {
        let (_source, _control, transaction) = fixture();
        let policy = PolicyEngine::local_default();
        let context = ToolContext {
            workspace: &transaction,
            policy: &policy,
        };
        let output = ReplaceTextTool
            .execute(
                &context,
                json!({"path":"hello.txt","old":"world","new":"Pactrail"}),
            )
            .await;
        assert!(output.is_ok());
        assert_eq!(
            fs::read_to_string(transaction.workspace_root().join("hello.txt")).ok(),
            Some("hello Pactrail\nsecond line\n".to_owned())
        );
    }

    #[tokio::test]
    async fn reads_line_ranges() {
        let (_source, _control, transaction) = fixture();
        let policy = PolicyEngine::local_default();
        let context = ToolContext {
            workspace: &transaction,
            policy: &policy,
        };
        let output = ReadFileTool
            .execute(
                &context,
                json!({"path":"hello.txt","start_line":2,"end_line":2}),
            )
            .await
            .unwrap_or_else(|error| unreachable!("read: {error}"));
        assert_eq!(output.content["content"], "second line");
    }

    #[tokio::test]
    async fn file_listing_is_lexical_and_memory_bounded() {
        let (_source, _control, transaction) = fixture();
        for name in ["z.txt", "a.txt", "b.txt"] {
            transaction
                .write_file(name, b"fixture")
                .unwrap_or_else(|error| unreachable!("candidate file: {error}"));
        }
        let policy = PolicyEngine::local_default();
        let context = ToolContext {
            workspace: &transaction,
            policy: &policy,
        };
        let output = ListFilesTool
            .execute(&context, json!({"max_entries": 2}))
            .await
            .unwrap_or_else(|error| unreachable!("list: {error}"));
        assert_eq!(output.content["files"], json!(["a.txt", "b.txt"]));
        assert!(output.truncated);
    }
}
