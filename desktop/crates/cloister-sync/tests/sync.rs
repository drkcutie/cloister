//! Sync between Devices against a local Supabase (`backend/scripts/test.sh`): a Block started on
//! one Device is enforced on another without user action, and an offline Device keeps
//! enforcing what it knows and catches up when it reconnects.

mod common;

use std::path::Path;

use chrono::{Datelike, Utc};
use std::time::{Duration, Instant};

use cloister_core::{Command, EntryKind, RitualScreen, SlotInput};
use cloister_sync::{
    Backend, CommandOutcome, Device, DeviceInfo, DeviceStatus, Platform, SyncOptions, SyncStatus,
};

fn device(backend: &Backend, mirror: &Path, name: &str) -> Device {
    Device::open(
        backend.clone(),
        mirror,
        DeviceInfo {
            name: name.into(),
            platform: Platform::Macos,
        },
    )
    .unwrap()
}

/// Two Devices signed in to one new account.
fn two_devices(dir: &Path, prefix: &str) -> (Device, Device) {
    two_devices_as(dir, &common::unique_email(prefix))
}

fn two_devices_as(dir: &Path, email: &str) -> (Device, Device) {
    let backend = common::backend();
    let a = device(&backend, &dir.join("a.sqlite"), "Desktop A");
    a.sign_in(common::sign_in(&backend, email)).unwrap();
    std::thread::sleep(Duration::from_millis(1100)); // one sign-in email per second
    let b = device(&backend, &dir.join("b.sqlite"), "Desktop B");
    b.sign_in(common::sign_in(&backend, email)).unwrap();
    (a, b)
}

fn start_block(minutes: i64, blocklists: &[&str]) -> Command {
    Command::StartBlock {
        block_id: cloister_sync::new_block_id(),
        minutes,
        deep: true,
        blocklists: blocklists.iter().map(|s| s.to_string()).collect(),
        confirm_long: false,
        client_time: None,
        intent: None,
        extra_domains: vec![],
        extra_apps: vec![],
    }
}

fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("timed out waiting for {what}");
}

fn blocks(device: &Device, domain: &str) -> bool {
    device
        .enforcement()
        .unwrap()
        .blocked_domains
        .iter()
        .any(|d| d == domain)
}

/// Realtime only: no periodic pulls, so only a pushed change can reach the Device.
fn realtime_only() -> SyncOptions {
    SyncOptions {
        poll_every: Duration::from_secs(3600),
        ..SyncOptions::default()
    }
}

#[test]
#[ignore = "needs a local Supabase; run backend/scripts/test.sh"]
fn a_block_started_on_one_device_is_enforced_on_another() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = two_devices(dir.path(), "realtime");
    let sync = b.start_sync(realtime_only());
    wait_until("B online", || b.sync_status() == SyncStatus::Online);
    assert!(!blocks(&b, "youtube.com"));

    assert_eq!(
        a.send(&start_block(30, &[])).unwrap(),
        CommandOutcome::Accepted
    );
    assert!(
        blocks(&a, "youtube.com"),
        "A enforces its own Block at once"
    );
    wait_until("B enforces the Block started on A", || {
        blocks(&b, "youtube.com")
    });
    sync.stop();
}

#[test]
#[ignore = "needs a local Supabase; run backend/scripts/test.sh"]
fn the_backend_refuses_to_cancel_a_block_from_any_device() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = two_devices(dir.path(), "cancel");
    let command = start_block(30, &[]);
    let Command::StartBlock { block_id, .. } = &command else {
        unreachable!()
    };
    a.send(&command).unwrap();
    let outcome = b
        .send(&Command::CancelBlock {
            block_id: block_id.clone(),
        })
        .unwrap();
    assert_eq!(
        outcome,
        CommandOutcome::Rejected {
            reason: "block_cannot_be_cancelled".into()
        }
    );
    assert!(blocks(&b, "youtube.com"));
}

