//! Backend-only guarantees that the shared fixtures can't express: row ownership and a
//! tampered client writing to tables directly (ADR 0004). Needs a local Supabase
//! (`backend/scripts/test.sh`).

use postgres::{Client, NoTls, Transaction};
use serde_json::{json, Value};

fn connect() -> Client {
    let url = std::env::var("CLOISTER_DB_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@127.0.0.1:56422/postgres".into());
    Client::connect(&url, NoTls).expect("local Supabase database")
}

fn new_user(tx: &mut Transaction, email: &str) -> String {
    tx.query_one(
        "insert into auth.users (id, email) values (gen_random_uuid(), $1) returning id::text",
        &[&email],
    )
    .unwrap()
    .get(0)
}

fn act_as(tx: &mut Transaction, user_id: &str) {
    tx.batch_execute("reset role").unwrap();
    let claims = json!({ "sub": user_id, "role": "authenticated" }).to_string();
    tx.execute("select set_config('request.jwt.claims', $1, true)", &[&claims])
        .unwrap();
    tx.batch_execute("set local role authenticated").unwrap();
}

fn command(tx: &mut Transaction, cmd: Value) -> Value {
    tx.query_one("select public.command($1)", &[&cmd]).unwrap().get(0)
}

/// Run `sql` in a savepoint; returns the SQLSTATE and message if it failed.
fn try_sql(tx: &mut Transaction, sql: &str) -> Option<(String, String)> {
    let mut sp = tx.transaction().unwrap();
    match sp.batch_execute(sql) {
        Ok(()) => {
            sp.commit().unwrap();
            None
        }
        Err(e) => {
            let db = e.as_db_error().expect("a database error");
            Some((db.code().code().to_string(), db.message().to_string()))
        }
    }
}

#[test]
#[ignore = "needs a local Supabase; run backend/scripts/test.sh"]
fn each_user_sees_only_their_own_blocks_and_devices() {
    let mut client = connect();
    let mut tx = client.transaction().unwrap();
    let a = new_user(&mut tx, "a@rules.test");
    let b = new_user(&mut tx, "b@rules.test");
    act_as(&mut tx, &a);
    let started = command(&mut tx, json!({"type": "start_block", "block_id": "a1", "minutes": 30}));
    assert_eq!(started["outcome"], "accepted");
    tx.batch_execute(
        "insert into public.devices (id, name, platform) values (gen_random_uuid(), 'Laptop', 'macos')",
    )
    .unwrap();

    act_as(&mut tx, &b);
    let blocks: i64 = tx.query_one("select count(*) from public.blocks", &[]).unwrap().get(0);
    let devices: i64 = tx.query_one("select count(*) from public.devices", &[]).unwrap().get(0);
    assert_eq!((blocks, devices), (0, 0));
    let state: Value = tx.query_one("select public.state()", &[]).unwrap().get(0);
    assert_eq!(state, json!({"blocks": []}));
    // B can't extend or even find A's Block.
    let extended = command(
        &mut tx,
        json!({"type": "extend_block", "block_id": "a1", "ends_at": "2100-01-01T00:00:00Z"}),
    );
    assert_eq!(extended["reason"], "unknown_block");
    tx.rollback().unwrap();
}

#[test]
#[ignore = "needs a local Supabase; run backend/scripts/test.sh"]
fn a_tampered_client_cant_write_blocks_directly() {
    let mut client = connect();
    let mut tx = client.transaction().unwrap();
    let a = new_user(&mut tx, "tamper@rules.test");
    act_as(&mut tx, &a);
    command(&mut tx, json!({"type": "start_block", "block_id": "b1", "minutes": 60}));
    for sql in [
        "update public.blocks set ends_at = now() where id = 'b1'",
        "delete from public.blocks where id = 'b1'",
        "insert into public.blocks (id, starts_at, ends_at, blocklists) values ('x', now(), now() + interval '1 minute', '{}')",
    ] {
        let (code, _) = try_sql(&mut tx, sql).unwrap_or_else(|| panic!("{sql} should fail"));
        assert_eq!(code, "42501", "{sql}: permission denied");
    }
    tx.rollback().unwrap();
}

#[test]
#[ignore = "needs a local Supabase; run backend/scripts/test.sh"]
fn the_triggers_hold_even_for_privileged_writes() {
    let mut client = connect();
    let mut tx = client.transaction().unwrap();
    let a = new_user(&mut tx, "owner@rules.test");
    act_as(&mut tx, &a);
    command(&mut tx, json!({"type": "start_block", "block_id": "b1", "minutes": 60}));
    tx.batch_execute("reset role").unwrap();
    let shorten = try_sql(
        &mut tx,
        "update public.blocks set ends_at = ends_at - interval '1 minute' where id = 'b1'",
    );
    assert_eq!(shorten, Some(("CL001".into(), "block_cannot_be_shortened".into())));
    let cancel = try_sql(&mut tx, "delete from public.blocks where id = 'b1'");
    assert_eq!(cancel, Some(("CL001".into(), "block_cannot_be_cancelled".into())));
    let loosen = try_sql(&mut tx, "update public.blocks set blocklists = '{}' where id = 'b1'");
    assert_eq!(loosen, Some(("CL001".into(), "block_cannot_be_changed".into())));
    // Deleting the account is the one way its Blocks go.
    let gone = try_sql(&mut tx, &format!("delete from auth.users where id = '{a}'"));
    assert_eq!(gone, None);
    tx.rollback().unwrap();
}

#[test]
#[ignore = "needs a local Supabase; run backend/scripts/test.sh"]
fn a_client_cant_pin_server_time() {
    // Connect the way the API does: as `authenticator`, switching to the request's role.
    let url = std::env::var("CLOISTER_DB_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@127.0.0.1:56422/postgres".into())
        .replacen("postgres:", "authenticator:", 1);
    let mut client = Client::connect(&url, NoTls).expect("connect as authenticator");
    client
        .batch_execute(
            "set role authenticated; \
             select set_config('cloister.server_time', '2000-01-01T00:00:00Z', false)",
        )
        .unwrap();
    let pinned: bool = client
        .query_one("select public.server_now() < '2001-01-01'", &[])
        .unwrap()
        .get(0);
    assert!(!pinned, "only the database owner may pin server time");
}
