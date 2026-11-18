//! The Blocklists screen, Begin extras, the Quit-Social Block and the Devices screen through
//! the app model: against a real enforcement service over IPC (as the app runs without an
//! account), and against the rules engine standing in for the account's backend.

use std::cell::RefCell;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};
use cloister_app_model::{
    devices_view, BeginForm, Blocklists, Length, NoticeKind, Rules, ServiceRules, SocketEnforcer,
    Today, Zone,
};
use cloister_core::{decide, Command, Enforcement, EntryKind, Outcome, State};
use cloister_enforcer::ipc;
use cloister_enforcer::service::shared_domains;
use cloister_enforcer::store::StateStore;
use cloister_enforcer::{Clock, Platform, Service};
use cloister_sync::{DeviceStatus, DeviceSummary, Platform as DevicePlatform, SyncStatus};

fn t(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<Enforcement>>>);

impl Recorder {
    fn last(&self) -> Enforcement {
        self.0.lock().unwrap().last().cloned().unwrap_or_default()
    }
}

impl Platform for Recorder {
    fn apply(&mut self, e: &Enforcement) -> io::Result<()> {
        self.0.lock().unwrap().push(e.clone());
        Ok(())
    }
}

#[derive(Clone)]
struct FakeClock(Arc<Mutex<DateTime<Utc>>>);

impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

/// A dry-run enforcement service on a socket.
struct Service_ {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    clock: FakeClock,
    platform: Recorder,
}

fn service(at: &str) -> Service_ {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock(Arc::new(Mutex::new(t(at))));
    let platform = Recorder::default();
    let service = Service::open(
        StateStore::new(dir.path().join("state.json")),
        platform.clone(),
        clock.clone(),
        shared_domains(),
    )
    .unwrap();
    let name = dir
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let socket = PathBuf::from(format!("/tmp/clb-{name}.sock"));
    ipc::serve(&socket, Arc::new(Mutex::new(service))).unwrap();
    Service_ {
        _dir: dir,
        socket,
        clock,
        platform,
    }
}

/// The rules engine as the account's backend: decides every command at a pinned time.
struct Engine {
    state: RefCell<State>,
    now: RefCell<DateTime<Utc>>,
}

impl Engine {
    fn at(now: &str) -> Self {
        Engine {
            state: RefCell::new(State::default()),
            now: RefCell::new(t(now)),
        }
    }
}

impl Rules for &Engine {
    fn state(&self) -> Result<(State, DateTime<Utc>), cloister_app_model::Notice> {
        Ok((self.state.borrow().clone(), *self.now.borrow()))
    }

    fn send(&self, command: &Command) -> Result<Result<(), String>, cloister_app_model::Notice> {
        let decision = decide(&self.state.borrow(), command, *self.now.borrow());
        match decision.outcome {
            Outcome::Accepted => {
                *self.state.borrow_mut() = decision.state;
                Ok(Ok(()))
            }
            Outcome::Rejected { reason } => Ok(Err(reason.code().to_string())),
        }
    }
}

fn entries(list: &cloister_app_model::ListView) -> Vec<(&str, Option<&str>)> {
    list.domains
        .iter()
        .map(|e| (e.entry.as_str(), e.badge.as_deref()))
        .collect()
}

#[test]
fn every_distraction_category_is_on_by_default_and_the_work_list_is_listed() {
    let engine = Engine::at("2026-01-05T09:00:00Z");
    let mut screen = Blocklists::new(&engine).in_zone(Zone::Utc);
    screen.refresh();
    let view = screen.view();
    let titles: Vec<_> = view
        .distraction
        .iter()
        .map(|l| (l.title.as_str(), l.on))
        .collect();
    assert_eq!(
        titles,
        [
            ("Social", true),
            ("News", true),
            ("Video", true),
            ("Shopping", true),
            ("Forums", true)
        ]
    );
    assert_eq!(view.work.title, "Work List");
    assert!(view.work.domains.iter().any(|e| e.entry == "github.com"));
    assert!(view.pending.is_empty());
}

#[test]
fn a_site_added_to_the_work_list_is_blocked_at_once_through_the_local_service() {
    let s = service("2026-01-05T09:00:00Z");
    let mut today = Today::new(SocketEnforcer::new(&s.socket)).in_zone(Zone::Utc);
    let mut form = BeginForm {
        intent: "Ship the release.".into(),
        length: Length::Preset(60),
        ..BeginForm::default()
    };
    assert!(form.add_extra(EntryKind::Domain, "https://www.Example.com/x"));
    assert!(!form.add_extra(EntryKind::Domain, "not a site"));
    assert!(form.add_extra(EntryKind::App, "Steam"));
    today.begin(&form, s.clock.now()).unwrap();
    let applied = s.platform.last();
    assert!(applied.blocked_domains.contains(&"example.com".to_string()));
    assert!(applied.blocked_apps.contains(&"Steam".to_string()));

    let mut screen =
        Blocklists::new(ServiceRules(SocketEnforcer::new(&s.socket))).in_zone(Zone::Utc);
    screen.refresh();
    let notice = screen
        .add("social", EntryKind::Domain, "https://www.Lemmy.World/c/all")
        .unwrap();
    assert_eq!(notice.kind, NoticeKind::Recorded);
    assert_eq!(notice.text, "lemmy.world is in Social from now.");
    assert!(s
        .platform
        .last()
        .blocked_domains
        .contains(&"lemmy.world".to_string()));
    let social = &screen.view().distraction[0];
    assert!(entries(social).contains(&("lemmy.world", None)));

    // Without an account, loosening isn't possible here at all.
    let refused = screen
        .remove("social", EntryKind::Domain, "lemmy.world")
        .unwrap_err();
    assert!(
        refused.text.starts_with("Removals go through your account"),
        "{}",
        refused.text
    );
    let invalid = screen
        .add("social", EntryKind::Domain, "not a site")
        .unwrap_err();
    assert_eq!(
        invalid.text,
        "Enter a domain, like example.com, or an app name."
    );
}