#[test]
#[ignore = "needs a local Supabase; run backend/scripts/test.sh"]
fn an_offline_device_keeps_enforcing_and_catches_up_on_reconnect() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = two_devices(dir.path(), "offline");
    a.send(&start_block(30, &[])).unwrap();
    b.pull().unwrap();
    drop(b);

    // B loses the network: it still enforces the Block it knows and says so.
    let unreachable = Backend::new("http://127.0.0.1:9", common::backend().anon_key());
    let offline = device(&unreachable, &dir.path().join("b.sqlite"), "Desktop B");
    assert!(offline.pull().is_err());
    assert_eq!(offline.sync_status(), SyncStatus::Offline);
    assert_eq!(offline.sync_status().label(), "Offline, still enforcing");
    assert!(blocks(&offline, "youtube.com"));
    drop(offline);

    // Meanwhile A starts a Block covering the Work List.
    a.send(&start_block(30, &["work"])).unwrap();

    // B comes back and picks it up without user action.
    let b = device(
        &common::backend(),
        &dir.path().join("b.sqlite"),
        "Desktop B",
    );
    assert!(!blocks(&b, "github.com"));
    let sync = b.start_sync(realtime_only());
    wait_until("B picks up the Block started while it was offline", || {
        blocks(&b, "github.com")
    });
    assert!(blocks(&b, "youtube.com"));
    sync.stop();
}

#[test]
#[ignore = "needs a local Supabase; run backend/scripts/test.sh"]
fn a_block_started_offline_is_enforced_at_once_and_reaches_the_account_later() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = two_devices(dir.path(), "queue");
    drop(b);
    let unreachable = Backend::new("http://127.0.0.1:9", common::backend().anon_key());
    let offline = device(&unreachable, &dir.path().join("b.sqlite"), "Desktop B");
    let block = start_block(30, &["work"]);
    assert_eq!(offline.send(&block).unwrap(), CommandOutcome::Accepted);
    assert!(blocks(&offline, "github.com"), "enforced here at once");
    assert_eq!(offline.queued().unwrap(), 1);
    // A loosening can't be queued: it needs the backend.
    let cancel = Command::CancelBlock {
        block_id: "x".into(),
    };
    assert!(offline
        .send(&Command::OptOutOfBlocklist {
            blocklist: "news".into()
        })
        .is_err());
    assert!(offline.send(&cancel).is_err());
    drop(offline);
    // Back online, the queue goes to the backend, which decides again.
    let b = device(
        &common::backend(),
        &dir.path().join("b.sqlite"),
        "Desktop B",
    );
    b.pull().unwrap();
    assert_eq!(b.queued().unwrap(), 0);
    a.pull().unwrap();
    assert!(blocks(&a, "github.com"), "the other Device has it");
}

#[test]
#[ignore = "needs a local Supabase; run backend/scripts/test.sh"]
fn the_devices_list_shows_each_device_and_its_status() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = two_devices(dir.path(), "devices");
    a.report_enforcing(true).unwrap();
    let list = b.devices().unwrap();
    let summary: Vec<_> = list
        .iter()
        .map(|d| (d.name.as_str(), d.status, d.this_device))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("Desktop A", DeviceStatus::Enforcing, false),
            ("Desktop B", DeviceStatus::Offline, true),
        ]
    );
}

