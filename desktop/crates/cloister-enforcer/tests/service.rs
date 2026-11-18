//! The enforcement service through its IPC socket, with a recording platform and a fake
//! clock, so it runs unprivileged. The real platform is covered by the root smoke test.

use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};
use cloister_core::{
    BlocklistEdits, BlocklistRemoval, Command, DayPlan, Enforcement, EntryKind, Lockdown, Ritual,
    RitualScreen, Settings, ShutdownDelay, Slot,
};
use cloister_enforcer::dns::SharedDomains;
use cloister_enforcer::ipc::{self, Planning, Request, Response};
use cloister_enforcer::service::shared_domains;
use cloister_enforcer::store::StateStore;
use cloister_enforcer::{Clock, Platform, Service};

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<Enforcement>>>);

impl Recorder {
    fn last(&self) -> Enforcement {
        self.0
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("applied at least once")
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

impl FakeClock {
    fn at(s: &str) -> Self {
        FakeClock(Arc::new(Mutex::new(s.parse().unwrap())))
    }
    fn advance(&self, d: Duration) {
        *self.0.lock().unwrap() += d;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

struct Running {
    socket: PathBuf,
    service: Arc<Mutex<Service<Recorder, FakeClock>>>,
    platform: Recorder,
    domains: SharedDomains,
}

/// Unix socket paths must be short, so they live in /tmp rather than $TMPDIR.
fn socket_path(dir: &tempfile::TempDir) -> PathBuf {
    let name = dir
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    PathBuf::from(format!("/tmp/cl-{name}.sock"))
}

fn start(dir: &tempfile::TempDir, clock: &FakeClock) -> Running {
    let platform = Recorder::default();
    let domains = shared_domains();
    let store = StateStore::new(dir.path().join("state.json"));
    let service = Service::open(store, platform.clone(), clock.clone(), domains.clone()).unwrap();
    let service = Arc::new(Mutex::new(service));
    let socket = socket_path(dir);
    ipc::serve(&socket, service.clone()).unwrap();
    Running {
        socket,
        service,
        platform,
        domains,
    }
}

fn send(r: &Running, command: Command) -> Response {
    ipc::send(&r.socket, &Request::Command { command }).unwrap()
}

fn start_block(minutes: i64) -> Command {
    Command::StartBlock {
        block_id: "b1".into(),
        minutes,
        deep: true,
        blocklists: vec![],
        confirm_long: false,
        client_time: None,
        intent: Some("Write the report".into()),
        extra_domains: vec![],
        extra_apps: vec![],
    }
}

fn blocks_youtube(r: &Running) -> bool {
    r.domains.read().unwrap().is_blocked("m.youtube.com")
        && r.platform
            .last()
            .blocked_domains
            .contains(&"youtube.com".to_string())
}

#[test]
fn a_block_started_over_ipc_is_enforced_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::at("2026-01-05T09:00:00Z");
    let r = start(&dir, &clock);
    assert!(!blocks_youtube(&r));

    let Response::Accepted { enforcement } = send(&r, start_block(60)) else {
        panic!()
    };
    assert!(enforcement
        .blocked_domains
        .contains(&"youtu.be".to_string()));
    assert!(blocks_youtube(&r));

    let status = ipc::send(&r.socket, &Request::Status).unwrap();
    let Response::Status { blocks, .. } = status else {
        panic!()
    };
    assert_eq!(blocks.len(), 1);
}

#[test]
fn the_block_survives_restarting_the_service() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::at("2026-01-05T09:00:00Z");
    let first = start(&dir, &clock);
    send(&first, start_block(60));
    drop(first);

    clock.advance(Duration::minutes(30));
    let second = start(&dir, &clock);
    assert!(
        blocks_youtube(&second),
        "enforced straight after restart, before any IPC"
    );
}

#[test]
fn nothing_over_ipc_can_end_or_shorten_a_block() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::at("2026-01-05T09:00:00Z");
    let r = start(&dir, &clock);
    send(&r, start_block(60));

    let cancel = send(
        &r,
        Command::CancelBlock {
            block_id: "b1".into(),
        },
    );
    assert_eq!(
        cancel,
        Response::Rejected {
            reason: "loosening_not_accepted".into()
        }
    );

    let shorten = send(
        &r,
        Command::ExtendBlock {
            block_id: "b1".into(),
            ends_at: "2026-01-05T09:10:00Z".parse().unwrap(),
            confirm_long: false,
        },
    );
    assert_eq!(
        shorten,
        Response::Rejected {
            reason: "block_cannot_be_shortened".into()
        }
    );

    assert!(blocks_youtube(&r));
}

#[test]
fn malformed_requests_are_rejected() {
    use std::io::{BufRead, BufReader, Write};
    let dir = tempfile::tempdir().unwrap();
    let r = start(&dir, &FakeClock::at("2026-01-05T09:00:00Z"));
    let mut stream = std::os::unix::net::UnixStream::connect(&r.socket).unwrap();
    stream
        .write_all(b"{\"request\":\"command\",\"command\":{\"type\":\"set_state\"}}\n")
        .unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    let Response::Rejected { reason } = serde_json::from_str(&line).unwrap() else {
        panic!("{line}")
    };
    assert!(reason.starts_with("bad_request"), "{reason}");
}

#[test]
fn every_tick_reapplies_and_the_block_ends_on_time() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::at("2026-01-05T09:00:00Z");
    let r = start(&dir, &clock);
    send(&r, start_block(60));
    let applied_before = r.platform.0.lock().unwrap().len();

