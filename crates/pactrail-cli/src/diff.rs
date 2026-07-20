use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use pactrail_core::{ChangeReceipt, FileChange};
use similar::TextDiff;
use tempfile::NamedTempFile;
use thiserror::Error;

const MAX_DIFF_FILE_BYTES: u64 = 512 * 1024;
const MAX_DIFF_OUTPUT_BYTES: usize = 1024 * 1024;
const REVIEW_FILE: &str = "review.diff";

pub(crate) fn render_receipt_diff(
    run_root: &Path,
    receipt: &ChangeReceipt,
) -> Result<String, DiffError> {
    let review = run_root.join(REVIEW_FILE);
    if review.is_file() {
        return read_review(&review);
    }
    render_live_diff(run_root, receipt)
}

pub(crate) fn write_receipt_diff(
    run_root: &Path,
    receipt: &ChangeReceipt,
) -> Result<(), DiffError> {
    let review = run_root.join(REVIEW_FILE);
    let rendered = render_live_diff(run_root, receipt)?;
    if review.exists() {
        if read_review(&review)? == rendered {
            return Ok(());
        }
        return Err(DiffError::ImmutableReviewMismatch(review));
    }
    let mut temporary = NamedTempFile::new_in(run_root).map_err(|source| DiffError::Io {
        path: run_root.to_path_buf(),
        source,
    })?;
    temporary
        .write_all(rendered.as_bytes())
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|source| DiffError::Io {
            path: temporary.path().to_path_buf(),
            source,
        })?;
    temporary
        .persist_noclobber(&review)
        .map_err(|error| DiffError::Io {
            path: review,
            source: error.error,
        })?;
    Ok(())
}

fn render_live_diff(run_root: &Path, receipt: &ChangeReceipt) -> Result<String, DiffError> {
    let source_root = PathBuf::from(&receipt.contract.workspace_root);
    let candidate_root = run_root.join("workspace");
    let mut output = String::new();
    for change in &receipt.changes {
        if output.len() >= MAX_DIFF_OUTPUT_BYTES {
            output.push_str("\n... diff output truncated ...\n");
            break;
        }
        render_change(&mut output, &source_root, &candidate_root, change)?;
    }
    if receipt.changes.is_empty() {
        output.push_str("No file changes.\n");
    }
    truncate_diff(&mut output);
    Ok(output)
}