/// A REST call as the signed-in user; returns the HTTP status and the body.
fn rest(
    session: &cloister_sync::Session,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> (u16, String) {
    let backend = common::backend();
    let req = ureq::request(method, &format!("{}/rest/v1/{path}", backend.url()))
        .set("apikey", backend.anon_key())
        .set("Authorization", &format!("Bearer {}", session.access_token))
        .set("Prefer", "return=minimal");
    let answer = match body {
        Some(b) => req.send_json(b),
        None => req.call(),
    };
    match answer {
        Ok(r) => (r.status(), r.into_string().unwrap_or_default()),
        Err(ureq::Error::Status(code, r)) => (code, r.into_string().unwrap_or_default()),
        Err(e) => panic!("{e}"),
    }
}

/// A Device's heartbeat, the way every client sends it (`rpc/device_heartbeat`).
fn heartbeat(session: &cloister_sync::Session, device: serde_json::Value) -> u16 {
    rest(
        session,
        "POST",
        "rpc/device_heartbeat",
        Some(serde_json::json!({ "device": device })),
    )
    .0
}

/// A phone registering itself the way the Android app does (`android/sync` Device.register).
fn register_phone(session: &cloister_sync::Session, id: &str, hardcore: bool) -> u16 {
    heartbeat(
        session,
        serde_json::json!({
            "id": id, "name": "Pixel", "platform": "android", "enforcing": true, "hardcore": hardcore,
        }),
    )
}

#[test]
#[ignore = "needs a local Supabase; run backend/scripts/test.sh"]
fn the_devices_list_says_which_phone_runs_hardcore_mode() {
    let dir = tempfile::tempdir().unwrap();
    let (_a, b) = two_devices(dir.path(), "hardcore");
    std::thread::sleep(Duration::from_millis(1100));
    let phone = common::sign_in(&common::backend(), &account_email(&b));
    let id = "6f1c1a52-0a7e-4f5e-9d47-6b8f0f3c2a11";
    assert_eq!(register_phone(&phone, id, true), 204);
    let list = b.devices().unwrap();
    let pixel = list
        .iter()
        .find(|d| d.name == "Pixel")
        .expect("the phone is listed");
    assert!(pixel.hardcore);
    assert!(list
        .iter()
        .filter(|d| d.name != "Pixel")
        .all(|d| !d.hardcore));
}

#[test]
#[ignore = "needs a local Supabase; run backend/scripts/test.sh"]
fn only_a_phone_can_claim_hardcore_mode() {
    let dir = tempfile::tempdir().unwrap();
    let (a, _b) = two_devices(dir.path(), "hardcore-desktop");
    std::thread::sleep(Duration::from_millis(1100));
    let session = common::sign_in(&common::backend(), &account_email(&a));
    let status = heartbeat(
        &session,
        serde_json::json!({
            "id": "0d4b3a0e-6e0b-4b8a-8f86-3c1b2d6f9e01", "name": "Mac", "platform": "macos",
            "enforcing": true, "hardcore": true,
        }),
    );
    assert_eq!(status, 400);
}

#[test]
#[ignore = "needs a local Supabase; run backend/scripts/test.sh"]
fn devices_are_written_only_by_their_own_heartbeat() {
    let backend = common::backend();
    let mine = common::sign_in(&backend, &common::unique_email("heartbeat-a"));
    std::thread::sleep(Duration::from_millis(1100));
    let theirs = common::sign_in(&backend, &common::unique_email("heartbeat-b"));
    let id = cloister_sync::new_block_id();
    let laptop = |enforcing: bool| serde_json::json!({ "id": id, "name": "Laptop", "platform": "macos", "enforcing": enforcing });
    assert_eq!(heartbeat(&mine, laptop(false)), 204);

    // No direct writes: a tampered client can't fake a row, its status or when it was seen.
    let fake = serde_json::json!({
        "id": cloister_sync::new_block_id(), "name": "Fake", "platform": "macos", "enforcing": true,
        "last_seen_at": "2100-01-01T00:00:00Z",
    });
    let (code, _) = rest(&mine, "POST", "devices", Some(fake));
    assert!(code == 401 || code == 403, "insert: {code}");
    let filter = format!("devices?id=eq.{id}");
    let (code, _) = rest(
        &mine,
        "PATCH",
        &filter,
        Some(serde_json::json!({ "enforcing": true, "last_seen_at": "2100-01-01T00:00:00Z" })),
    );
    assert!(code == 401 || code == 403, "update: {code}");
    let (code, _) = rest(&mine, "DELETE", &filter, None);
    assert!(code == 401 || code == 403, "delete: {code}");

    // The heartbeat can't carry last_seen_at, created_at or a user id.
    let stamped = serde_json::json!({
        "id": id, "name": "Laptop", "platform": "macos", "enforcing": true,
        "last_seen_at": "2100-01-01T00:00:00Z",
    });
    assert_eq!(heartbeat(&mine, stamped), 400);

    // Another account can't take over (or even touch) this Device's id.
    assert_eq!(heartbeat(&theirs, laptop(true)), 400);

    // The owner's heartbeat updates the row; the server stamps last_seen_at; platform stays.
    let before = Utc::now();
    let renamed = serde_json::json!({ "id": id, "name": "Work laptop", "platform": "linux", "enforcing": true });
    assert_eq!(heartbeat(&mine, renamed), 204);
    let (code, body) = rest(
        &mine,
        "GET",
        &format!("devices?select=name,platform,enforcing,last_seen_at&id=eq.{id}"),
        None,
    );
    assert_eq!(code, 200);
    let rows: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(rows.as_array().map(Vec::len), Some(1), "{rows}");
    let row = &rows[0];
    assert_eq!(row["name"], "Work laptop");
    assert_eq!(row["platform"], "macos");
    assert_eq!(row["enforcing"], true);
    let seen: chrono::DateTime<Utc> = serde_json::from_value(row["last_seen_at"].clone()).unwrap();
    assert!(
        seen > before - chrono::Duration::seconds(30)
            && seen < Utc::now() + chrono::Duration::seconds(30),
        "{seen}"
    );
    // The other account sees nothing of it.
    let (_, body) = rest(
        &theirs,
        "GET",
        &format!("devices?select=id&id=eq.{id}"),
        None,
    );
    assert_eq!(body.trim(), "[]");
}

/// The email `device`'s account signed in with.
fn account_email(device: &Device) -> String {
    device.email().expect("signed in")
}

#[test]
#[ignore = "needs a local Supabase; run backend/scripts/test.sh"]
fn a_blocklist_addition_on_one_device_applies_on_another_at_once_but_a_removal_waits() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = two_devices(dir.path(), "blocklists");
    let sync = b.start_sync(realtime_only());
    wait_until("B online", || b.sync_status() == SyncStatus::Online);
    assert_eq!(
        a.send(&start_block(30, &["work"])).unwrap(),
        CommandOutcome::Accepted
    );
    wait_until("B enforces the Block", || blocks(&b, "github.com"));

    let add = Command::AddBlocklistEntry {
        blocklist: "work".into(),
        kind: EntryKind::Domain,
        entry: "figma.com".into(),
    };
    assert_eq!(a.send(&add).unwrap(), CommandOutcome::Accepted);
    wait_until("B blocks the added domain", || blocks(&b, "figma.com"));

    let remove = Command::RemoveBlocklistEntry {
        blocklist: "work".into(),
        kind: EntryKind::Domain,
        entry: "github.com".into(),
    };
    assert_eq!(a.send(&remove).unwrap(), CommandOutcome::Accepted);
    wait_until("B knows the pending removal", || {
        !b.state().unwrap().blocklist_removals.is_empty()
    });
    assert!(blocks(&b, "github.com"), "removals take effect tomorrow");
    assert!(blocks(&a, "github.com"));
    sync.stop();
}

