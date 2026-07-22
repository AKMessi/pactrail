use async_trait::async_trait;
use pactrail_core::Capability;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::builtins::{descriptor, input, mutation_feedback, read_bounded, success};
use crate::{Tool, ToolContext, ToolDescriptor, ToolError, ToolOutput};

const MAX_PATCH_BYTES: usize = 512 * 1024;
const MAX_PATCH_HUNKS: usize = 128;
const MAX_PATCH_LINES: usize = 20_000;
const MAX_PATCH_FILE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ApplyPatchInput {
    /// Strict single-file unified diff. Use workspace-relative `a/` and `b/` paths or `/dev/null`.
    patch: String,
    /// Optional BLAKE3 digest of the current file before applying the patch.
    expected_digest: Option<String>,
}

/// Applies one strict unified diff without fuzzy matching or external commands.
pub struct ApplyPatchTool;

#[async_trait]
impl Tool for ApplyPatchTool {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor::<ApplyPatchInput>(
            "apply_patch",
            "Apply one bounded, strict unified diff to one workspace-relative UTF-8 file. Supports add, update, and delete; every hunk and line number must match exactly, no fuzzy offsets are used, and validation completes before the isolated candidate is changed.",
            Capability::FileWrite,
        )
    }

    async fn execute(
        &self,
        context: &ToolContext<'_>,
        value: Value,
    ) -> Result<ToolOutput, ToolError> {
        let request: ApplyPatchInput = input(value, "apply_patch")?;
        validate_patch_envelope(&request)?;
        let patch = UnifiedPatch::parse(&request.patch)?;
        context.authorize(&Capability::FileWrite, patch.path.clone(), "apply_patch")?;
        if patch.operation != PatchOperation::Add {
            context.authorize(&Capability::FileRead, patch.path.clone(), "apply_patch")?;
        }
        let destination = context.workspace.resolve_write(&patch.path)?;
        let original = match patch.operation {
            PatchOperation::Add => {
                if destination.exists() {
                    return Err(ToolError::InvalidPatch(format!(
                        "cannot add {:?}: the candidate path already exists",
                        patch.path
                    )));
                }
                String::new()
            }
            PatchOperation::Update | PatchOperation::Delete => {
                let readable = context.workspace.resolve_read(&patch.path)?;
                let bytes = read_bounded(&readable, MAX_PATCH_FILE_BYTES)?;
                String::from_utf8(bytes).map_err(|_| ToolError::NonUtf8(readable))?
            }
        };
        validate_expected_digest(
            request.expected_digest.as_deref(),
            &original,
            patch.operation,
        )?;
        let applied = patch.apply(&original)?;
        if applied.text == original {
            return Err(ToolError::InvalidPatch(
                "patch produced no file-content change".to_owned(),
            ));
        }
        match patch.operation {
            PatchOperation::Delete => context.workspace.remove_file(&patch.path)?,
            PatchOperation::Add | PatchOperation::Update => context
                .workspace
                .write_file(&patch.path, applied.text.as_bytes())?,
        }

        let operation = patch.operation.label();
        let digest = (patch.operation != PatchOperation::Delete)
            .then(|| blake3::hash(applied.text.as_bytes()).to_hex().to_string());
        let post_edit = (patch.operation != PatchOperation::Delete)
            .then(|| mutation_feedback(&patch.path, Some(&original), &applied.text));
        let effect = if patch.operation == PatchOperation::Delete {
            format!("fs.delete:{}", patch.path)
        } else {
            format!("fs.write:{}", patch.path)
        };
        Ok(success(
            json!({
                "path": patch.path,
                "operation": operation,
                "hunks": patch.hunks.len(),
                "lines_added": applied.lines_added,
                "lines_removed": applied.lines_removed,
                "result_bytes": applied.text.len(),
                "digest": digest,
                "post_edit": post_edit,
                "exists_in_candidate": patch.operation != PatchOperation::Delete,
            }),
            format!(
                "{operation} patch applied to {} ({} hunks, +{}/-{})",
                patch.path,
                patch.hunks.len(),
                applied.lines_added,
                applied.lines_removed
            ),
            vec![effect],
        ))
    }
}

