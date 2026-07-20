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

use crate::registry::replace_checked_preserving_newlines;
use crate::{Tool, ToolAnnotations, ToolContext, ToolDescriptor, ToolError, ToolOutput};

const MAX_READ_BYTES: u64 = 1024 * 1024;
const DEFAULT_READ_LINES: usize = 300;
const MAX_READ_LINES: usize = 1_000;
const MAX_SEARCH_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_EDIT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_MUTATION_FEEDBACK_LINES: usize = 80;
const MAX_MUTATION_FEEDBACK_TEXT_BYTES: usize = 12 * 1024;
const MAX_MUTATION_FEEDBACK_LINE_BYTES: usize = 2 * 1024;
const MUTATION_CONTEXT_LINES: usize = 3;

pub(crate) fn descriptor<T: JsonSchema>(
    name: &str,
    description: &str,
    required_capability: Capability,
) -> ToolDescriptor {
    let annotations = match required_capability {
        Capability::FileRead | Capability::MemoryRead => ToolAnnotations::READ_ONLY,
        Capability::FileWrite => ToolAnnotations::WORKSPACE_MUTATION,
        Capability::ProcessSpawn
        | Capability::Network
        | Capability::SecretUse
        | Capability::ExternalWrite => ToolAnnotations::HOST_EXECUTION,
    };
    ToolDescriptor {
        name: name.to_owned(),
        description: description.to_owned(),
        input_schema: serde_json::to_value(schema_for!(T)).unwrap_or_else(|_| json!({})),
        required_capability,
        annotations,
    }
}

pub(crate) fn input<T: for<'de> Deserialize<'de>>(
    value: Value,
    tool: &'static str,
) -> Result<T, ToolError> {
    serde_json::from_value(value).map_err(|source| ToolError::InvalidInput { tool, source })
}

pub(crate) fn success(
    content: Value,
    summary: impl Into<String>,
    effects: Vec<String>,
) -> ToolOutput {
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
    /// Workspace-relative file path, such as `src/lib.rs`. Absolute paths are forbidden.
    path: String,
    /// Optional one-based first line to return.
    start_line: Option<usize>,
    /// Optional inclusive one-based last line to return.
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
        let end = request
            .end_line
            .unwrap_or_else(|| start.saturating_add(DEFAULT_READ_LINES.saturating_sub(1)));
        if start == 0 || end < start {
            return Err(ToolError::InvalidRange(format!(
                "line range must be 1-based and ordered, got {start}..={end}"
            )));
        }
        let requested_lines = end.saturating_sub(start).saturating_add(1);
        if requested_lines > MAX_READ_LINES {
            return Err(ToolError::InvalidRange(format!(
                "a read may return at most {MAX_READ_LINES} lines; requested {requested_lines}"
            )));
        }
        let selected_lines = text
            .lines()
            .enumerate()
            .filter(|(index, _)| {
                let line = index + 1;
                line >= start && line <= end
            })
            .map(|(_, line)| line)
            .collect::<Vec<_>>();
        let selected = selected_lines.join("\n");
        let returned_end = end.min(total_lines);
        let truncated = returned_end < total_lines;
        let next_start_line = truncated.then(|| returned_end.saturating_add(1));
        Ok(ToolOutput {
            content: json!({
                "path": request.path,
                "start_line": start,
                "end_line": returned_end,
                "total_lines": total_lines,
                "next_start_line": next_start_line,
                "content": selected,
            }),
            summary: format!("read {} lines from {}", selected_lines.len(), request.path),
            observed_effects: vec![format!("fs.read:{}", request.path)],
            succeeded: true,
            truncated,
        })
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListFilesInput {
    /// Workspace-relative directory. Omit or use `.` for the workspace root; never pass a file or absolute path.
    path: Option<String>,
    /// Maximum number of file paths to return.
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
            "List non-ignored regular files below a workspace-relative directory. Call once per directory, then read suggested or relevant files; repeating the same listing returns no new evidence.",
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
        let mut suggested_reads = files
            .iter()
            .filter_map(|path| project_anchor_rank(path).map(|rank| (rank, path)))
            .collect::<Vec<_>>();
        suggested_reads.sort_by(|(left_rank, left), (right_rank, right)| {
            left_rank.cmp(right_rank).then_with(|| left.cmp(right))
        });
        let suggested_reads = suggested_reads
            .into_iter()
            .take(8)
            .map(|(_, path)| path)
            .collect::<Vec<_>>();
        Ok(ToolOutput {
            content: json!({
                "files": files,
                "suggested_reads": suggested_reads,
                "guidance": "Do not repeat this identical listing. Use read_many_files for suggested or task-relevant files, or answer from evidence already collected.",
            }),
            summary: format!("listed {count} files below {relative}"),
            observed_effects: vec![format!("fs.list:{relative}")],
            succeeded: true,
            truncated,
        })
    }
}