#[test]
#[ignore = "needs a local Supabase; run backend/scripts/test.sh"]
fn settings_changed_on_one_device_lock_down_another() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = two_devices(dir.path(), "settings");
    let sync = b.start_sync(realtime_only());
    wait_until("B online", || b.sync_status() == SyncStatus::Online);
    assert!(!blocks(&b, "github.com"));
    // No Workdays at all: every day is a Rest Day, whatever the time now.
    let rest_every_day = Command::ChangeSettings {
        workdays: vec![],
        workday_start: "09:00".into(),
        shutdown_time: "17:00".into(),
        time_zone: Some("Europe/Paris".into()),
    };
    assert_eq!(a.send(&rest_every_day).unwrap(), CommandOutcome::Accepted);
    wait_until("B locks down", || blocks(&b, "github.com"));
    assert_eq!(b.state().unwrap().time_zone, "Europe/Paris");
    assert_eq!(
        a.send(&start_block(30, &[])).unwrap(),
        CommandOutcome::Rejected {
            reason: "rest_day".into()
        }
    );
    sync.stop();
}

fn ritual(device: &Device) -> RitualScreen {
    device.enforcement().unwrap().ritual
}

#[test]
#[ignore = "needs a local Supabase; run backend/scripts/test.sh"]
fn completing_the_ritual_on_one_device_drops_it_on_another() {
    let dir = tempfile::tempdir().unwrap();
    let email = common::unique_email("ritual");
    let (a, b) = two_devices_as(dir.path(), &email);
    // Every day a Workday from 00:00 to a minute ago (UTC), so the ritual is due now.
    let now = a.server_time();
    let shutdown = (now - chrono::Duration::minutes(1))
        .format("%H:%M")
        .to_string();
    if shutdown.as_str() < "00:05" {
        return; // Too close to midnight for a Workday that has ended today.
    }
    let settings = Command::ChangeSettings {
        workdays: vec![1, 2, 3, 4, 5, 6, 7],
        workday_start: "00:00".into(),
        shutdown_time: shutdown.clone(),
        time_zone: Some("UTC".into()),
    };
    assert_eq!(a.send(&settings).unwrap(), CommandOutcome::Accepted);
    // Made now, the settings have no ritual due yet today (none before settings_since), so
    // date them back a day as the database owner.
    let url = std::env::var("CLOISTER_DB_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@127.0.0.1:56422/postgres".into());
    let mut db = postgres::Client::connect(&url, postgres::NoTls).unwrap();
    db.batch_execute("set session_replication_role = replica")
        .unwrap();
    db.execute(
        "update public.settings s set settings_since = s.settings_since - interval '1 day'
         from public.accounts a where a.user_id = s.user_id and a.email = $1",
        &[&email],
    )
    .unwrap();
    a.pull().unwrap();
    b.pull().unwrap();
    assert_eq!(ritual(&b), RitualScreen::Due);
    let sync = b.start_sync(realtime_only());
    wait_until("B online", || b.sync_status() == SyncStatus::Online);

    let tomorrow = now.date_naive() + chrono::Duration::days(1);
    let at = |hhmm: &str| {
        tomorrow
            .and_time(chrono::NaiveTime::parse_from_str(hhmm, "%H:%M").unwrap())
            .and_utc()
    };
    let steps = [
        Command::CaptureLooseEnd {
            loose_end_id: "l1".into(),
            text: "Send the invoice".into(),
        },
        Command::WriteDayPlan {
            date: tomorrow,
            starts_at: at("00:00"),
            ends_at: at(&shutdown),
            slots: vec![SlotInput {
                id: cloister_sync::new_block_id(),
                starts_at: at("00:00"),
                ends_at: at(&shutdown),
                label: "Draft chapter 4".into(),
                deep: true,
            }],
        },
        Command::CompleteRitual {
            typed: "Shutdown complete".into(),
            // Every day is a Workday, so Sunday's ritual holds the Weekly Review.
            weekly_target: (now.weekday() == chrono::Weekday::Sun).then_some(20),
        },
    ];
    for step in steps {
        assert_eq!(a.send(&step).unwrap(), CommandOutcome::Accepted, "{step:?}");
    }
    assert_eq!(ritual(&a), RitualScreen::Completed);
    wait_until("B drops the ritual", || {
        ritual(&b) == RitualScreen::Completed
    });
    assert_eq!(b.state().unwrap().loose_ends.len(), 1);
    sync.stop();
}