#[test]
fn a_removal_shows_a_badge_and_is_listed_as_pending_until_tomorrow() {
    let engine = Engine::at("2026-01-05T09:00:00Z");
    let mut screen = Blocklists::new(&engine).in_zone(Zone::Utc);
    screen.refresh();

    let notice = screen
        .remove("forums", EntryKind::Domain, "reddit.com")
        .unwrap();
    assert_eq!(notice.text, "Removals take effect tomorrow.");
    assert_eq!(notice.kind, NoticeKind::WhatIsSo);
    let notice = screen.opt_out("news").unwrap();
    assert_eq!(notice.text, "News is switched off from tomorrow.");

    let view = screen.view();
    let forums = view.distraction.iter().find(|l| l.id == "forums").unwrap();
    assert!(entries(forums).contains(&("reddit.com", Some("Takes effect tomorrow"))));
    let news = view.distraction.iter().find(|l| l.id == "news").unwrap();
    assert!(news.on, "still on today");
    assert_eq!(news.badge.as_deref(), Some("Off from tomorrow"));
    let pending: Vec<_> = view
        .pending
        .iter()
        .map(|p| (p.what.as_str(), p.when.as_str()))
        .collect();
    assert_eq!(
        pending,
        [
            ("News is switched off.", "Takes effect tomorrow at 00:00."),
            (
                "reddit.com leaves Forums.",
                "Takes effect tomorrow at 00:00."
            ),
        ]
    );

    // Tomorrow: gone from the list, nothing pending, News off.
    *engine.now.borrow_mut() += Duration::days(1);
    screen.refresh();
    let view = screen.view();
    let forums = view.distraction.iter().find(|l| l.id == "forums").unwrap();
    assert!(!forums.domains.iter().any(|e| e.entry == "reddit.com"));
    assert!(view.pending.is_empty());
    assert!(!view.distraction.iter().find(|l| l.id == "news").unwrap().on);

    // Opting back in applies at once.
    screen.opt_in("news").unwrap();
    assert!(
        screen
            .view()
            .distraction
            .iter()
            .find(|l| l.id == "news")
            .unwrap()
            .on
    );
}

#[test]
fn the_quit_social_block_is_a_tag_not_the_hero() {
    let s = service("2026-01-05T09:00:00Z");
    let mut screen =
        Blocklists::new(ServiceRules(SocketEnforcer::new(&s.socket))).in_zone(Zone::Utc);
    let notice = screen.start_quit_social().unwrap();
    assert_eq!(
        notice.text,
        "Quit-Social Block started. Ends on Wed 4 Feb at 09:00."
    );
    assert_eq!(
        screen.view().quit_social_ends,
        Some(t("2026-02-04T09:00:00Z"))
    );
    assert!(s
        .platform
        .last()
        .blocked_domains
        .contains(&"x.com".to_string()));
    assert!(!s
        .platform
        .last()
        .blocked_domains
        .contains(&"youtube.com".to_string()));

    let mut today = Today::new(SocketEnforcer::new(&s.socket)).in_zone(Zone::Utc);
    let now = s.clock.now();
    today.refresh(now);
    let view = today.view(now);
    assert!(view.active.is_none(), "Begin stays available");
    assert_eq!(view.quit_social_ends, Some(t("2026-02-04T09:00:00Z")));
}

#[test]
fn the_devices_screen_shows_each_device_and_this_ones_sync_state() {
    let now = t("2026-01-05T09:00:00Z");
    let devices = [
        DeviceSummary {
            id: "a".into(),
            name: "Studio Mac".into(),
            platform: DevicePlatform::Macos,
            status: DeviceStatus::Enforcing,
            last_seen_at: t("2026-01-05T08:59:40Z"),
            this_device: true,
        },
        DeviceSummary {
            id: "b".into(),
            name: "Pixel".into(),
            platform: DevicePlatform::Android,
            status: DeviceStatus::Offline,
            last_seen_at: t("2026-01-03T18:00:00Z"),
            this_device: false,
        },
    ];
    let view = devices_view(&devices, SyncStatus::Offline, now, &Zone::Utc);
    assert_eq!(view.sync, "Offline, still enforcing");
    assert!(!view.online);
    let rows: Vec<_> = view
        .rows
        .iter()
        .map(|r| {
            (
                r.name.as_str(),
                r.platform,
                r.status,
                r.this_device,
                r.seen.as_str(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("Studio Mac", "macOS", "Enforcing", true, "Seen at 08:59."),
            ("Pixel", "Android", "Offline", false, "Seen on Sat 3 Jan."),
        ]
    );
    assert_eq!(
        devices_view(&[], SyncStatus::Online, now, &Zone::Utc).sync,
        "Online"
    );
}
