//! The enforcement service through its IPC socket, with a recording platform and a fake
//! clock, so it runs unprivileged. The real platform is covered by the root smoke test.

use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};
use cloister_core::{Command, Enforcement};
use cloister_enforcer::dns::SharedDomains;
use cloister_enforcer::ipc::{self, Request, Response};
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

fn synced_block(id: &str, starts: &str, ends: &str) -> cloister_core::Block {
    cloister_core::Block {
        id: id.into(),
        starts_at: starts.parse().unwrap(),
        ends_at: ends.parse().unwrap(),
        blocklists: vec!["forums".into(), "news".into(), "shopping".into(), "social".into(), "video".into()],
        deep: true,
    }
}

fn sync(r: &Running, blocks: Vec<cloister_core::Block>) -> Response {
    ipc::send(&r.socket, &Request::Sync { blocks }).unwrap()
}

#[test]
fn blocks_synced_from_another_device_are_enforced_and_kept() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::at("2026-01-05T09:00:00Z");
    let r = start(&dir, &clock);
    let answer = sync(&r, vec![synced_block("remote", "2026-01-05T08:50:00Z", "2026-01-05T10:00:00Z")]);
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
    sync(&r, vec![synced_block("b1", "2026-01-05T09:00:00Z", "2026-01-05T09:05:00Z")]);
    clock.advance(Duration::minutes(30));
    r.service.lock().unwrap().tick();
    assert!(blocks_youtube(&r));
    // An impossible Block (longer than 24 hours) is refused.
    let answer = sync(&r, vec![synced_block("huge", "2026-01-05T09:00:00Z", "2026-02-05T09:00:00Z")]);
    assert_eq!(answer, Response::Rejected { reason: "invalid_block".into() });
}
