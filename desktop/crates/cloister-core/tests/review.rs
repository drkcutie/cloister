//! The scoreboard (`week_summary`) against the shared scenarios in `fixtures/review`.
//! The Kotlin engine runs the same files.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use cloister_core::{week_summary, State};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    name: String,
    #[allow(dead_code)]
    description: String,
    state: State,
    at: DateTime<Utc>,
    expect: Value,
}

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/review")
}

#[test]
fn every_review_scenario_passes() {
    let mut paths: Vec<_> = std::fs::read_dir(dir())
        .expect("fixtures/review exists")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty());
    let failures: Vec<String> = paths
        .iter()
        .filter_map(|p| {
            let s: Scenario = serde_json::from_str(&std::fs::read_to_string(p).unwrap())
                .unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            let got = serde_json::to_value(week_summary(&s.state, s.at)).unwrap();
            (got != s.expect).then(|| format!("{}:\n want {}\n  got {got}", s.name, s.expect))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
