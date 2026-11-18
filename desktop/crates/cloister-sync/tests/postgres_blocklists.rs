//! Backend-only Blocklist guarantees the shared fixtures can't express: a tampered client (or
//! the service role) writing the Blocklist tables directly still can't loosen anything sooner
//! than tomorrow (ADR 0001, ADR 0004). Needs a local Supabase (`backend/scripts/test.sh`).

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
    tx.execute(
        "select set_config('request.jwt.claims', $1, true)",
        &[&claims],
    )
    .unwrap();
    tx.batch_execute("set local role authenticated").unwrap();
}

fn as_owner(tx: &mut Transaction) {
    tx.batch_execute("reset role").unwrap();
}

fn at(tx: &mut Transaction, time: &str) {
    tx.execute(
        "select set_config('cloister.server_time', $1, true)",
        &[&time],
    )
    .unwrap();
}

fn command(tx: &mut Transaction, cmd: Value) -> Value {
    tx.query_one("select public.command($1)", &[&cmd])
        .unwrap()
        .get(0)
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

fn loosening_refused(result: Option<(String, String)>) {
    assert_eq!(
        result,
        Some(("CL001".into(), "blocklist_cannot_be_loosened".into()))
    );
}

#[test]
#[ignore = "needs a local Supabase; run backend/scripts/test.sh"]
fn clients_can_only_read_their_own_blocklist_edits() {
    let mut client = connect();
    let mut tx = client.transaction().unwrap();
    let a = new_user(&mut tx, "a@blocklists.test");
    let b = new_user(&mut tx, "b@blocklists.test");
    act_as(&mut tx, &a);
    let added = command(
        &mut tx,
        json!({"type": "add_blocklist_entry", "blocklist": "work", "kind": "domain", "entry": "figma.com"}),
    );
    assert_eq!(added["outcome"], "accepted");

    let direct = [
        format!("insert into public.blocklist_entries (user_id, blocklist, kind, entry) values ('{a}', 'work', 'domain', 'x.org')"),
        "delete from public.blocklist_entries".to_string(),
        format!("insert into public.blocklist_removals (user_id, blocklist, kind, entry, effective_at) values ('{a}', 'social', 'domain', 'x.com', now())"),
        format!("insert into public.blocklist_opt_outs (user_id, blocklist, effective_at) values ('{a}', 'news', now())"),
    ];
    for sql in direct {
        let failed = try_sql(&mut tx, &sql).map(|(code, _)| code);
        assert_eq!(failed.as_deref(), Some("42501"), "{sql}");
    }

    act_as(&mut tx, &b);
    let seen: i64 = tx
        .query_one("select count(*) from public.blocklist_entries", &[])
        .unwrap()
        .get(0);
    assert_eq!(seen, 0, "B can't see A's Blocklist edits");
    tx.rollback().unwrap();
}

#[test]
#[ignore = "needs a local Supabase; run backend/scripts/test.sh"]
fn not_even_the_service_role_loosens_a_blocklist_today() {
    let mut client = connect();
    let mut tx = client.transaction().unwrap();
    let a = new_user(&mut tx, "owner@blocklists.test");
    at(&mut tx, "2026-01-05T09:00:00Z");
    tx.execute(
        "update public.accounts set time_zone = 'Europe/Paris' where user_id = $1::text::uuid",
        &[&a],
    )
    .unwrap();
    tx.execute(
        "insert into public.blocklist_entries (user_id, blocklist, kind, entry)
         values ($1::text::uuid, 'work', 'domain', 'Figma.com')",
        &[&a],
    )
    .unwrap();

    // An addition can't be deleted or edited away.
    loosening_refused(try_sql(&mut tx, "delete from public.blocklist_entries"));
    loosening_refused(try_sql(
        &mut tx,
        "update public.blocklist_entries set entry = 'other.com'",
    ));

    // A removal written directly with an immediate date is moved to the next local midnight.
    tx.execute(
        "insert into public.blocklist_removals (user_id, blocklist, kind, entry, effective_at)
         values ($1::text::uuid, 'social', 'domain', 'x.com', '2000-01-01')",
        &[&a],
    )
    .unwrap();
    let effective: String = tx
        .query_one(
            "select cloister.rfc3339(effective_at) from public.blocklist_removals",
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(effective, "2026-01-05T23:00:00Z", "midnight in Paris");
    loosening_refused(try_sql(
        &mut tx,
        "update public.blocklist_removals set effective_at = '2000-01-01'",
    ));

    // Same for opting out of a category; the Work List can't be opted out of at all.
    tx.execute(
        "insert into public.blocklist_opt_outs (user_id, blocklist, effective_at)
         values ($1::text::uuid, 'news', '2000-01-01')",
        &[&a],
    )
    .unwrap();
    let effective: String = tx
        .query_one(
            "select cloister.rfc3339(effective_at) from public.blocklist_opt_outs",
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(effective, "2026-01-05T23:00:00Z");
    loosening_refused(try_sql(
        &mut tx,
        "update public.blocklist_opt_outs set effective_at = '2000-01-01'",
    ));
    assert_eq!(
        try_sql(
            &mut tx,
            &format!("insert into public.blocklist_opt_outs (user_id, blocklist, effective_at) values ('{a}', 'work', now())"),
        ),
        Some(("CL001".into(), "cannot_opt_out".into()))
    );

    // Deleting the account still takes everything with it.
    as_owner(&mut tx);
    tx.execute("delete from auth.users where id = $1::text::uuid", &[&a])
        .unwrap();
    tx.rollback().unwrap();
}

#[test]
#[ignore = "needs a local Supabase; run backend/scripts/test.sh"]
fn a_quit_social_block_cant_be_forged_or_stretched_through_start_block() {
    let mut client = connect();
    let mut tx = client.transaction().unwrap();
    let a = new_user(&mut tx, "quit@blocklists.test");
    act_as(&mut tx, &a);
    at(&mut tx, "2026-01-05T09:00:00Z");
    let forged = command(
        &mut tx,
        json!({"type": "start_block", "block_id": "b1", "minutes": 60, "quit_social": true}),
    );
    assert_eq!(forged["reason"], "invalid_command");
    let started = command(
        &mut tx,
        json!({"type": "start_quit_social_block", "block_id": "q1", "confirm_long": true}),
    );
    assert_eq!(started["outcome"], "accepted");
    let state: Value = tx.query_one("select public.state()", &[]).unwrap().get(0);
    assert_eq!(state["blocks"][0]["ends_at"], "2026-02-04T09:00:00Z");
    assert_eq!(state["blocks"][0]["quit_social"], true);
    tx.rollback().unwrap();
}