    clock.advance(Duration::minutes(59));
    r.service.lock().unwrap().tick();
    assert!(blocks_youtube(&r));
    assert!(r.platform.0.lock().unwrap().len() > applied_before);

    clock.advance(Duration::minutes(1));
    r.service.lock().unwrap().tick();
    assert!(r.platform.last().blocked_domains.is_empty());
    assert!(!r.domains.read().unwrap().is_blocked("youtube.com"));
}

#[test]
fn a_depth_rating_is_recorded_over_ipc_after_the_deep_session_ends() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::at("2026-01-05T09:00:00Z");
    let r = start(&dir, &clock);
    send(&r, start_block(60));
    clock.advance(Duration::minutes(60));

    let rate = send(
        &r,
        Command::RecordDepthRating {
            block_id: "b1".into(),
            rating: 4,
        },
    );
    assert!(matches!(rate, Response::Accepted { .. }), "{rate:?}");

    let Response::Status { blocks, .. } = ipc::send(&r.socket, &Request::Status).unwrap() else {
        panic!()
    };
    assert_eq!(blocks[0].depth_rating, Some(4));
    assert_eq!(blocks[0].intent.as_deref(), Some("Write the report"));
}

fn synced_block(id: &str, starts: &str, ends: &str) -> cloister_core::Block {
    cloister_core::Block {
        id: id.into(),
        starts_at: starts.parse().unwrap(),
        ends_at: ends.parse().unwrap(),
        blocklists: vec![
            "forums".into(),
            "news".into(),
            "shopping".into(),
            "social".into(),
            "video".into(),
        ],
        deep: true,
        intent: None,
        depth_rating: None,
        extra_domains: vec![],
        extra_apps: vec![],
        quit_social: false,
    }
}

fn sync(r: &Running, blocks: Vec<cloister_core::Block>) -> Response {
    ipc::send(
        &r.socket,
        &Request::Sync {
            blocks,
            blocklists: None,
            planning: None,
        },
    )
    .unwrap()
}

#[test]
fn blocks_synced_from_another_device_are_enforced_and_kept() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::at("2026-01-05T09:00:00Z");
    let r = start(&dir, &clock);
    let answer = sync(
        &r,
        vec![synced_block(
            "remote",
            "2026-01-05T08:50:00Z",
            "2026-01-05T10:00:00Z",
        )],
    );
    assert!(matches!(answer, Response::Accepted { .. }), "{answer:?}");
    assert!(blocks_youtube(&r));
    drop(r);
    let again = start(&dir, &clock);
    assert!(blocks_youtube(&again), "kept across a restart");
}

