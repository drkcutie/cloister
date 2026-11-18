//! Seam A against the backend: runs every shared JSON scenario in `fixtures/rules` through the
//! Postgres rules (the `command` function and the triggers on `blocks`) on a local Supabase.
//!
//! Needs a running local Supabase with the migrations applied (`backend/scripts/test.sh` does
//! everything). The database URL comes from `CLOISTER_DB_URL`, defaulting to the local stack.
//!
//! Each scenario runs as a fresh user inside one transaction that is rolled back. Server time
//! is pinned per step with the `cloister.server_time` setting, which the backend honours only
//! for the database owner (a client can't set it).

use std::path::{Path, PathBuf};

use cloister_core::{Command, EntryKind, State};
use postgres::{Client, NoTls, Transaction};
use serde::Deserialize;
use serde_json::{json, Value};

const DEFAULT_DB_URL: &str = "postgresql://postgres:postgres@127.0.0.1:56422/postgres";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    name: String,
    #[allow(dead_code)]
    description: String,
    initial_state: Value,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    server_time: String,
    #[serde(default)]
    command: Option<Value>,
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

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn connect() -> Client {
    let url = std::env::var("CLOISTER_DB_URL").unwrap_or_else(|_| DEFAULT_DB_URL.into());
    Client::connect(&url, NoTls).unwrap_or_else(|e| {
        panic!(
            "can't reach the local Supabase database at {url} ({e}); run backend/scripts/test.sh"
        )
    })
}

fn query_json(
    tx: &mut Transaction,
    sql: &str,
    params: &[&(dyn postgres::types::ToSql + Sync)],
) -> Value {
    tx.query_one(sql, params).unwrap().get::<_, Value>(0)
}