fn validate_patch_envelope(request: &ApplyPatchInput) -> Result<(), ToolError> {
    if request.patch.is_empty() {
        return Err(ToolError::InvalidPatch("patch cannot be empty".to_owned()));
    }
    if request.patch.len() > MAX_PATCH_BYTES {
        return Err(ToolError::InvalidPatch(format!(
            "patch is {} bytes; limit is {MAX_PATCH_BYTES}",
            request.patch.len()
        )));
    }
    if request.patch.contains('\0') {
        return Err(ToolError::InvalidPatch(
            "patch cannot contain NUL bytes".to_owned(),
        ));
    }
    if let Some(digest) = &request.expected_digest
        && !valid_digest(digest)
    {
        return Err(ToolError::InvalidPatch(
            "expected_digest must be exactly 64 hexadecimal characters".to_owned(),
        ));
    }
    Ok(())
}

fn validate_expected_digest(
    expected: Option<&str>,
    original: &str,
    operation: PatchOperation,
) -> Result<(), ToolError> {
    let Some(expected) = expected else {
        return Ok(());
    };
    if operation == PatchOperation::Add {
        return Err(ToolError::InvalidPatch(
            "expected_digest is invalid for an added file because no current file exists"
                .to_owned(),
        ));
    }
    let actual = blake3::hash(original.as_bytes()).to_hex().to_string();
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(ToolError::InvalidPatch(format!(
            "current file digest is {actual}, not expected {}",
            expected.to_ascii_lowercase()
        )))
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PatchOperation {
    Add,
    Update,
    Delete,
}

impl PatchOperation {
    const fn label(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }
}

#[derive(Debug)]
struct UnifiedPatch {
    path: String,
    operation: PatchOperation,
    hunks: Vec<PatchHunk>,
}

impl UnifiedPatch {
    fn parse(input: &str) -> Result<Self, ToolError> {
        let lines = input
            .split('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line))
            .collect::<Vec<_>>();
        if lines.len() > MAX_PATCH_LINES {
            return Err(ToolError::InvalidPatch(format!(
                "patch has {} lines; limit is {MAX_PATCH_LINES}",
                lines.len()
            )));
        }
        if lines.len() < 3 {
            return Err(ToolError::InvalidPatch(
                "expected `---`, `+++`, and at least one `@@` hunk".to_owned(),
            ));
        }
        let old_path = parse_header_path(lines[0], "--- ")?;
        let new_path = parse_header_path(lines[1], "+++ ")?;
        let (path, operation) = patch_identity(old_path, new_path)?;
        let mut cursor = 2_usize;
        let mut hunks = Vec::new();
        while cursor < lines.len() {
            if lines[cursor].is_empty() && cursor + 1 == lines.len() {
                break;
            }
            if hunks.len() == MAX_PATCH_HUNKS {
                return Err(ToolError::InvalidPatch(format!(
                    "patch exceeds the {MAX_PATCH_HUNKS}-hunk limit"
                )));
            }
            let header = lines[cursor];
            if !header.starts_with("@@ ") {
                return Err(ToolError::InvalidPatch(format!(
                    "line {} must begin a hunk with `@@`",
                    cursor + 1
                )));
            }
            cursor = cursor.saturating_add(1);
            let (hunk, next) = PatchHunk::parse(header, &lines, cursor, hunks.len() + 1)?;
            hunks.push(hunk);
            cursor = next;
        }
        if hunks.is_empty() {
            return Err(ToolError::InvalidPatch(
                "patch must contain at least one hunk".to_owned(),
            ));
        }
        validate_operation_shape(operation, &hunks)?;
        Ok(Self {
            path,
            operation,
            hunks,
        })
    }

    fn apply(&self, original: &str) -> Result<AppliedPatch, ToolError> {
        let document = Document::parse(original)?;
        let mut output = Vec::new();
        let mut original_cursor = 0_usize;
        let mut lines_added = 0_usize;
        let mut lines_removed = 0_usize;
        let mut explicit_no_newline = false;
        for (hunk_index, hunk) in self.hunks.iter().enumerate() {
            let old_index = range_index(hunk.old_start, hunk.old_count, "old", hunk_index + 1)?;
            if old_index < original_cursor || old_index > document.lines.len() {
                return Err(ToolError::InvalidPatch(format!(
                    "hunk {} old start {} overlaps a prior hunk or exceeds the {}-line file",
                    hunk_index + 1,
                    hunk.old_start,
                    document.lines.len()
                )));
            }
            output.extend(document.lines[original_cursor..old_index].iter().cloned());
            original_cursor = old_index;
            let new_index = range_index(hunk.new_start, hunk.new_count, "new", hunk_index + 1)?;
            if output.len() != new_index {
                return Err(ToolError::InvalidPatch(format!(
                    "hunk {} new start {} disagrees with the {} output lines preceding it",
                    hunk_index + 1,
                    hunk.new_start,
                    output.len()
                )));
            }
            for line in &hunk.lines {
                match line {
                    HunkLine::Context(expected) => {
                        require_source_line(
                            &document.lines,
                            original_cursor,
                            expected,
                            hunk_index + 1,
                        )?;
                        output.push(expected.clone());
                        original_cursor = original_cursor.saturating_add(1);
                    }
                    HunkLine::Remove(expected) => {
                        require_source_line(
                            &document.lines,
                            original_cursor,
                            expected,
                            hunk_index + 1,
                        )?;
                        original_cursor = original_cursor.saturating_add(1);
                        lines_removed = lines_removed.saturating_add(1);
                    }
                    HunkLine::Add(line) => {
                        output.push(line.clone());
                        lines_added = lines_added.saturating_add(1);
                    }
                }
            }
            explicit_no_newline |= hunk.new_no_newline;
        }
        let touches_end = original_cursor == document.lines.len();
        output.extend(document.lines[original_cursor..].iter().cloned());
        if explicit_no_newline && !touches_end {
            return Err(ToolError::InvalidPatch(
                "a `No newline at end of file` marker was used before the final source region"
                    .to_owned(),
            ));
        }
        if self.operation == PatchOperation::Delete && !output.is_empty() {
            return Err(ToolError::InvalidPatch(format!(
                "delete patch left {} output lines; it must remove the complete file",
                output.len()
            )));
        }
        let ends_with_newline = if output.is_empty() || explicit_no_newline {
            false
        } else if self.operation == PatchOperation::Add || touches_end {
            true
        } else {
            document.ends_with_newline
        };
        let text = Document::render(&output, document.line_ending, ends_with_newline);
        if u64::try_from(text.len()).unwrap_or(u64::MAX) > MAX_PATCH_FILE_BYTES {
            return Err(ToolError::InvalidPatch(format!(
                "patched file would exceed {MAX_PATCH_FILE_BYTES} bytes"
            )));
        }
        Ok(AppliedPatch {
            text,
            lines_added,
            lines_removed,
        })
    }
}

fn parse_header_path<'a>(line: &'a str, prefix: &str) -> Result<Option<&'a str>, ToolError> {
    let value = line.strip_prefix(prefix).ok_or_else(|| {
        ToolError::InvalidPatch(format!("expected {prefix:?} file header, got {line:?}"))
    })?;
    if value.is_empty() || value.contains('\t') {
        return Err(ToolError::InvalidPatch(
            "file headers require one exact path without timestamps".to_owned(),
        ));
    }
    if value == "/dev/null" {
        return Ok(None);
    }
    Ok(Some(
        value
            .strip_prefix("a/")
            .or_else(|| value.strip_prefix("b/"))
            .unwrap_or(value),
    ))
}

