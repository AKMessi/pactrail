use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pactrail_context::{
    ContextBudget, ContextPack, IndexBuildTelemetry, RepositoryIndex, RepositoryIndexBuild,
};
use pactrail_core::TaskContract;
use serde::Serialize;

const DEFAULT_FILES: usize = 2_000;
const MAX_FILES: usize = 100_000;
const DEFAULT_CONTEXT_BYTES: usize = 32 * 1024;
const DEFAULT_PHASE_BUDGET: Duration = Duration::from_mins(2);
const FIXTURE_ANCHOR_FILES: usize = 3;

#[derive(Clone, Copy)]
struct Config {
    files: usize,
    context_bytes: usize,
    max_cold: Duration,
    max_warm: Duration,
    max_incremental: Duration,
    max_context: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            files: DEFAULT_FILES,
            context_bytes: DEFAULT_CONTEXT_BYTES,
            max_cold: DEFAULT_PHASE_BUDGET,
            max_warm: DEFAULT_PHASE_BUDGET,
            max_incremental: DEFAULT_PHASE_BUDGET,
            max_context: DEFAULT_PHASE_BUDGET,
        }
    }
}

#[derive(Serialize)]
struct PhaseReport {
    elapsed_ms: u64,
    budget_ms: u64,
    telemetry: IndexBuildTelemetry,
}

#[derive(Serialize)]
struct ContextReport {
    elapsed_ms: u64,
    budget_ms: u64,
    rendered_bytes: usize,
    budget_bytes: usize,
    cited_files: usize,
    coverage_basis_points: u16,
    truncated: bool,
}

#[derive(Serialize)]
struct RepositoryScaleReport {
    schema_version: u32,
    fixture_profile: &'static str,
    generated_source_files: usize,
    indexed_files: usize,
    bytes_hashed: u64,
    repository_digest_stable_cold_to_warm: bool,
    cold: PhaseReport,
    warm: PhaseReport,
    incremental: PhaseReport,
    context: ContextReport,
    passed: bool,
    violations: Vec<String>,
}

// Keeping the measured lifecycle linear makes it harder to accidentally retain
// a previous index and distort a later phase's memory or latency measurement.
#[allow(clippy::too_many_lines)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = parse_config(env::args().skip(1))?;
    let source = tempfile::tempdir()?;
    let cache = tempfile::tempdir()?;
    create_fixture(source.path(), config.files)?;

    let cold_started = Instant::now();
    let RepositoryIndexBuild {
        index: cold_index,
        telemetry: cold_telemetry,
    } = RepositoryIndex::build_with_cache(source.path(), cache.path())?;
    let cold_elapsed = cold_started.elapsed();
    let cold_digest = cold_index.digest.clone();
    let indexed_files = cold_index.files.len();
    drop(cold_index);

    let warm_started = Instant::now();
    let RepositoryIndexBuild {
        index: warm_index,
        telemetry: warm_telemetry,
    } = RepositoryIndex::build_with_cache(source.path(), cache.path())?;
    let warm_elapsed = warm_started.elapsed();
    let warm_digest = warm_index.digest.clone();

    let context_budget = ContextBudget::new(config.context_bytes)?;
    let contract = TaskContract::new("change Unit42 and update its callers", ".");
    let context_started = Instant::now();
    let context = ContextPack::compile_with_budget(&contract, &warm_index, &[], context_budget)?;
    let context_elapsed = context_started.elapsed();
    drop(warm_index);
    let context_report = ContextReport {
        elapsed_ms: millis(context_elapsed),
        budget_ms: millis(config.max_context),
        rendered_bytes: context.rendered_bytes,
        budget_bytes: context.budget_bytes,
        cited_files: context.cited_files.len(),
        coverage_basis_points: context.retrieval.citation_coverage_basis_points,
        truncated: context.truncated,
    };
    drop(context);

    let changed_path = fixture_path(source.path(), 42);
    fs::write(changed_path, fixture_content(42, true))?;
    let incremental_started = Instant::now();
    let RepositoryIndexBuild {
        index: incremental_index,
        telemetry: incremental_telemetry,
    } = RepositoryIndex::build_with_cache(source.path(), cache.path())?;
    let incremental_elapsed = incremental_started.elapsed();
    drop(incremental_index);

    let mut violations = Vec::new();
    check_duration("cold index", cold_elapsed, config.max_cold, &mut violations);
    check_duration("warm index", warm_elapsed, config.max_warm, &mut violations);
    check_duration(
        "incremental index",
        incremental_elapsed,
        config.max_incremental,
        &mut violations,
    );
    check_duration(
        "context compile",
        context_elapsed,
        config.max_context,
        &mut violations,
    );
    if indexed_files != config.files.saturating_add(FIXTURE_ANCHOR_FILES) {
        violations.push(format!(
            "indexed {indexed_files} files, expected {}",
            config.files.saturating_add(FIXTURE_ANCHOR_FILES)
        ));
    }
    if cold_telemetry.cache_hits != 0
        || cold_telemetry.cache_misses != cold_telemetry.cache_eligible_files
    {
        violations.push("cold build did not analyze every eligible file cold".to_owned());
    }
    if warm_telemetry.cache_hits != warm_telemetry.cache_eligible_files
        || warm_telemetry.cache_misses != 0
    {
        violations.push("warm build did not reuse every eligible file".to_owned());
    }
    if incremental_telemetry.cache_misses != 1
        || incremental_telemetry.cache_hits.saturating_add(1)
            != incremental_telemetry.cache_eligible_files
    {
        violations.push("one-file edit did not produce exactly one cold cache entry".to_owned());
    }
    if cold_digest != warm_digest {
        violations.push("cold and warm repository digests differ".to_owned());
    }
    if context_report.rendered_bytes > context_report.budget_bytes {
        violations.push("context pack exceeded its deterministic byte budget".to_owned());
    }
    if context_report.cited_files == 0 {
        violations.push("targeted context did not cite a repository file".to_owned());
    }

    let passed = violations.is_empty();
    let report = RepositoryScaleReport {
        schema_version: 1,
        fixture_profile: "synthetic-polyglot-v1",
        generated_source_files: config.files,
        indexed_files,
        bytes_hashed: cold_telemetry.bytes_hashed,
        repository_digest_stable_cold_to_warm: cold_digest == warm_digest,
        cold: PhaseReport {
            elapsed_ms: millis(cold_elapsed),
            budget_ms: millis(config.max_cold),
            telemetry: cold_telemetry,
        },
        warm: PhaseReport {
            elapsed_ms: millis(warm_elapsed),
            budget_ms: millis(config.max_warm),
            telemetry: warm_telemetry,
        },
        incremental: PhaseReport {
            elapsed_ms: millis(incremental_elapsed),
            budget_ms: millis(config.max_incremental),
            telemetry: incremental_telemetry,
        },
        context: context_report,
        passed,
        violations,
    };
    let stdout = io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer_pretty(&mut output, &report)?;
    output.write_all(b"\n")?;
    output.flush()?;
    if !passed {
        return Err("repository-scale performance or correctness budget failed".into());
    }
    Ok(())
}