fn read_review(path: &Path) -> Result<String, DiffError> {
    let file = fs::File::open(path).map_err(|source| DiffError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut bytes = Vec::new();
    file.take((MAX_DIFF_OUTPUT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| DiffError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if bytes.len() > MAX_DIFF_OUTPUT_BYTES {
        return Err(DiffError::ReviewTooLarge(path.to_path_buf()));
    }
    String::from_utf8(bytes).map_err(|_| DiffError::ReviewNotUtf8(path.to_path_buf()))
}

fn truncate_diff(output: &mut String) {
    if output.len() <= MAX_DIFF_OUTPUT_BYTES {
        return;
    }
    let marker = "\n... diff output truncated ...\n";
    let mut boundary = MAX_DIFF_OUTPUT_BYTES.saturating_sub(marker.len());
    while !output.is_char_boundary(boundary) {
        boundary = boundary.saturating_sub(1);
    }
    output.truncate(boundary);
    output.push_str(marker);
}

fn render_change(
    output: &mut String,
    source_root: &Path,
    candidate_root: &Path,
    change: &FileChange,
) -> Result<(), DiffError> {
    let before_path = source_root.join(&change.path);
    let after_path = candidate_root.join(&change.path);
    let before = read_text(&before_path)?;
    let after = read_text(&after_path)?;
    match (before, after) {
        (FileContents::Text(before), FileContents::Text(after)) => {
            render_text_change(output, change, &before, &after);
        }
        (FileContents::Missing, FileContents::Text(after)) => {
            render_text_change(output, change, "", &after);
        }
        (FileContents::Text(before), FileContents::Missing) => {
            render_text_change(output, change, &before, "");
        }
        (FileContents::Missing, FileContents::Missing) => {
            output.push_str("--- ");
            output.push_str(&change.path);
            output.push_str(" (missing)\n");
        }
        _ => {
            output.push_str("--- a/");
            output.push_str(&change.path);
            output.push_str("\n+++ b/");
            output.push_str(&change.path);
            output.push_str("\nBinary or oversized file changed.\n");
        }
    }
    Ok(())
}

fn render_text_change(output: &mut String, change: &FileChange, before: &str, after: &str) {
    let diff = TextDiff::from_lines(before, after);
    let old_header = if change.before_digest.is_some() {
        format!("a/{}", change.path)
    } else {
        "/dev/null".to_owned()
    };
    let new_header = if change.after_digest.is_some() {
        format!("b/{}", change.path)
    } else {
        "/dev/null".to_owned()
    };
    output.push_str(
        &diff
            .unified_diff()
            .context_radius(3)
            .header(&old_header, &new_header)
            .to_string(),
    );
}

enum FileContents {
    Missing,
    Text(String),
    BinaryOrLarge,
}

fn read_text(path: &Path) -> Result<FileContents, DiffError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileContents::Missing);
        }
        Err(source) => {
            return Err(DiffError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if !metadata.is_file() || metadata.len() > MAX_DIFF_FILE_BYTES {
        return Ok(FileContents::BinaryOrLarge);
    }
    let bytes = fs::read(path).map_err(|source| DiffError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    match String::from_utf8(bytes) {
        Ok(text) if !text.contains('\0') => Ok(FileContents::Text(text)),
        Ok(_) | Err(_) => Ok(FileContents::BinaryOrLarge),
    }
}

#[derive(Debug, Error)]
pub(crate) enum DiffError {
    #[error("could not read diff input {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("immutable review artifact changed at {0}")]
    ImmutableReviewMismatch(PathBuf),
    #[error("review artifact exceeds the safety limit at {0}")]
    ReviewTooLarge(PathBuf),
    #[error("review artifact is not UTF-8 at {0}")]
    ReviewNotUtf8(PathBuf),
}

#[cfg(test)]
mod tests {
    use pactrail_core::{ChangeReceipt, ReceiptInput, ReceiptOutcome, RunId, TaskContract};

    use super::*;

    #[test]
    fn renders_unified_diff_from_transaction_workspace() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("tempdir: {error}"));
        let source = root.path().join("source");
        let run = root.path().join("run");
        fs::create_dir_all(run.join("workspace"))
            .unwrap_or_else(|error| unreachable!("workspace: {error}"));
        fs::create_dir(&source).unwrap_or_else(|error| unreachable!("source: {error}"));
        fs::write(source.join("hello.txt"), "before\n")
            .unwrap_or_else(|error| unreachable!("before: {error}"));
        fs::write(run.join("workspace/hello.txt"), "after\n")
            .unwrap_or_else(|error| unreachable!("after: {error}"));
        let mut contract = TaskContract::new("edit", source.display().to_string());
        contract.allowed_write_paths = vec!["hello.txt".to_owned()];
        contract.obligations.clear();
        let receipt = ChangeReceipt::build(ReceiptInput {
            run_id: RunId::new(),
            contract,
            outcome: ReceiptOutcome::ReadyToApply,
            baseline_digest: "baseline".to_owned(),
            final_event_hash: "event".to_owned(),
            changes: vec![FileChange {
                path: "hello.txt".to_owned(),
                before_digest: Some("before".to_owned()),
                after_digest: Some("after".to_owned()),
                before_unix_mode: None,
                after_unix_mode: None,
                bytes_added: 6,
                bytes_removed: 7,
            }],
            evidence: Vec::new(),
            approvals: Vec::new(),
            unresolved_risks: Vec::new(),
        })
        .unwrap_or_else(|error| unreachable!("receipt: {error}"));
        let diff = render_receipt_diff(&run, &receipt)
            .unwrap_or_else(|error| unreachable!("diff: {error}"));
        assert!(diff.contains("-before"));
        assert!(diff.contains("+after"));
    }

    #[test]
    fn persisted_review_survives_source_and_candidate_removal() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("tempdir: {error}"));
        let source = root.path().join("source");
        let run = root.path().join("run");
        fs::create_dir_all(run.join("workspace"))
            .unwrap_or_else(|error| unreachable!("workspace: {error}"));
        fs::create_dir(&source).unwrap_or_else(|error| unreachable!("source: {error}"));
        fs::write(source.join("hello.txt"), "before\n")
            .unwrap_or_else(|error| unreachable!("before: {error}"));
        fs::write(run.join("workspace/hello.txt"), "after\n")
            .unwrap_or_else(|error| unreachable!("after: {error}"));
        let receipt = fixture_receipt(&source);
        write_receipt_diff(&run, &receipt).unwrap_or_else(|error| unreachable!("persist: {error}"));
        fs::remove_dir_all(&source).unwrap_or_else(|error| unreachable!("source removal: {error}"));
        fs::remove_dir_all(run.join("workspace"))
            .unwrap_or_else(|error| unreachable!("candidate removal: {error}"));
        let diff = render_receipt_diff(&run, &receipt)
            .unwrap_or_else(|error| unreachable!("stored diff: {error}"));
        assert!(diff.contains("-before"));
        assert!(diff.contains("+after"));
    }

    fn fixture_receipt(source: &Path) -> ChangeReceipt {
        let mut contract = TaskContract::new("edit", source.display().to_string());
        contract.allowed_write_paths = vec!["hello.txt".to_owned()];
        contract.obligations.clear();
        ChangeReceipt::build(ReceiptInput {
            run_id: RunId::new(),
            contract,
            outcome: ReceiptOutcome::ReadyToApply,
            baseline_digest: "baseline".to_owned(),
            final_event_hash: "event".to_owned(),
            changes: vec![FileChange {
                path: "hello.txt".to_owned(),
                before_digest: Some("before".to_owned()),
                after_digest: Some("after".to_owned()),
                before_unix_mode: None,
                after_unix_mode: None,
                bytes_added: 6,
                bytes_removed: 7,
            }],
            evidence: Vec::new(),
            approvals: Vec::new(),
            unresolved_risks: Vec::new(),
        })
        .unwrap_or_else(|error| unreachable!("receipt: {error}"))
    }
}