fn patch_identity(
    old_path: Option<&str>,
    new_path: Option<&str>,
) -> Result<(String, PatchOperation), ToolError> {
    match (old_path, new_path) {
        (None, Some(path)) => Ok((path.to_owned(), PatchOperation::Add)),
        (Some(path), None) => Ok((path.to_owned(), PatchOperation::Delete)),
        (Some(old), Some(new)) if old == new => Ok((old.to_owned(), PatchOperation::Update)),
        (Some(old), Some(new)) => Err(ToolError::InvalidPatch(format!(
            "rename patches are unsupported: old path {old:?} differs from new path {new:?}"
        ))),
        (None, None) => Err(ToolError::InvalidPatch(
            "both file headers cannot be /dev/null".to_owned(),
        )),
    }
}

fn validate_operation_shape(
    operation: PatchOperation,
    hunks: &[PatchHunk],
) -> Result<(), ToolError> {
    let old_total = checked_line_total(hunks.iter().map(|hunk| hunk.old_count))?;
    let new_total = checked_line_total(hunks.iter().map(|hunk| hunk.new_count))?;
    match operation {
        PatchOperation::Add if old_total != 0 => Err(ToolError::InvalidPatch(
            "added-file patches must have zero old lines".to_owned(),
        )),
        PatchOperation::Delete if new_total != 0 => Err(ToolError::InvalidPatch(
            "deleted-file patches must have zero new lines".to_owned(),
        )),
        PatchOperation::Add | PatchOperation::Update | PatchOperation::Delete => Ok(()),
    }
}