#[test]
fn sync_can_only_tighten() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::at("2026-01-05T09:00:00Z");
    let r = start(&dir, &clock);
    send(&r, start_block(60));
    // A shorter copy of b1 and nothing else: b1 keeps its end.
    sync(
        &r,
        vec![synced_block(
            "b1",
            "2026-01-05T09:00:00Z",
            "2026-01-05T09:05:00Z",
        )],
    );
    clock.advance(Duration::minutes(30));
    r.service.lock().unwrap().tick();
    assert!(blocks_youtube(&r));
    // An impossible Block (longer than 24 hours) is refused.
    let answer = sync(
        &r,
        vec![synced_block(
            "huge",
            "2026-01-05T09:00:00Z",
            "2026-02-05T09:00:00Z",
        )],
    );
    assert_eq!(
        answer,
        Response::Rejected {
            reason: "invalid_block".into()
        }
    );
}

fn reddit_removal(effective_at: &str) -> BlocklistRemoval {
    BlocklistRemoval {
        blocklist: "forums".into(),
        kind: EntryKind::Domain,
        entry: "reddit.com".into(),
        effective_at: effective_at.parse().unwrap(),
    }
}

fn blocks(r: &Running, domain: &str) -> bool {
    r.domains.read().unwrap().is_blocked(domain)
}

#[test]
fn blocklist_additions_apply_at_once_over_ipc_but_loosening_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::at("2026-01-05T09:00:00Z");
    let r = start(&dir, &clock);
    send(&r, start_block(60));
    let add = send(
        &r,
        Command::AddBlocklistEntry {
            blocklist: "social".into(),
            kind: EntryKind::App,
            entry: "Discord".into(),
        },
    );
    let Response::Accepted { enforcement } = add else {
        panic!("{add:?}")
    };
    assert!(enforcement.blocked_apps.contains(&"Discord".to_string()));
    for loosening in [
        Command::RemoveBlocklistEntry {
            blocklist: "forums".into(),
            kind: EntryKind::Domain,
            entry: "reddit.com".into(),
        },
        Command::OptOutOfBlocklist {
            blocklist: "news".into(),
        },
    ] {
        assert_eq!(
            send(&r, loosening),
            Response::Rejected {
                reason: "loosening_not_accepted".into()
            }
        );
    }
}

#[test]
fn a_synced_removal_never_takes_effect_before_tomorrow_here() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::at("2026-01-05T23:00:00Z");
    let r = start(&dir, &clock);
    send(&r, start_block(120));
    // A tampered sync claims the removal took effect an hour ago.
    let answer = ipc::send(
        &r.socket,
        &Request::Sync {
            blocks: vec![],
            blocklists: Some(BlocklistEdits {
                removals: vec![reddit_removal("2026-01-05T22:00:00Z")],
                ..BlocklistEdits::default()
            }),
            planning: None,
        },
    )
    .unwrap();
    assert!(matches!(answer, Response::Accepted { .. }), "{answer:?}");
    assert!(blocks(&r, "reddit.com"), "still blocked until midnight");
    clock.advance(Duration::minutes(60));
    r.service.lock().unwrap().tick();
    assert!(!blocks(&r, "reddit.com"), "removed from the next day");
    assert!(blocks(&r, "youtube.com"));
}

#[test]
fn a_synced_quit_social_block_is_kept_for_its_30_days() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::at("2026-01-05T09:00:00Z");
    let r = start(&dir, &clock);
    let mut q = synced_block("q1", "2026-01-05T09:00:00Z", "2026-02-04T09:00:00Z");
    q.blocklists = vec!["social".into()];
    q.deep = false;
    q.quit_social = true;
    let answer = sync(&r, vec![q]);
    assert!(matches!(answer, Response::Accepted { .. }), "{answer:?}");
    clock.advance(Duration::days(29));
    r.service.lock().unwrap().tick();
    assert!(blocks(&r, "x.com"));
    assert!(!blocks(&r, "youtube.com"));
}

