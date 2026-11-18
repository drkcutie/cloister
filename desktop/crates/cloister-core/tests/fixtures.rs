//! Seam A: runs every shared JSON scenario in `fixtures/rules` against the Rust rules engine.
//! The format is documented in `fixtures/README.md`; Kotlin and Postgres runners read the same files.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use cloister_core::{decide, enforcement, Command, Enforcement, Outcome, State};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    name: String,
    #[allow(dead_code)]
    description: String,
    initial_state: State,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    server_time: DateTime<Utc>,
    #[serde(default)]
    command: Option<Command>,
    expect: Expect,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Expect {
    outcome: Option<String>,
    reason: Option<String>,
    state: Option<Value>,
    enforcement: Option<ExpectEnforcement>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ExpectEnforcement {
    blocked_domains: Option<Vec<String>>,
    #[serde(default)]
    blocked_domains_include: Vec<String>,
    #[serde(default)]
    blocked_domains_exclude: Vec<String>,
    blocked_apps: Option<Vec<String>>,
    ritual: Option<String>,
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/rules")
}

fn run(path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let scenario: Scenario = serde_json::from_str(&text).map_err(|e| format!("parse: {e}"))?;
    let mut state = scenario.initial_state;
    for (i, step) in scenario.steps.into_iter().enumerate() {
        let at = |msg: String| format!("{} step {i}: {msg}", scenario.name);
        let current = match &step.command {
            Some(command) => {
                let decision = decide(&state, command, step.server_time);
                let (outcome, reason) = match &decision.outcome {
                    Outcome::Accepted => ("accepted".to_string(), None),
                    Outcome::Rejected { reason } => {
                        ("rejected".to_string(), Some(reason.code().to_string()))
                    }
                };
                let want = step
                    .expect
                    .outcome
                    .as_deref()
                    .ok_or_else(|| at("a step with a command must expect an outcome".into()))?;
                if outcome != want || reason != step.expect.reason {
                    return Err(at(format!(
                        "expected {want} {:?}, got {outcome} {reason:?}",
                        step.expect.reason
                    )));
                }
                if outcome == "rejected" && decision.state != state {
                    return Err(at("a rejected command changed the state".into()));
                }
                state = decision.state;
                decision.enforcement
            }
            None => enforcement(&state, step.server_time),
        };
        if let Some(want) = &step.expect.state {
            let got = serde_json::to_value(&state).unwrap();
            if &got != want {
                return Err(at(format!("state mismatch\n want {want}\n  got {got}")));
            }
        }
        if let Some(want) = &step.expect.enforcement {
            check_enforcement(&current, want).map_err(at)?;
        }
    }
    Ok(())
}

fn check_enforcement(got: &Enforcement, want: &ExpectEnforcement) -> Result<(), String> {
    let domains = &got.blocked_domains;
    if let Some(exact) = &want.blocked_domains {
        let mut exact = exact.clone();
        exact.sort();
        if &exact != domains {
            return Err(format!("blocked_domains: want {exact:?}, got {domains:?}"));
        }
    }
    for d in &want.blocked_domains_include {
        if !domains.contains(d) {
            return Err(format!("{d} should be blocked, got {domains:?}"));
        }
    }
    for d in &want.blocked_domains_exclude {
        if domains.contains(d) {
            return Err(format!("{d} should not be blocked"));
        }
    }
    if let Some(apps) = &want.blocked_apps {
        let mut apps = apps.clone();
        apps.sort();
        if apps != got.blocked_apps {
            return Err(format!(
                "blocked_apps: want {apps:?}, got {:?}",
                got.blocked_apps
            ));
        }
    }
    if let Some(ritual) = &want.ritual {
        let got_ritual = serde_json::to_value(got.ritual).unwrap();
        if got_ritual != Value::String(ritual.clone()) {
            return Err(format!("ritual: want {ritual}, got {got_ritual}"));
        }
    }
    Ok(())
}

#[test]
fn every_shared_fixture_passes() {
    let mut paths: Vec<_> = std::fs::read_dir(fixtures_dir())
        .expect("fixtures/rules exists")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no fixtures found");
    let failures: Vec<String> = paths.iter().filter_map(|p| run(p).err()).collect();
    assert!(
        failures.is_empty(),
        "{} fixture(s) failed:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
