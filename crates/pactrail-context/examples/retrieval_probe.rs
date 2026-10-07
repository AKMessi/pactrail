//! Offline retrieval measurement; never changes the production context strategy.
use std::collections::BTreeSet;
use std::env;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::Instant;

use pactrail_context::{RepositoryGraph, RepositoryIndex};
use serde_json::json;

fn graph_paths(index: &RepositoryIndex, query: &str, limit: usize) -> Vec<String> {
    let evidence = index.graph.query(query, limit, limit);
    let mut seen = BTreeSet::new();
    let mut paths = Vec::new();
    for symbol in evidence.symbols {
        for path in symbol
            .definitions
            .iter()
            .map(|item| &item.path)
            .chain(symbol.references.iter().map(|item| &item.path))
        {
            if seen.insert(path.clone()) {
                paths.push(path.clone());
                if paths.len() == limit {
                    return paths;
                }
            }
        }
    }
    paths
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let root = PathBuf::from(
        args.next()
            .ok_or("usage: retrieval_probe ROOT QUERY_FILE [K]")?,
    );
    let query_file = PathBuf::from(args.next().ok_or("query file required")?);
    let limit = args
        .next()
        .map(|value| {
            value
                .into_string()
                .map_err(|_| "K must be UTF-8")
                .and_then(|value| value.parse::<usize>().map_err(|_| "K must be an integer"))
        })
        .transpose()?
        .unwrap_or(20);
    if args.next().is_some() || !(1..=100).contains(&limit) {
        return Err("K must be 1–100; no extra arguments".into());
    }
    let mut bytes = Vec::new();
    File::open(query_file)?
        .take(65_537)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 65_536 {
        return Err("query exceeds 64 KiB".into());
    }
    let query = String::from_utf8(bytes)?;
    let started = Instant::now();
    let index = RepositoryIndex::build(&root)?;
    let index_ms = started.elapsed().as_millis();
    let lexical = RepositoryIndex {
        graph: RepositoryGraph::default(),
        ..index.clone()
    };
    let started = Instant::now();
    let lexical_paths: Vec<_> = lexical
        .retrieve(&query, limit)
        .iter()
        .map(|file| &file.path)
        .collect();
    let lexical_ms = started.elapsed().as_millis();
    let started = Instant::now();
    let graph = graph_paths(&index, &query, limit);
    let graph_ms = started.elapsed().as_millis();
    let started = Instant::now();
    let hybrid: Vec<_> = index
        .retrieve(&query, limit)
        .iter()
        .map(|file| &file.path)
        .collect();
    let hybrid_ms = started.elapsed().as_millis();
    let report = json!({"schema_version": 1, "repository_digest": index.digest,
        "query_digest": blake3::hash(query.as_bytes()).to_hex().to_string(),
        "k": limit, "indexed_files": index.files.len(), "index_ms": index_ms,
        "arms": {"lexical": {"paths": lexical_paths, "elapsed_ms": lexical_ms},
                 "graph": {"paths": graph, "elapsed_ms": graph_ms},
                 "current_hybrid": {"paths": hybrid, "elapsed_ms": hybrid_ms}},
        "graph_truncated": index.graph.truncated,
        "interpretation": "navigation ranking only; no model-quality, token-budget or exhaustive relevance claim"});
    serde_json::to_writer_pretty(io::stdout().lock(), &report)?;
    writeln!(io::stdout().lock())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pactrail_context::SymbolLocation;

    #[test]
    fn graph_measurement_is_bounded_and_deduplicates_repeated_paths() {
        let mut index = RepositoryIndex::default();
        index.graph.definitions.insert(
            "Receipt".to_owned(),
            (0..4)
                .map(|line| SymbolLocation {
                    path: if line < 2 { "a.rs" } else { "b.rs" }.to_owned(),
                    line: line + 1,
                    kind: "struct".to_owned(),
                })
                .collect(),
        );
        assert_eq!(graph_paths(&index, "Receipt", 1).len(), 1);
        assert_eq!(graph_paths(&index, "Receipt", 4), ["a.rs", "b.rs"]);
        assert!(graph_paths(&index, "missing", 4).is_empty());
    }
}