// Planning: settings, Day Plans and Schedules held by the service itself -------------------

fn weekdays_9_to_5() -> Settings {
    Settings {
        workdays: vec![1, 2, 3, 4, 5],
        workday_start: "09:00".into(),
        shutdown_time: "17:00".into(),
    }
}

fn sync_planning(r: &Running, planning: Planning) -> Response {
    ipc::send(
        &r.socket,
        &Request::Sync {
            blocks: vec![],
            blocklists: None,
            planning: Some(planning),
        },
    )
    .unwrap()
}

fn utc(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

fn plan(date: &str, slots: Vec<Slot>) -> DayPlan {
    let minutes = slots
        .iter()
        .filter(|s| s.deep)
        .map(|s| (s.ends_at - s.starts_at).num_minutes())
        .sum();
    DayPlan {
        date: date.parse().unwrap(),
        starts_at: utc(&format!("{date}T09:00:00Z")),
        ends_at: utc(&format!("{date}T17:00:00Z")),
        planned_deep_minutes: minutes,
        slots,
    }
}

fn deep_slot(id: &str, starts: &str, ends: &str) -> Slot {
    Slot {
        id: id.into(),
        starts_at: utc(starts),
        ends_at: utc(ends),
        label: "Draft chapter 3".into(),
        deep: true,
        depth_rating: None,
    }
}

#[test]
fn evening_lockdown_is_enforced_on_the_clock_by_the_service_alone() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::at("2026-01-05T16:00:00Z");
    let r = start(&dir, &clock);
    let answer = sync_planning(
        &r,
        Planning {
            settings: Some(weekdays_9_to_5()),
            ..Planning::default()
        },
    );
    assert!(matches!(answer, Response::Accepted { .. }), "{answer:?}");
    assert!(!blocks(&r, "github.com"));
    drop(r);

    // Restarted, with nobody syncing any more: the clock alone starts Evening Lockdown.
    clock.advance(Duration::hours(1));
    let r = start(&dir, &clock);
    assert!(blocks(&r, "github.com") && blocks(&r, "youtube.com"));
    let Response::Status {
        lockdown,
        next_workday_start,
        ..
    } = ipc::send(&r.socket, &Request::Status).unwrap()
    else {
        panic!()
    };
    assert_eq!(lockdown, Some(Lockdown::EveningLockdown));
    assert_eq!(next_workday_start, Some(utc("2026-01-06T09:00:00Z")));
    clock.advance(Duration::hours(16));
    r.service.lock().unwrap().tick();
    assert!(!blocks(&r, "github.com"), "Workday Start ends it");
}

#[test]
fn a_planned_deep_session_starts_on_time_with_only_the_service_running() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::at("2026-01-05T08:00:00Z");
    let r = start(&dir, &clock);
    sync_planning(
        &r,
        Planning {
            day_plans: vec![plan(
                "2026-01-05",
                vec![deep_slot(
                    "s1",
                    "2026-01-05T10:00:00Z",
                    "2026-01-05T11:00:00Z",
                )],
            )],
            ..Planning::default()
        },
    );
    assert!(!blocks(&r, "youtube.com"));
    clock.advance(Duration::hours(2));
    r.service.lock().unwrap().tick();
    assert!(blocks(&r, "youtube.com"));
    clock.advance(Duration::hours(1));
    r.service.lock().unwrap().tick();
    assert!(!blocks(&r, "youtube.com"));
}