fn project_anchor_rank(path: &str) -> Option<u8> {
    let normalized = path.to_ascii_lowercase();
    let file_name = normalized.rsplit('/').next().unwrap_or(&normalized);
    match file_name {
        "readme" | "readme.md" | "readme.mdx" | "readme.rst" | "readme.txt" => Some(0),
        "cargo.toml" | "package.json" | "pyproject.toml" | "go.mod" | "pom.xml"
        | "build.gradle" | "build.gradle.kts" | "mix.exs" | "composer.json" => Some(1),
        _ => match normalized.as_str() {
            "src/main.rs" | "src/lib.rs" | "main.py" | "app.py" | "src/index.ts"
            | "src/index.js" | "cmd/main.go" => Some(2),
            _ => None,
        },
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchInput {
    /// Literal text to find.
    query: String,
    /// Workspace-relative file or directory to search. Omit or use `.` for the root. Absolute paths are forbidden.
    path: Option<String>,
    /// Maximum number of matching lines to return.
    #[serde(default = "default_search_limit")]
    max_results: usize,
    /// Whether matching preserves letter case.
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
            "Search a workspace-relative UTF-8 file or directory for a literal string and return cited matching lines.",
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
        let relative = request.path.clone().unwrap_or_else(|| ".".to_owned());
        context.authorize(&Capability::FileRead, relative.clone(), "search")?;
        let start = if relative == "." {
            context.workspace.workspace_root().to_path_buf()
        } else {
            context.workspace.resolve_read(&relative)?
        };
        if !start.is_dir() && !start.is_file() {
            return Err(ToolError::InvalidRange(format!(
                "path {relative:?} must name a workspace-relative file or directory"
            )));
        }
        let needle = if request.case_sensitive {
            request.query.clone()
        } else {
            request.query.to_lowercase()
        };
        let mut matches = Vec::new();
        let truncated = if start.is_file() {
            search_file(
                context.workspace.workspace_root(),
                &start,
                &request,
                &needle,
                &mut matches,
            )?
        } else {
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
                if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                    continue;
                }
                if search_file(
                    context.workspace.workspace_root(),
                    entry.path(),
                    &request,
                    &needle,
                    &mut matches,
                )? {
                    truncated = true;
                    break;
                }
            }
            truncated
        };
        Ok(ToolOutput {
            content: serde_json::to_value(&matches).map_err(ToolError::Serialization)?,
            summary: format!("found {} matches for {:?}", matches.len(), request.query),
            observed_effects: vec![format!("fs.search:{relative}")],
            succeeded: true,
            truncated,
        })
    }
}