fn checked_line_total(mut counts: impl Iterator<Item = usize>) -> Result<usize, ToolError> {
    counts.try_fold(0_usize, |total, count| {
        total
            .checked_add(count)
            .filter(|total| *total <= MAX_PATCH_LINES)
            .ok_or_else(|| {
                ToolError::InvalidPatch(format!(
                    "declared hunk lines exceed the {MAX_PATCH_LINES}-line limit"
                ))
            })
    })
}

#[derive(Debug)]
struct PatchHunk {
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
    lines: Vec<HunkLine>,
    new_no_newline: bool,
}

impl PatchHunk {
    fn parse(
        header: &str,
        patch_lines: &[&str],
        mut cursor: usize,
        hunk_index: usize,
    ) -> Result<(Self, usize), ToolError> {
        let (old_start, old_count, new_start, new_count) = parse_hunk_header(header, hunk_index)?;
        let mut lines = Vec::new();
        let mut observed_old = 0_usize;
        let mut observed_new = 0_usize;
        let mut new_no_newline = false;
        let mut previous = None;
        while cursor < patch_lines.len() && !patch_lines[cursor].starts_with("@@ ") {
            let line = patch_lines[cursor];
            if line.is_empty() && cursor + 1 == patch_lines.len() {
                break;
            }
            if line == "\\ No newline at end of file" {
                match previous {
                    Some(HunkLineKind::Add | HunkLineKind::Context) => new_no_newline = true,
                    Some(HunkLineKind::Remove) => {}
                    None => {
                        return Err(ToolError::InvalidPatch(format!(
                            "hunk {hunk_index} has a newline marker without a preceding line"
                        )));
                    }
                }
                cursor = cursor.saturating_add(1);
                continue;
            }
            if new_no_newline {
                return Err(ToolError::InvalidPatch(format!(
                    "hunk {hunk_index} contains lines after a new-file no-newline marker"
                )));
            }
            let parsed = if let Some(content) = line.strip_prefix(' ') {
                observed_old = observed_old.saturating_add(1);
                observed_new = observed_new.saturating_add(1);
                previous = Some(HunkLineKind::Context);
                HunkLine::Context(content.to_owned())
            } else if let Some(content) = line.strip_prefix('-') {
                observed_old = observed_old.saturating_add(1);
                previous = Some(HunkLineKind::Remove);
                HunkLine::Remove(content.to_owned())
            } else if let Some(content) = line.strip_prefix('+') {
                observed_new = observed_new.saturating_add(1);
                previous = Some(HunkLineKind::Add);
                HunkLine::Add(content.to_owned())
            } else {
                return Err(ToolError::InvalidPatch(format!(
                    "hunk {hunk_index} line {} must start with space, `+`, or `-`",
                    cursor + 1
                )));
            };
            lines.push(parsed);
            cursor = cursor.saturating_add(1);
        }
        if observed_old != old_count || observed_new != new_count {
            return Err(ToolError::InvalidPatch(format!(
                "hunk {hunk_index} header declares -{old_count}/+{new_count} lines but body contains -{observed_old}/+{observed_new}"
            )));
        }
        if lines.is_empty()
            || lines
                .iter()
                .all(|line| matches!(line, HunkLine::Context(_)))
        {
            return Err(ToolError::InvalidPatch(format!(
                "hunk {hunk_index} must add or remove at least one line"
            )));
        }
        Ok((
            Self {
                old_start,
                old_count,
                new_start,
                new_count,
                lines,
                new_no_newline,
            },
            cursor,
        ))
    }
}