fn parse_config(
    arguments: impl Iterator<Item = String>,
) -> Result<Config, Box<dyn std::error::Error>> {
    let mut config = Config::default();
    let mut arguments = arguments.peekable();
    while let Some(argument) = arguments.next() {
        let value = arguments
            .next()
            .ok_or_else(|| format!("{argument} requires a value"))?;
        match argument.as_str() {
            "--files" => config.files = parse_usize(&value, "files")?,
            "--context-bytes" => config.context_bytes = parse_usize(&value, "context bytes")?,
            "--max-cold-ms" => config.max_cold = parse_duration(&value, "cold budget")?,
            "--max-warm-ms" => config.max_warm = parse_duration(&value, "warm budget")?,
            "--max-incremental-ms" => {
                config.max_incremental = parse_duration(&value, "incremental budget")?;
            }
            "--max-context-ms" => config.max_context = parse_duration(&value, "context budget")?,
            _ => return Err(format!("unknown repository-scale option {argument:?}").into()),
        }
    }
    if !(100..=MAX_FILES).contains(&config.files) {
        return Err(format!("files must be between 100 and {MAX_FILES}").into());
    }
    ContextBudget::new(config.context_bytes)?;
    Ok(config)
}

fn parse_usize(value: &str, name: &str) -> Result<usize, Box<dyn std::error::Error>> {
    value
        .parse::<usize>()
        .map_err(|error| format!("invalid {name} {value:?}: {error}").into())
}

fn parse_duration(value: &str, name: &str) -> Result<Duration, Box<dyn std::error::Error>> {
    let millis = value
        .parse::<u64>()
        .map_err(|error| format!("invalid {name} {value:?}: {error}"))?;
    if millis == 0 {
        return Err(format!("{name} must be positive").into());
    }
    Ok(Duration::from_millis(millis))
}

fn create_fixture(root: &Path, files: usize) -> io::Result<()> {
    fs::write(
        root.join("Cargo.toml"),
        b"[workspace]\nmembers = [\"packages/*\"]\nresolver = \"2\"\n",
    )?;
    fs::write(
        root.join("README.md"),
        b"# Synthetic repository-scale fixture\n\nGenerated deterministically by Pactrail.\n",
    )?;
    fs::write(
        root.join("AGENTS.md"),
        b"Keep public APIs deterministic and update callers with every change.\n",
    )?;
    for index in 0..files {
        let path = fixture_path(root, index);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, fixture_content(index, false))?;
    }
    Ok(())
}

fn fixture_path(root: &Path, index: usize) -> PathBuf {
    let extension = match index % 4 {
        0 => "rs",
        1 => "py",
        2 => "js",
        _ => "ts",
    };
    root.join(format!(
        "packages/package_{:04}/src/unit_{index:06}.{extension}",
        index % 128
    ))
}

fn fixture_content(index: usize, changed: bool) -> String {
    let revision = usize::from(changed);
    match index % 4 {
        0 => format!(
            "pub struct Unit{index} {{ pub value: usize }}\n\npub fn use_unit_{index}(input: Unit{index}) -> usize {{ input.value + {revision} }}\n"
        ),
        1 => format!(
            "class Unit{index}:\n    def __init__(self, value: int):\n        self.value = value\n\ndef use_unit_{index}(input: Unit{index}) -> int:\n    return input.value + {revision}\n"
        ),
        2 => format!(
            "export class Unit{index} {{ constructor(value) {{ this.value = value; }} }}\nexport function useUnit{index}(input) {{ return input.value + {revision}; }}\n"
        ),
        _ => format!(
            "export interface Unit{index} {{ value: number }}\nexport function useUnit{index}(input: Unit{index}): number {{ return input.value + {revision}; }}\n"
        ),
    }
}

fn check_duration(name: &str, actual: Duration, budget: Duration, violations: &mut Vec<String>) {
    if actual > budget {
        violations.push(format!(
            "{name} took {} ms, above the {} ms budget",
            millis(actual),
            millis(budget)
        ));
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