/// Become a fresh signed-in user with `initial_state` already in the database.
fn sign_in_with_state(tx: &mut Transaction, scenario: &str, initial_state: &Value) {
    let user_id: String = tx
        .query_one(
            "insert into auth.users (id, email) values (gen_random_uuid(), $1) returning id::text",
            &[&format!("{scenario}@fixtures.test")],
        )
        .unwrap()
        .get(0);
    // History is written as-is: a scenario's initial state is a given, not a command.
    tx.batch_execute("set local session_replication_role = replica")
        .unwrap();
    let state: State = serde_json::from_value(initial_state.clone()).expect("initial_state");
    for block in &state.blocks {
        tx.execute(
            "insert into public.blocks
               (user_id, id, starts_at, ends_at, blocklists, deep, intent, depth_rating,
                extra_domains, extra_apps, quit_social)
             values ($1::text::uuid, $2, $3::text::timestamptz, $4::text::timestamptz, $5, $6, $7,
                     $8::int::smallint, $9, $10, $11)",
            &[
                &user_id,
                &block.id,
                &block.starts_at.to_rfc3339(),
                &block.ends_at.to_rfc3339(),
                &block.blocklists,
                &block.deep,
                &block.intent,
                &block.depth_rating.map(i32::from),
                &block.extra_domains,
                &block.extra_apps,
                &block.quit_social,
            ],
        )
        .unwrap();
    }
    tx.execute(
        "update public.accounts set time_zone = $2 where user_id = $1::text::uuid",
        &[&user_id, &state.time_zone],
    )
    .unwrap();
    let kind = |k: EntryKind| {
        serde_json::to_value(k)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    };
    for e in &state.blocklist_entries {
        tx.execute(
            "insert into public.blocklist_entries (user_id, blocklist, kind, entry)
             values ($1::text::uuid, $2, $3, $4)",
            &[&user_id, &e.blocklist, &kind(e.kind), &e.entry],
        )
        .unwrap();
    }
    for r in &state.blocklist_removals {
        tx.execute(
            "insert into public.blocklist_removals (user_id, blocklist, kind, entry, effective_at)
             values ($1::text::uuid, $2, $3, $4, $5::text::timestamptz)",
            &[
                &user_id,
                &r.blocklist,
                &kind(r.kind),
                &r.entry,
                &r.effective_at.to_rfc3339(),
            ],
        )
        .unwrap();
    }
    for plan in &state.day_plans {
        tx.execute(
            "insert into public.day_plans
               (user_id, date, starts_at, ends_at, planned_deep_minutes, slots)
             values ($1::text::uuid, $2::text::date, $3::text::timestamptz, $4::text::timestamptz,
                     $5::int, $6)",
            &[
                &user_id,
                &plan.date.to_string(),
                &plan.starts_at.to_rfc3339(),
                &plan.ends_at.to_rfc3339(),
                &(plan.planned_deep_minutes as i32),
                &serde_json::to_value(&plan.slots).unwrap(),
            ],
        )
        .unwrap();
    }
    for o in &state.blocklist_opt_outs {
        tx.execute(
            "insert into public.blocklist_opt_outs (user_id, blocklist, effective_at)
             values ($1::text::uuid, $2, $3::text::timestamptz)",
            &[&user_id, &o.blocklist, &o.effective_at.to_rfc3339()],
        )
        .unwrap();
    }
    for schedule in &state.schedules {
        let weekdays: Vec<i16> = schedule.weekdays.iter().map(|&d| d.into()).collect();
        tx.execute(
            "insert into public.schedules (user_id, id, time_zone, weekdays, starts, ends, slots)
             values ($1::text::uuid, $2, $3, $4, $5, $6, $7)",
            &[
                &user_id,
                &schedule.id,
                &schedule.time_zone,
                &weekdays,
                &schedule.starts,
                &schedule.ends,
                &serde_json::to_value(&schedule.slots).unwrap(),
            ],
        )
        .unwrap();
    }
    for b in &state.breaches {
        tx.execute(
            "insert into public.breaches (user_id, id, kind, occurred_at)
             values ($1::text::uuid, $2, $3, $4::text::timestamptz)",
            &[
                &user_id,
                &b.id,
                &serde_json::to_value(b.kind).unwrap().as_str().unwrap(),
                &b.occurred_at.to_rfc3339(),
            ],
        )
        .unwrap();
    }
    if let Some(settings) = &state.settings {
        let days = |d: &[u8]| d.iter().map(|&d| i16::from(d)).collect::<Vec<i16>>();
        let pending = state.pending_settings.as_ref();
        tx.execute(
            "insert into public.settings
               (user_id, workdays, workday_start, shutdown_time, pending_workdays,
                pending_workday_start, pending_shutdown_time, pending_effective_at,
                settings_since, past_settings)
             values ($1::text::uuid, $2, $3, $4, $5, $6, $7, $8::text::timestamptz,
                     $9::text::timestamptz, $10)",
            &[
                &user_id,
                &days(&settings.workdays),
                &settings.workday_start,
                &settings.shutdown_time,
                &pending.map(|p| days(&p.workdays)),
                &pending.map(|p| p.workday_start.clone()),
                &pending.map(|p| p.shutdown_time.clone()),
                &pending.map(|p| p.effective_at.to_rfc3339()),
                &state.settings_since.map(|t| t.to_rfc3339()),
                &serde_json::to_value(&state.past_settings).unwrap(),
            ],
        )
        .unwrap();
    }
    if let Some(target) = &state.weekly_target {
        let pending = state.pending_weekly_target.as_ref();
        tx.execute(
            "insert into public.weekly_targets (user_id, hours, pending_hours, pending_effective_at)
             values ($1::text::uuid, $2::int::smallint, $3::int::smallint, $4::text::timestamptz)",
            &[
                &user_id,
                &i32::from(target.hours),
                &pending.map(|p| i32::from(p.hours)),
                &pending.map(|p| p.effective_at.to_rfc3339()),
            ],
        )
        .unwrap();
    }
    for l in &state.loose_ends {
        tx.execute(
            "insert into public.loose_ends (user_id, id, text, captured_at)
             values ($1::text::uuid, $2, $3, $4::text::timestamptz)",
            &[&user_id, &l.id, &l.text, &l.captured_at.to_rfc3339()],
        )
        .unwrap();
    }
    for r in &state.rituals {
        tx.execute(
            "insert into public.rituals (user_id, date, completed_at)
             values ($1::text::uuid, $2::text::date, $3::text::timestamptz)",
            &[&user_id, &r.date.to_string(), &r.completed_at.to_rfc3339()],
        )
        .unwrap();
    }
    for d in &state.shutdown_delays {
        tx.execute(
            "insert into public.shutdown_delays (user_id, date, used_at)
             values ($1::text::uuid, $2::text::date, $3::text::timestamptz)",
            &[&user_id, &d.date.to_string(), &d.used_at.to_rfc3339()],
        )
        .unwrap();
    }
    tx.batch_execute("set local session_replication_role = origin")
        .unwrap();
    let claims = json!({ "sub": user_id, "role": "authenticated" }).to_string();
    tx.execute(
        "select set_config('request.jwt.claims', $1, true)",
        &[&claims],
    )
    .unwrap();
    tx.batch_execute("set local role authenticated").unwrap();
}

