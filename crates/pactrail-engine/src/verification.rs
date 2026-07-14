use std::fs;
use std::path::Path;

use serde_json::Value;

/// Deterministic repository check discovered from project manifests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationCommand {
    pub program: String,
    pub args: Vec<String>,
    pub description: String,
}

/// Detects conservative, non-installing test commands from repository manifests.
#[must_use]
pub fn detect_verification_commands(root: &Path) -> Vec<VerificationCommand> {
    let mut commands = Vec::new();
    if root.join("Cargo.toml").is_file() {
        commands.push(VerificationCommand {
            program: "cargo".to_owned(),
            args: vec![
                "test".to_owned(),
                "--workspace".to_owned(),
                "--all-targets".to_owned(),
            ],
            description: "Rust workspace tests".to_owned(),
        });
    }
    if root.join("go.mod").is_file() {
        commands.push(VerificationCommand {
            program: "go".to_owned(),
            args: vec!["test".to_owned(), "./...".to_owned()],
            description: "Go module tests".to_owned(),
        });
    }
    if has_python_tests(root) {
        commands.push(VerificationCommand {
            program: "python".to_owned(),
            args: vec!["-m".to_owned(), "pytest".to_owned()],
            description: "Python tests".to_owned(),
        });
    }
    if package_has_test_script(root) {
        let program = if root.join("pnpm-lock.yaml").is_file() {
            "pnpm"
        } else if root.join("yarn.lock").is_file() {
            "yarn"
        } else if root.join("bun.lock").is_file() || root.join("bun.lockb").is_file() {
            "bun"
        } else {
            "npm"
        };
        commands.push(VerificationCommand {
            program: program.to_owned(),
            args: vec!["test".to_owned()],
            description: "JavaScript package tests".to_owned(),
        });
    }
    commands
}

fn has_python_tests(root: &Path) -> bool {
    ["pyproject.toml", "pytest.ini", "tox.ini"]
        .iter()
        .any(|name| root.join(name).is_file())
        || root.join("tests").is_dir()
}

fn package_has_test_script(root: &Path) -> bool {
    let path = root.join("package.json");
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|value| {
            value
                .pointer("/scripts/test")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .is_some_and(|script| !script.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_polyglot_checks_without_install_commands() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        fs::write(root.path().join("Cargo.toml"), "[workspace]\n")
            .unwrap_or_else(|error| unreachable!("cargo: {error}"));
        fs::write(
            root.path().join("package.json"),
            r#"{"scripts":{"test":"vitest run"}}"#,
        )
        .unwrap_or_else(|error| unreachable!("package: {error}"));
        let commands = detect_verification_commands(root.path());
        assert_eq!(commands.len(), 2);
        assert!(
            commands
                .iter()
                .all(|command| !command.args.contains(&"install".to_owned()))
        );
    }
}
