//! Sync between Devices against a local Supabase (`backend/scripts/test.sh`): a Block started on
//! one Device is enforced on another without user action, and an offline Device keeps
//! enforcing what it knows and catches up when it reconnects.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use cloister_core::{Command, EntryKind};
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
    let backend = common::backend();
    let email = common::unique_email(prefix);
    let a = device(&backend, &dir.join("a.sqlite"), "Desktop A");
    a.sign_in(common::sign_in(&backend, &email)).unwrap();
    std::thread::sleep(Duration::from_millis(1100)); // one sign-in email per second
    let b = device(&backend, &dir.join("b.sqlite"), "Desktop B");
    b.sign_in(common::sign_in(&backend, &email)).unwrap();
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