fn parse_hunk_header(
    header: &str,
    hunk_index: usize,
) -> Result<(usize, usize, usize, usize), ToolError> {
    let body = header
        .strip_prefix("@@ ")
        .and_then(|value| value.split_once(" @@"))
        .map(|(ranges, _)| ranges)
        .ok_or_else(|| {
            ToolError::InvalidPatch(format!("hunk {hunk_index} has an invalid header"))
        })?;
    let mut ranges = body.split_ascii_whitespace();
    let old = ranges.next().ok_or_else(|| {
        ToolError::InvalidPatch(format!("hunk {hunk_index} is missing the old range"))
    })?;
    let new = ranges.next().ok_or_else(|| {
        ToolError::InvalidPatch(format!("hunk {hunk_index} is missing the new range"))
    })?;
    if ranges.next().is_some() {
        return Err(ToolError::InvalidPatch(format!(
            "hunk {hunk_index} has unexpected range fields"
        )));
    }
    let (old_start, old_count) = parse_range(old, '-', hunk_index)?;
    let (new_start, new_count) = parse_range(new, '+', hunk_index)?;
    Ok((old_start, old_count, new_start, new_count))
}

fn parse_range(value: &str, prefix: char, hunk_index: usize) -> Result<(usize, usize), ToolError> {
    let value = value.strip_prefix(prefix).ok_or_else(|| {
        ToolError::InvalidPatch(format!(
            "hunk {hunk_index} range {value:?} must start with {prefix}"
        ))
    })?;
    let (start, count) = value
        .split_once(',')
        .map_or((value, "1"), |(start, count)| (start, count));
    let start = start.parse::<usize>().map_err(|_| {
        ToolError::InvalidPatch(format!(
            "hunk {hunk_index} has invalid range start {start:?}"
        ))
    })?;
    let count = count.parse::<usize>().map_err(|_| {
        ToolError::InvalidPatch(format!(
            "hunk {hunk_index} has invalid range count {count:?}"
        ))
    })?;
    if count > 0 && start == 0 {
        return Err(ToolError::InvalidPatch(format!(
            "hunk {hunk_index} uses line zero for a non-empty range"
        )));
    }
    Ok((start, count))
}

fn range_index(
    start: usize,
    count: usize,
    side: &str,
    hunk_index: usize,
) -> Result<usize, ToolError> {
    if count == 0 {
        Ok(start)
    } else if start == 0 {
        Err(ToolError::InvalidPatch(format!(
            "hunk {hunk_index} {side} range starts at zero but is non-empty"
        )))
    } else {
        Ok(start - 1)
    }
}

fn require_source_line(
    source: &[String],
    index: usize,
    expected: &str,
    hunk_index: usize,
) -> Result<(), ToolError> {
    let Some(actual) = source.get(index) else {
        return Err(ToolError::InvalidPatch(format!(
            "hunk {hunk_index} expected source line {} {:?}, but the file ended",
            index + 1,
            bounded_line(expected)
        )));
    };
    if actual == expected {
        Ok(())
    } else {
        Err(ToolError::InvalidPatch(format!(
            "hunk {hunk_index} source mismatch at line {}: expected {:?}, found {:?}",
            index + 1,
            bounded_line(expected),
            bounded_line(actual)
        )))
    }
}

fn bounded_line(value: &str) -> String {
    const LIMIT: usize = 160;
    let mut output = value.chars().take(LIMIT).collect::<String>();
    if value.chars().count() > LIMIT {
        output.push('…');
    }
    output
}

#[derive(Clone, Copy, Debug)]
enum HunkLineKind {
    Context,
    Remove,
    Add,
}

#[derive(Debug)]
enum HunkLine {
    Context(String),
    Remove(String),
    Add(String),
}