fn search_file(
    root: &Path,
    path: &Path,
    request: &SearchInput,
    needle: &str,
    matches: &mut Vec<SearchMatch>,
) -> Result<bool, ToolError> {
    let metadata = path.metadata().map_err(|source| ToolError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.len() > MAX_SEARCH_FILE_BYTES {
        return Ok(false);
    }
    let file = File::open(path).map_err(|source| ToolError::Io {
        path: path.to_path_buf(),
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
        if haystack.contains(needle) {
            matches.push(SearchMatch {
                path: portable_relative(root, path)?,
                line: index + 1,
                text: line,
            });
            if matches.len() == request.max_results {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WriteFileInput {
    /// Workspace-relative file path, such as `SMOKE_TEST.md`. Absolute paths are forbidden.
    path: String,
    /// Complete UTF-8 content that the file should contain.
    content: String,
}

/// Writes UTF-8 content to the isolated transaction.
pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor::<WriteFileInput>(
            "write_file",
            "Create or replace one UTF-8 file inside the task's write scope and return bounded current-source evidence.",
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
        let post_edit = mutation_feedback(&request.path, None, &request.content);
        Ok(success(
            json!({
                "path": request.path,
                "digest": digest,
                "bytes": request.content.len(),
                "post_edit": post_edit,
            }),
            "wrote workspace file",
            vec![format!("fs.write:{}", request.path)],
        ))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReplaceTextInput {
    /// Workspace-relative file path. Absolute paths are forbidden.
    path: String,
    /// Exact text expected in the current file.
    old: String,
    /// Replacement text.
    new: String,
    /// Required number of occurrences of `old`; defaults to one.
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
            "Replace exact text only when the expected occurrence count matches, then return bounded current-source evidence around the change.",
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
        if request.old == request.new {
            return Err(ToolError::InvalidRange(
                "old and new text must differ; no-op replacements produce no evidence".to_owned(),
            ));
        }
        context.authorize(&Capability::FileRead, request.path.clone(), "replace_text")?;
        context.authorize(&Capability::FileWrite, request.path.clone(), "replace_text")?;
        let path = context.workspace.resolve_read(&request.path)?;
        let bytes = read_bounded(&path, MAX_EDIT_BYTES)?;
        let text = String::from_utf8(bytes).map_err(|_| ToolError::NonUtf8(path.clone()))?;
        let (replacement, actual) = replace_checked_preserving_newlines(
            &text,
            &request.old,
            &request.new,
            request.expected_replacements,
        )
        .map_err(|actual| ToolError::ReplacementCount {
            expected: request.expected_replacements,
            actual,
        })?;
        if u64::try_from(replacement.len()).unwrap_or(u64::MAX) > MAX_EDIT_BYTES {
            return Err(ToolError::InvalidRange(format!(
                "replacement would exceed the {MAX_EDIT_BYTES}-byte edit limit"
            )));
        }
        context
            .workspace
            .write_file(&request.path, replacement.as_bytes())?;
        let digest = blake3::hash(replacement.as_bytes()).to_hex().to_string();
        let post_edit = mutation_feedback(&request.path, Some(&text), &replacement);
        Ok(success(
            json!({
                "path": request.path,
                "replacements": actual,
                "digest": digest,
                "result_bytes": replacement.len(),
                "post_edit": post_edit,
            }),
            format!("replaced {actual} exact occurrence(s)"),
            vec![format!("fs.write:{}", request.path)],
        ))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RemoveFileInput {
    /// Workspace-relative file path. Absolute paths are forbidden.
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
            json!({
                "path": request.path,
                "removed": true,
                "exists_in_candidate": false,
                "guidance": "The path is absent from the isolated candidate. Use workspace_changes to inspect the complete candidate transaction.",
            }),
            "removed workspace file",
            vec![format!("fs.delete:{}", request.path)],
        ))
    }
}

pub(crate) fn mutation_feedback(path: &str, before: Option<&str>, after: &str) -> Value {
    let lines = after.lines().collect::<Vec<_>>();
    let total_lines = lines.len();
    let (changed_start, changed_end) = before.map_or_else(
        || (1, total_lines.max(1)),
        |before| changed_line_span(before, after),
    );
    let ranges = mutation_preview_ranges(changed_start, changed_end, total_lines);
    let mut remaining_bytes = MAX_MUTATION_FEEDBACK_TEXT_BYTES;
    let mut text_truncated = false;
    let mut previews = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        let mut preview_lines = Vec::with_capacity(end.saturating_sub(start).saturating_add(1));
        for line_number in start..=end {
            let Some(line) = lines.get(line_number.saturating_sub(1)) else {
                continue;
            };
            if remaining_bytes == 0 {
                text_truncated = true;
                break;
            }
            let line_limit = remaining_bytes.min(MAX_MUTATION_FEEDBACK_LINE_BYTES);
            let (text, line_truncated) = truncate_source_line(line, line_limit);
            remaining_bytes = remaining_bytes.saturating_sub(text.len());
            text_truncated |= line_truncated;
            preview_lines.push(json!({
                "line": line_number,
                "text": text,
                "text_truncated": line_truncated,
            }));
        }
        if !preview_lines.is_empty() {
            previews.push(json!({
                "start_line": start,
                "end_line": end,
                "lines": preview_lines,
            }));
        }
    }
    let changed_lines_fully_shown = !text_truncated
        && changed_start <= changed_end
        && previews_cover_range(&previews, changed_start, changed_end);
    json!({
        "path": path,
        "digest": blake3::hash(after.as_bytes()).to_hex().to_string(),
        "bytes": after.len(),
        "total_lines": total_lines,
        "changed_line_start": changed_start,
        "changed_line_end": changed_end,
        "changed_lines_fully_shown": changed_lines_fully_shown,
        "previews": previews,
        "guidance": if changed_lines_fully_shown {
            "This is current source from the isolated candidate after the mutation."
        } else {
            "Post-edit evidence is bounded. Use read_file with path and a narrow start_line/end_line range before relying on omitted changed source."
        },
    })
}

fn changed_line_span(before: &str, after: &str) -> (usize, usize) {
    if before == after {
        let line = after.lines().count().max(1);
        return (line, line);
    }
    let mut prefix = before
        .as_bytes()
        .iter()
        .zip(after.as_bytes())
        .take_while(|(left, right)| left == right)
        .count();
    while prefix > 0 && (!before.is_char_boundary(prefix) || !after.is_char_boundary(prefix)) {
        prefix -= 1;
    }
    let suffix_limit = before.len().min(after.len()).saturating_sub(prefix);
    let mut suffix = before
        .as_bytes()
        .iter()
        .rev()
        .zip(after.as_bytes().iter().rev())
        .take(suffix_limit)
        .take_while(|(left, right)| left == right)
        .count();
    while suffix > 0
        && (!before.is_char_boundary(before.len().saturating_sub(suffix))
            || !after.is_char_boundary(after.len().saturating_sub(suffix)))
    {
        suffix -= 1;
    }
    let changed_end_offset = after.len().saturating_sub(suffix);
    let start_line = after[..prefix]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1;
    let end_line = if changed_end_offset > prefix {
        after[..changed_end_offset.saturating_sub(1)]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1
    } else {
        start_line
    };
    (start_line, end_line.max(start_line))
}

fn mutation_preview_ranges(start: usize, end: usize, total_lines: usize) -> Vec<(usize, usize)> {
    if total_lines == 0 {
        return Vec::new();
    }
    let start = start.clamp(1, total_lines);
    let end = end.clamp(start, total_lines);
    let expanded_start = start.saturating_sub(MUTATION_CONTEXT_LINES).max(1);
    let expanded_end = end.saturating_add(MUTATION_CONTEXT_LINES).min(total_lines);
    if expanded_end
        .saturating_sub(expanded_start)
        .saturating_add(1)
        <= MAX_MUTATION_FEEDBACK_LINES
    {
        return vec![(expanded_start, expanded_end)];
    }

    let first_lines = MAX_MUTATION_FEEDBACK_LINES / 2;
    let first_end = expanded_start
        .saturating_add(first_lines.saturating_sub(1))
        .min(total_lines);
    let last_lines = MAX_MUTATION_FEEDBACK_LINES.saturating_sub(first_lines);
    let last_start = expanded_end
        .saturating_sub(last_lines.saturating_sub(1))
        .max(1);
    if last_start <= first_end.saturating_add(1) {
        vec![(expanded_start, expanded_end)]
    } else {
        vec![(expanded_start, first_end), (last_start, expanded_end)]
    }
}

fn truncate_source_line(line: &str, max_bytes: usize) -> (String, bool) {
    if line.len() <= max_bytes {
        return (line.to_owned(), false);
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !line.is_char_boundary(boundary) {
        boundary -= 1;
    }
    (line[..boundary].to_owned(), true)
}

fn previews_cover_range(previews: &[Value], start: usize, end: usize) -> bool {
    let mut cursor = start;
    for preview in previews {
        let Some(preview_start) = preview.get("start_line").and_then(Value::as_u64) else {
            continue;
        };
        let Some(preview_end) = preview.get("end_line").and_then(Value::as_u64) else {
            continue;
        };
        let preview_start = usize::try_from(preview_start).unwrap_or(usize::MAX);
        let preview_end = usize::try_from(preview_end).unwrap_or_default();
        if preview_start <= cursor && preview_end >= cursor {
            cursor = preview_end.saturating_add(1);
            if cursor > end {
                return true;
            }
        }
    }
    false
}

fn resolve_directory(context: &ToolContext<'_>, relative: &str) -> Result<PathBuf, ToolError> {
    let path = if relative == "." {
        context.workspace.workspace_root().to_path_buf()
    } else {
        context.workspace.resolve_read(relative)?
    };
    if !path.is_dir() {
        return Err(ToolError::InvalidRange(format!(
            "path {relative:?} must name a workspace-relative directory; omit it or use `.` for the workspace root"
        )));
    }
    Ok(path)
}

pub(crate) fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, ToolError> {
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
        let context = ToolContext::new(&transaction, &policy, None);
        let output = ReplaceTextTool
            .execute(
                &context,
                json!({"path":"hello.txt","old":"world","new":"Pactrail"}),
            )
            .await
            .unwrap_or_else(|error| unreachable!("replace: {error}"));
        assert_eq!(
            fs::read_to_string(transaction.workspace_root().join("hello.txt")).ok(),
            Some("hello Pactrail\nsecond line\n".to_owned())
        );
        assert_eq!(output.content["post_edit"]["changed_line_start"], 1);
        assert_eq!(output.content["post_edit"]["changed_line_end"], 1);
        assert_eq!(
            output.content["post_edit"]["previews"][0]["lines"][0]["text"],
            "hello Pactrail"
        );
        assert_eq!(output.content["digest"].as_str().map(str::len), Some(64));

        let no_op = ReplaceTextTool
            .execute(
                &context,
                json!({"path":"hello.txt","old":"Pactrail","new":"Pactrail"}),
            )
            .await;
        assert!(
            matches!(no_op, Err(ToolError::InvalidRange(message)) if message.contains("no-op"))
        );
    }

    #[tokio::test]
    async fn reads_line_ranges() {
        let (_source, _control, transaction) = fixture();
        let policy = PolicyEngine::local_default();
        let context = ToolContext::new(&transaction, &policy, None);
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
    async fn default_reads_are_paginated_and_report_the_next_line() {
        let (_source, _control, transaction) = fixture();
        let lines_text = (1..=350)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        transaction
            .write_file("large.txt", lines_text.as_bytes())
            .unwrap_or_else(|error| unreachable!("large fixture: {error}"));
        let policy = PolicyEngine::local_default();
        let context = ToolContext::new(&transaction, &policy, None);

        let output = ReadFileTool
            .execute(&context, json!({"path":"large.txt"}))
            .await
            .unwrap_or_else(|error| unreachable!("read: {error}"));

        assert!(output.truncated);
        assert_eq!(output.content["start_line"], 1);
        assert_eq!(output.content["end_line"], DEFAULT_READ_LINES);
        assert_eq!(output.content["next_start_line"], DEFAULT_READ_LINES + 1);
        assert_eq!(
            output.content["content"]
                .as_str()
                .map(|text| text.lines().count()),
            Some(DEFAULT_READ_LINES)
        );
    }

    #[tokio::test]
    async fn search_accepts_a_specific_workspace_file() {
        let (_source, _control, transaction) = fixture();
        let policy = PolicyEngine::local_default();
        let context = ToolContext::new(&transaction, &policy, None);

        let output = SearchTool
            .execute(
                &context,
                json!({"path":"hello.txt","query":"second","max_results":10}),
            )
            .await
            .unwrap_or_else(|error| unreachable!("search: {error}"));

        assert_eq!(output.content[0]["path"], "hello.txt");
        assert_eq!(output.content[0]["line"], 2);
        assert_eq!(output.content[0]["text"], "second line");
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
        let context = ToolContext::new(&transaction, &policy, None);
        let output = ListFilesTool
            .execute(&context, json!({"max_entries": 2}))
            .await
            .unwrap_or_else(|error| unreachable!("list: {error}"));
        assert_eq!(output.content["files"], json!(["a.txt", "b.txt"]));
        assert!(output.truncated);
    }

    #[tokio::test]
    async fn file_listing_steers_models_toward_project_anchors() {
        let (_source, _control, transaction) = fixture();
        for name in ["README.md", "Cargo.toml", "src/lib.rs", "notes.txt"] {
            transaction
                .write_file(name, b"fixture")
                .unwrap_or_else(|error| unreachable!("candidate file: {error}"));
        }
        let policy = PolicyEngine::local_default();
        let context = ToolContext::new(&transaction, &policy, None);

        let output = ListFilesTool
            .execute(&context, json!({}))
            .await
            .unwrap_or_else(|error| unreachable!("list: {error}"));

        assert_eq!(
            output.content["suggested_reads"],
            json!(["README.md", "Cargo.toml", "src/lib.rs"])
        );
        assert!(
            output.content["guidance"]
                .as_str()
                .is_some_and(|guidance| guidance.contains("Do not repeat"))
        );
    }

    #[test]
    fn list_schema_explains_its_virtual_directory_path() {
        let schema = ListFilesTool.descriptor().input_schema;
        let description = schema["properties"]["path"]["description"]
            .as_str()
            .unwrap_or_default();

        assert!(description.contains("Workspace-relative directory"));
        assert!(description.contains("never pass a file or absolute path"));
    }

    #[test]
    fn mutation_feedback_bounds_distant_edits_and_cites_both_edges() {
        let before = (1..=120)
            .map(|line| format!("before {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut after_lines = before.lines().map(str::to_owned).collect::<Vec<_>>();
        after_lines[4] = "changed first".to_owned();
        after_lines[114] = "changed last".to_owned();
        let after = after_lines.join("\n");

        let feedback = mutation_feedback("src/lib.rs", Some(&before), &after);

        assert_eq!(feedback["changed_line_start"], 5);
        assert_eq!(feedback["changed_line_end"], 115);
        assert_eq!(feedback["previews"].as_array().map(Vec::len), Some(2));
        assert_eq!(feedback["changed_lines_fully_shown"], false);
        let rendered = feedback["previews"].to_string();
        assert!(rendered.contains("changed first"));
        assert!(rendered.contains("changed last"));
    }

    #[test]
    fn mutation_feedback_is_utf8_safe_and_byte_bounded() {
        let after = format!("prefix {} suffix", "🦀".repeat(10_000));
        let feedback = mutation_feedback("unicode.txt", None, &after);
        let preview = feedback["previews"][0]["lines"][0]["text"]
            .as_str()
            .unwrap_or_default();

        assert!(preview.len() <= MAX_MUTATION_FEEDBACK_LINE_BYTES);
        assert!(preview.is_char_boundary(preview.len()));
        assert_eq!(feedback["previews"][0]["lines"][0]["text_truncated"], true);
        assert_eq!(feedback["changed_lines_fully_shown"], false);
    }
}