fn run(client: &mut Client, path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let scenario: Scenario = serde_json::from_str(&text).map_err(|e| format!("parse: {e}"))?;
    let mut tx = client.transaction().map_err(|e| e.to_string())?;
    sign_in_with_state(&mut tx, &scenario.name, &scenario.initial_state);
    for (i, step) in scenario.steps.iter().enumerate() {
        let at = |msg: String| format!("{} step {i}: {msg}", scenario.name);
        tx.execute(
            "select set_config('cloister.server_time', $1, true)",
            &[&step.server_time],
        )
        .unwrap();
        if let Some(command) = &step.command {
            // Same parse as the Rust engine, so a typo in a fixture fails here too.
            serde_json::from_value::<Command>(command.clone())
                .map_err(|e| at(format!("command: {e}")))?;
            let before = query_json(&mut tx, "select public.state()", &[]);
            let result = query_json(&mut tx, "select public.command($1)", &[command]);
            let outcome = result["outcome"].as_str().unwrap_or_default().to_string();
            let reason = result["reason"].as_str().map(str::to_string);
            let want = step
                .expect
                .outcome
                .as_deref()
                .ok_or_else(|| at("a step with a command must expect an outcome".into()))?;
            if outcome != want || reason != step.expect.reason {
                return Err(at(format!(
                    "expected {want} {:?}, got {result}",
                    step.expect.reason
                )));
            }
            if outcome == "rejected" && query_json(&mut tx, "select public.state()", &[]) != before
            {
                return Err(at("a rejected command changed the state".into()));
            }
        }
        if let Some(want) = &step.expect.state {
            let got = query_json(&mut tx, "select public.state()", &[]);
            if &got != want {
                return Err(at(format!("state mismatch\n want {want}\n  got {got}")));
            }
        }
        if let Some(want) = &step.expect.enforcement {
            let got = query_json(&mut tx, "select public.enforcement()", &[]);
            check_enforcement(&got, want).map_err(at)?;
        }
    }
    tx.rollback().map_err(|e| e.to_string())
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn check_enforcement(got: &Value, want: &ExpectEnforcement) -> Result<(), String> {
    let domains = strings(&got["blocked_domains"]);
    if let Some(exact) = &want.blocked_domains {
        let mut exact = exact.clone();
        exact.sort();
        if exact != domains {
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
        let got_apps = strings(&got["blocked_apps"]);
        if apps != got_apps {
            return Err(format!("blocked_apps: want {apps:?}, got {got_apps:?}"));
        }
    }
    if let Some(ritual) = &want.ritual {
        if got["ritual"] != Value::String(ritual.clone()) {
            return Err(format!("ritual: want {ritual}, got {}", got["ritual"]));
        }
    }
    Ok(())
}

#[test]
#[ignore = "needs a local Supabase; run backend/scripts/test.sh"]
fn every_shared_fixture_passes_against_postgres() {
    let mut paths: Vec<_> = std::fs::read_dir(repo_root().join("fixtures/rules"))
        .expect("fixtures/rules exists")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no fixtures found");
    let mut client = connect();
    let failures: Vec<String> = paths
        .iter()
        .filter_map(|p| run(&mut client, p).err())
        .collect();
    assert!(
        failures.is_empty(),
        "{} fixture(s) failed against Postgres:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
#[ignore = "needs a local Supabase; run backend/scripts/test.sh"]
fn the_backend_catalog_matches_the_shared_blocklists() {
    let shared: Value = serde_json::from_str(
        &std::fs::read_to_string(repo_root().join("shared/blocklists.json")).unwrap(),
    )
    .unwrap();
    let mut client = connect();
    let got: Value = client
        .query_one("select public.catalog()", &[])
        .unwrap()
        .get(0);
    assert_eq!(got["blocklists"], shared["blocklists"]);
    assert_eq!(got["related_domains"], shared["related_domains"]);
}