struct AppliedPatch {
    text: String,
    lines_added: usize,
    lines_removed: usize,
}

#[derive(Clone, Copy)]
enum LineEnding {
    Lf,
    CrLf,
}

impl LineEnding {
    const fn value(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
        }
    }
}

struct Document {
    lines: Vec<String>,
    line_ending: LineEnding,
    ends_with_newline: bool,
}

impl Document {
    fn parse(text: &str) -> Result<Self, ToolError> {
        let lf_count = text.bytes().filter(|byte| *byte == b'\n').count();
        let crlf_count = text
            .as_bytes()
            .windows(2)
            .filter(|pair| *pair == b"\r\n")
            .count();
        if crlf_count != 0 && crlf_count != lf_count {
            return Err(ToolError::InvalidPatch(
                "source file has mixed LF and CRLF line endings".to_owned(),
            ));
        }
        let line_ending = if crlf_count > 0 {
            LineEnding::CrLf
        } else {
            LineEnding::Lf
        };
        let ends_with_newline = text.ends_with('\n');
        let mut lines = text
            .split('\n')
            .map(|line| {
                if matches!(line_ending, LineEnding::CrLf) {
                    line.strip_suffix('\r').unwrap_or(line).to_owned()
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>();
        if ends_with_newline {
            lines.pop();
        }
        if text.is_empty() {
            lines.clear();
        }
        Ok(Self {
            lines,
            line_ending,
            ends_with_newline,
        })
    }

    fn render(lines: &[String], line_ending: LineEnding, ends_with_newline: bool) -> String {
        let mut output = lines.join(line_ending.value());
        if ends_with_newline && !lines.is_empty() {
            output.push_str(line_ending.value());
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use pactrail_workspace::WorkspaceTransaction;
    use proptest::prelude::*;
    use serde_json::json;

    use super::*;
    use crate::PolicyEngine;

    fn fixture(content: &str) -> (tempfile::TempDir, tempfile::TempDir, WorkspaceTransaction) {
        let source = tempfile::tempdir().unwrap_or_else(|error| unreachable!("source: {error}"));
        fs::write(source.path().join("sample.txt"), content)
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

    async fn apply(
        transaction: &WorkspaceTransaction,
        patch: &str,
    ) -> Result<ToolOutput, ToolError> {
        let policy = PolicyEngine::local_default();
        ApplyPatchTool
            .execute(
                &ToolContext::new(transaction, &policy, None),
                json!({"patch": patch}),
            )
            .await
    }

    #[tokio::test]
    async fn applies_multiple_exact_hunks_and_returns_current_source_evidence() {
        let (_source, _control, transaction) = fixture("alpha\nbeta\ngamma\ndelta\n");
        let output = apply(
            &transaction,
            "--- a/sample.txt\n+++ b/sample.txt\n@@ -1,2 +1,2 @@\n alpha\n-beta\n+BETA\n@@ -3,2 +3,3 @@\n gamma\n+inserted\n delta\n",
        )
        .await
        .unwrap_or_else(|error| unreachable!("patch: {error}"));

        assert_eq!(
            fs::read_to_string(transaction.workspace_root().join("sample.txt")).ok(),
            Some("alpha\nBETA\ngamma\ninserted\ndelta\n".to_owned())
        );
        assert_eq!(output.content["hunks"], 2);
        assert_eq!(output.content["lines_added"], 2);
        assert_eq!(output.content["lines_removed"], 1);
        assert_eq!(
            output.content["post_edit"]["changed_lines_fully_shown"],
            true
        );
    }

    #[tokio::test]
    async fn preserves_crlf_and_supports_no_final_newline() {
        let (_source, _control, transaction) = fixture("alpha\r\nbeta\r\n");
        apply(
            &transaction,
            "--- a/sample.txt\n+++ b/sample.txt\n@@ -1,2 +1,2 @@\n alpha\n-beta\n+BETA\n\\ No newline at end of file\n",
        )
        .await
        .unwrap_or_else(|error| unreachable!("patch: {error}"));

        assert_eq!(
            fs::read(transaction.workspace_root().join("sample.txt")).ok(),
            Some(b"alpha\r\nBETA".to_vec())
        );
    }

    #[tokio::test]
    async fn mismatch_and_stale_digest_leave_the_candidate_unchanged() {
        let (_source, _control, transaction) = fixture("alpha\nbeta\n");
        let mismatch = apply(
            &transaction,
            "--- a/sample.txt\n+++ b/sample.txt\n@@ -1,2 +1,2 @@\n alpha\n-missing\n+BETA\n",
        )
        .await;
        assert!(matches!(mismatch, Err(ToolError::InvalidPatch(_))));

        let policy = PolicyEngine::local_default();
        let stale = ApplyPatchTool
            .execute(
                &ToolContext::new(&transaction, &policy, None),
                json!({
                    "patch": "--- a/sample.txt\n+++ b/sample.txt\n@@ -1,2 +1,2 @@\n alpha\n-beta\n+BETA\n",
                    "expected_digest": "0".repeat(64)
                }),
            )
            .await;
        assert!(matches!(stale, Err(ToolError::InvalidPatch(_))));
        assert_eq!(
            fs::read_to_string(transaction.workspace_root().join("sample.txt")).ok(),
            Some("alpha\nbeta\n".to_owned())
        );
        assert!(
            transaction
                .changes()
                .is_ok_and(|changes| changes.is_empty())
        );
    }

    #[tokio::test]
    async fn creates_and_deletes_files_with_strict_dev_null_headers() {
        let (_source, _control, transaction) = fixture("alpha\n");
        let created = apply(
            &transaction,
            "--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,2 @@\n+one\n+two\n",
        )
        .await
        .unwrap_or_else(|error| unreachable!("add: {error}"));
        assert_eq!(created.content["operation"], "add");
        assert_eq!(
            fs::read_to_string(transaction.workspace_root().join("new.txt")).ok(),
            Some("one\ntwo\n".to_owned())
        );

        let deleted = apply(
            &transaction,
            "--- a/sample.txt\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-alpha\n",
        )
        .await
        .unwrap_or_else(|error| unreachable!("delete: {error}"));
        assert_eq!(deleted.content["operation"], "delete");
        assert!(!transaction.workspace_root().join("sample.txt").exists());
    }

    #[tokio::test]
    async fn applies_zero_count_insertions_at_the_declared_boundary() {
        let (_source, _control, transaction) = fixture("one\ntwo\nthree\n");
        apply(
            &transaction,
            "--- a/sample.txt\n+++ b/sample.txt\n@@ -2,0 +3,1 @@\n+inserted\n",
        )
        .await
        .unwrap_or_else(|error| unreachable!("insert: {error}"));
        assert_eq!(
            fs::read_to_string(transaction.workspace_root().join("sample.txt")).ok(),
            Some("one\ntwo\ninserted\nthree\n".to_owned())
        );
    }

    #[tokio::test]
    async fn rejects_renames_traversal_mixed_endings_and_inconsistent_headers() {
        let (_source, _control, transaction) = fixture("alpha\nbeta\n");
        for patch in [
            "--- a/sample.txt\n+++ b/renamed.txt\n@@ -1 +1 @@\n-alpha\n+ALPHA\n",
            "--- a/../sample.txt\n+++ b/../sample.txt\n@@ -1 +1 @@\n-alpha\n+ALPHA\n",
            "--- a/sample.txt\n+++ b/sample.txt\n@@ -1,2 +1,3 @@\n alpha\n-beta\n+BETA\n",
        ] {
            assert!(apply(&transaction, patch).await.is_err(), "patch={patch:?}");
        }

        let (_source, _control, mixed) = fixture("alpha\r\nbeta\n");
        assert!(
            apply(
                &mixed,
                "--- a/sample.txt\n+++ b/sample.txt\n@@ -1,2 +1,2 @@\n alpha\n-beta\n+BETA\n"
            )
            .await
            .is_err()
        );
    }

    proptest! {
        #[test]
        fn arbitrary_bounded_patch_text_is_a_total_parser(
            characters in proptest::collection::vec(any::<char>(), 0..4_096)
        ) {
            let input = characters.into_iter().collect::<String>();
            let _ = UnifiedPatch::parse(&input);
        }
    }
}