#[test]
fn a_synced_loosening_of_settings_waits_until_tomorrow() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::at("2026-01-05T17:30:00Z");
    let r = start(&dir, &clock);
    sync_planning(
        &r,
        Planning {
            settings: Some(weekdays_9_to_5()),
            ..Planning::default()
        },
    );
    assert!(blocks(&r, "github.com"));
    // A tampered sync claims Shutdown Time is 22:00 with nothing pending.
    let mut later = weekdays_9_to_5();
    later.shutdown_time = "22:00".into();
    sync_planning(
        &r,
        Planning {
            settings: Some(later.clone()),
            ..Planning::default()
        },
    );
    assert!(blocks(&r, "github.com"), "tonight's lockdown stands");
    // Tomorrow the later Shutdown Time applies.
    clock.advance(Duration::hours(24) + Duration::hours(2));
    r.service.lock().unwrap().tick();
    assert!(!blocks(&r, "github.com"));
    // Tightening applies at once.
    let mut earlier = later;
    earlier.shutdown_time = "18:00".into();
    sync_planning(
        &r,
        Planning {
            settings: Some(earlier),
            ..Planning::default()
        },
    );
    assert!(blocks(&r, "github.com"));
}

#[test]
fn a_synced_day_plan_can_move_deep_slots_but_not_drop_or_backdate_them() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::at("2026-01-05T10:30:00Z");
    let r = start(&dir, &clock);
    let running = deep_slot("s1", "2026-01-05T10:00:00Z", "2026-01-05T11:00:00Z");
    let later = deep_slot("s2", "2026-01-05T14:00:00Z", "2026-01-05T15:00:00Z");
    sync_planning(
        &r,
        Planning {
            day_plans: vec![plan("2026-01-05", vec![running.clone(), later.clone()])],
            ..Planning::default()
        },
    );
    assert!(blocks(&r, "youtube.com"));
    // Dropping the running Deep Session is refused; the old plan stays.
    sync_planning(
        &r,
        Planning {
            day_plans: vec![plan(
                "2026-01-05",
                vec![
                    later.clone(),
                    deep_slot("s3", "2026-01-05T15:00:00Z", "2026-01-05T16:00:00Z"),
                ],
            )],
            ..Planning::default()
        },
    );
    assert!(blocks(&r, "youtube.com"));
    // Moving the later Slot into the past would free the afternoon: refused.
    sync_planning(
        &r,
        Planning {
            day_plans: vec![plan(
                "2026-01-05",
                vec![
                    deep_slot("s2", "2026-01-05T08:00:00Z", "2026-01-05T09:00:00Z"),
                    running.clone(),
                ],
            )],
            ..Planning::default()
        },
    );
    clock.advance(Duration::hours(4));
    r.service.lock().unwrap().tick();
    assert!(blocks(&r, "youtube.com"), "14:00 Deep Session kept");
    // Moving it to another future time is fine.
    sync_planning(
        &r,
        Planning {
            day_plans: vec![plan(
                "2026-01-05",
                vec![
                    running,
                    later,
                    deep_slot("s4", "2026-01-05T16:00:00Z", "2026-01-05T17:00:00Z"),
                ],
            )],
            ..Planning::default()
        },
    );
    clock.advance(Duration::hours(2));
    r.service.lock().unwrap().tick();
    assert!(blocks(&r, "youtube.com"), "16:00 added");
}

#[test]
fn a_deep_session_cannot_start_in_evening_lockdown_over_ipc() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::at("2026-01-05T18:00:00Z");
    let r = start(&dir, &clock);
    sync_planning(
        &r,
        Planning {
            settings: Some(weekdays_9_to_5()),
            ..Planning::default()
        },
    );
    assert_eq!(
        send(&r, start_block(60)),
        Response::Rejected {
            reason: "evening_lockdown".into()
        }
    );
}

// The Shutdown Ritual: due on the service's own clock; lifted only by synced records ------------

fn ritual_flag(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("ritual-due")
}

fn start_with_flag(dir: &tempfile::TempDir, clock: &FakeClock) -> Running {
    let r = start(dir, clock);
    r.service
        .lock()
        .unwrap()
        .set_ritual_flag(Some(ritual_flag(dir)));
    r.service.lock().unwrap().tick();
    r
}

fn ritual_now(r: &Running) -> RitualScreen {
    r.platform.last().ritual
}

