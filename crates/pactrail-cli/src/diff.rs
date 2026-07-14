use std::fs;
use std::path::{Path, PathBuf};

use pactrail_core::{ChangeReceipt, FileChange};
use similar::TextDiff;
use thiserror::Error;

const MAX_DIFF_FILE_BYTES: u64 = 512 * 1024;
const MAX_DIFF_OUTPUT_BYTES: usize = 1024 * 1024;

pub(crate) fn render_receipt_diff(
    run_root: &Path,
    receipt: &ChangeReceipt,
) -> Result<String, DiffError> {
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
    Ok(output)
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
            let diff = TextDiff::from_lines(&before, &after);
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
            unresolved_risks: Vec::new(),
        })
        .unwrap_or_else(|error| unreachable!("receipt: {error}"));
        let diff = render_receipt_diff(&run, &receipt)
            .unwrap_or_else(|error| unreachable!("diff: {error}"));
        assert!(diff.contains("-before"));
        assert!(diff.contains("+after"));
    }
}