fn tomorrow_planned() -> DayPlan {
    plan(
        "2026-01-06",
        vec![deep_slot(
            "t1",
            "2026-01-06T09:00:00Z",
            "2026-01-06T17:00:00Z",
        )],
    )
}

#[test]
fn the_ritual_falls_due_at_shutdown_time_and_raises_the_ritual_flag() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::at("2026-01-05T16:59:00Z");
    let r = start_with_flag(&dir, &clock);
    sync_planning(
        &r,
        Planning {
            settings: Some(weekdays_9_to_5()),
            ..Planning::default()
        },
    );
    assert!(!ritual_flag(&dir).exists());
    drop(r);

    // Restarted at Shutdown Time with nothing syncing: the service's own state makes it due.
    clock.advance(Duration::minutes(1));
    let r = start_with_flag(&dir, &clock);
    assert_eq!(ritual_now(&r), RitualScreen::Due);
    assert!(ritual_flag(&dir).exists(), "the ritual window is kept up");
    let Response::Status { ritual_due_at, .. } = ipc::send(&r.socket, &Request::Status).unwrap()
    else {
        panic!()
    };
    assert_eq!(ritual_due_at, Some(utc("2026-01-05T17:00:00Z")));
}

#[test]
fn a_completed_ritual_arrives_by_sync_and_lowers_the_flag_only_if_tomorrow_is_planned() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::at("2026-01-05T17:20:00Z");
    let r = start_with_flag(&dir, &clock);
    let done = Ritual {
        date: "2026-01-05".parse().unwrap(),
        completed_at: utc("2026-01-05T17:15:00Z"),
    };
    sync_planning(
        &r,
        Planning {
            settings: Some(weekdays_9_to_5()),
            rituals: vec![done.clone()],
            ..Planning::default()
        },
    );
    assert_eq!(
        ritual_now(&r),
        RitualScreen::Due,
        "no plan for tomorrow here"
    );
    assert!(ritual_flag(&dir).exists());
    sync_planning(
        &r,
        Planning {
            settings: Some(weekdays_9_to_5()),
            day_plans: vec![tomorrow_planned()],
            rituals: vec![done],
            ..Planning::default()
        },
    );
    assert_eq!(ritual_now(&r), RitualScreen::Completed);
    assert!(!ritual_flag(&dir).exists());
}

#[test]
fn a_synced_shutdown_delay_holds_lockdown_and_the_ritual_for_30_minutes() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::at("2026-01-05T16:30:00Z");
    let r = start_with_flag(&dir, &clock);
    sync_planning(
        &r,
        Planning {
            settings: Some(weekdays_9_to_5()),
            shutdown_delays: vec![ShutdownDelay {
                date: "2026-01-05".parse().unwrap(),
                used_at: utc("2026-01-05T16:20:00Z"),
            }],
            ..Planning::default()
        },
    );
    clock.advance(Duration::minutes(45));
    r.service.lock().unwrap().tick();
    assert_eq!(ritual_now(&r), RitualScreen::Delayed);
    assert!(!blocks(&r, "github.com"));
    assert!(!ritual_flag(&dir).exists());
    clock.advance(Duration::minutes(15));
    r.service.lock().unwrap().tick();
    assert_eq!(ritual_now(&r), RitualScreen::Due);
    assert!(blocks(&r, "github.com"));
    assert!(ritual_flag(&dir).exists());
}

#[test]
fn nothing_over_ipc_can_complete_or_delay_the_ritual() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::at("2026-01-05T17:05:00Z");
    let r = start(&dir, &clock);
    for command in [
        Command::UseShutdownDelay {},
        Command::CompleteRitual {
            typed: "Shutdown complete".into(),
            weekly_target: None,
        },
        Command::CloseLooseEnd {
            loose_end_id: "l1".into(),
        },
    ] {
        assert_eq!(
            send(&r, command),
            Response::Rejected {
                reason: "loosening_not_accepted".into()
            }
        );
    }
}
