//! A signed-in Device: the API the desktop app (and the Android port) builds on.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use chrono::{DateTime, NaiveDate, Utc};
use cloister_core::{
    enforcement, planned_deep_sessions, Block, Command, DayPlan, Enforcement, Schedule, State,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::realtime::{Channel, Event};
use crate::{Backend, Error, Mirror, Result, Session};

/// How long a Device may go without reporting before it counts as Offline.
pub const STALE_AFTER: chrono::Duration = chrono::Duration::seconds(90);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Macos,
    Windows,
    Linux,
    Android,
}

impl Platform {
    pub fn current() -> Self {
        match std::env::consts::OS {
            "macos" => Platform::Macos,
            "windows" => Platform::Windows,
            "android" => Platform::Android,
            _ => Platform::Linux,
        }
    }
}

/// How this Device appears on the Devices screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub name: String,
    pub platform: Platform,
}

/// Whether this Device is in touch with the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStatus {
    Online,
    Offline,
}

impl SyncStatus {
    pub fn label(&self) -> &'static str {
        match self {
            SyncStatus::Online => "Online",
            SyncStatus::Offline => "Offline, still enforcing",
        }
    }
}

/// A Device's status on the Devices screen (a word beside a square).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceStatus {
    Enforcing,
    Offline,
}

impl DeviceStatus {
    /// Enforcing while the Device says so and was seen within [`STALE_AFTER`].
    pub fn classify(
        enforcing: bool,
        last_seen_at: DateTime<Utc>,
        server_now: DateTime<Utc>,
    ) -> Self {
        if enforcing && server_now - last_seen_at <= STALE_AFTER {
            DeviceStatus::Enforcing
        } else {
            DeviceStatus::Offline
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            DeviceStatus::Enforcing => "Enforcing",
            DeviceStatus::Offline => "Offline",
        }
    }
}

/// One row of the Devices screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceSummary {
    pub id: String,
    pub name: String,
    pub platform: Platform,
    pub status: DeviceStatus,
    pub last_seen_at: DateTime<Utc>,
    pub this_device: bool,
}

/// The backend's answer to a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutcome {
    Accepted,
    /// `reason` is the stable code (see fixtures/README.md); map it to calm copy.
    Rejected {
        reason: String,
    },
}

/// Something the UI may want to redraw for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncEvent {
    /// The mirror (and so the Enforcement) changed.
    StateChanged,
    SyncStatusChanged(SyncStatus),
}

#[derive(Debug, Clone)]
pub struct SyncOptions {
    /// Re-read Blocks this often even without a Realtime message.
    pub poll_every: Duration,
    /// Report this Device's status this often (keep well under [`STALE_AFTER`]).
    pub report_every: Duration,
    /// Longest wait between reconnection attempts.
    pub max_backoff: Duration,
}

impl Default for SyncOptions {
    fn default() -> Self {
        SyncOptions {
            poll_every: Duration::from_secs(60),
            report_every: Duration::from_secs(30),
            max_backoff: Duration::from_secs(30),
        }
    }
}

const SESSION_KEY: &str = "session";
const OFFSET_KEY: &str = "server_offset_ms";

/// A Device. Cheap to clone; all clones share the mirror and sync state.
#[derive(Clone)]
pub struct Device {
    inner: Arc<Inner>,
}

struct Inner {
    backend: Backend,
    info: DeviceInfo,
    device_id: String,
    mirror: Mutex<Mirror>,
    session: Mutex<Option<Session>>,
    status: Mutex<SyncStatus>,
    enforcing: AtomicBool,
    subscribers: Mutex<Vec<Sender<SyncEvent>>>,
}

impl Device {
    /// Open this Device's mirror. Works offline; a saved session is picked up.
    pub fn open(backend: Backend, mirror: impl AsRef<Path>, info: DeviceInfo) -> Result<Self> {
        let mirror = Mirror::open(mirror)?;
        let device_id = mirror.device_id()?;
        let session = match mirror.get(SESSION_KEY)? {
            Some(s) => serde_json::from_str(&s).ok(),
            None => None,
        };
        Ok(Device {
            inner: Arc::new(Inner {
                backend,
                info,
                device_id,
                mirror: Mutex::new(mirror),
                session: Mutex::new(session),
                status: Mutex::new(SyncStatus::Offline),
                enforcing: AtomicBool::new(false),
                subscribers: Mutex::new(Vec::new()),
            }),
        })
    }

    pub fn id(&self) -> &str {
        &self.inner.device_id
    }

    pub fn is_signed_in(&self) -> bool {
        lock(&self.inner.session).is_some()
    }

    /// Keep `session` (from [`Backend::exchange_code`] or [`Backend::verify_email_code`]),
    /// register this Device with the account and fetch its Blocks.
    pub fn sign_in(&self, session: Session) -> Result<()> {
        self.store_session(&session)?;
        self.register()?;
        self.pull().map(|_| ())
    }

    /// Forget the session. The mirror keeps enforcing what it knows.
    pub fn sign_out(&self) -> Result<()> {
        *lock(&self.inner.session) = None;
        lock(&self.inner.mirror).remove(SESSION_KEY)
    }

    /// Send a command to the backend, which decides at server time. The mirror is up to
    /// date with the backend when this returns.
    pub fn send(&self, command: &Command) -> Result<CommandOutcome> {
        let session = self.session()?;
        let answer = self.online(self.inner.backend.rest_post(
            &session,
            "rpc/command",
            json!({ "cmd": command }),
            None,
        ))?;
        let outcome = match (answer["outcome"].as_str(), answer["reason"].as_str()) {
            (Some("accepted"), _) => CommandOutcome::Accepted,
            (Some("rejected"), Some(reason)) => CommandOutcome::Rejected {
                reason: reason.to_string(),
            },
            _ => return Err(Error::Protocol(answer.to_string())),
        };
        // Either way the backend knows best: a rejection often means this mirror is behind.
        self.pull()?;
        Ok(outcome)
    }

    /// Re-read the account's Blocks into the mirror. Returns whether anything changed.
    pub fn pull(&self) -> Result<bool> {
        let session = self.session()?;
        let backend = &self.inner.backend;
        let rows = self.online(backend.rest_get(
            &session,
            "blocks?select=id,starts_at,ends_at,blocklists,deep,intent,depth_rating&order=starts_at,id",
        ))?;
        let blocks: Vec<Block> =
            serde_json::from_value(rows).map_err(|e| Error::Protocol(e.to_string()))?;
        // Day Plans and Schedules in the engine's form (absent keys mean none).
        let state = self.online(backend.rest_post(&session, "rpc/state", json!({}), None))?;
        let plans = |key: &str| state.get(key).cloned().unwrap_or_else(|| json!([]));
        let day_plans: Vec<DayPlan> = serde_json::from_value(plans("day_plans"))
            .map_err(|e| Error::Protocol(e.to_string()))?;
        let schedules: Vec<Schedule> = serde_json::from_value(plans("schedules"))
            .map_err(|e| Error::Protocol(e.to_string()))?;
        let server_now = self.fetch_server_now(&session)?;
        let changed = {
            let mut mirror = lock(&self.inner.mirror);
            let offset = (server_now - Utc::now()).num_milliseconds();
            mirror.set(OFFSET_KEY, &offset.to_string())?;
            let merged = mirror.merge_blocks(&blocks)?;
            mirror.replace_day_plans(&day_plans, &schedules)? || merged
        };
        if changed {
            self.emit(SyncEvent::StateChanged);
        }
        Ok(changed)
    }

    /// The state this Device enforces (from the mirror; works offline).
    pub fn state(&self) -> Result<State> {
        lock(&self.inner.mirror).state()
    }

    /// The Day Plan for `date` (from the mirror; works offline): the saved one, or else the
    /// draft its Schedule fills in.
    pub fn day_plan(&self, date: NaiveDate) -> Result<Option<DayPlan>> {
        Ok(cloister_core::day_plan(&self.state()?, date))
    }

    /// Deep Sessions that deep Slots have become by now (by the best estimate of server
    /// time), as Blocks. Handed to the enforcement service, which only ever tightens: a
    /// started Deep Session can't change, so it is safe to hand over once it has begun.
    pub fn started_planned_sessions(&self) -> Result<Vec<Block>> {
        let now = self.server_time();
        Ok(planned_deep_sessions(&self.state()?, now)
            .into_iter()
            .filter(|b| b.starts_at <= now)
            .collect())
    }

    /// The Enforcement for now, by the best estimate of server time.
    pub fn enforcement(&self) -> Result<Enforcement> {
        Ok(enforcement(&self.state()?, self.server_time()))
    }

    /// The local clock corrected by the offset seen at the last sync.
    pub fn server_time(&self) -> DateTime<Utc> {
        let offset = lock(&self.inner.mirror)
            .get(OFFSET_KEY)
            .ok()
            .flatten()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        Utc::now() + chrono::Duration::milliseconds(offset)
    }

    pub fn sync_status(&self) -> SyncStatus {
        *lock(&self.inner.status)
    }

    /// Tell the account whether this Device is enforcing (the enforcement service is up).
    /// Repeated by the sync loop so the Devices screen sees it is alive.
    pub fn report_enforcing(&self, enforcing: bool) -> Result<()> {
        self.inner.enforcing.store(enforcing, Ordering::SeqCst);
        self.register()
    }

    /// Every Device on the account with its status, oldest first.
    pub fn devices(&self) -> Result<Vec<DeviceSummary>> {
        #[derive(Deserialize)]
        struct Row {
            id: String,
            name: String,
            platform: Platform,
            enforcing: bool,
            last_seen_at: DateTime<Utc>,
        }
        let session = self.session()?;
        let rows = self.online(self.inner.backend.rest_get(
            &session,
            "devices?select=id,name,platform,enforcing,last_seen_at&order=created_at,id",
        ))?;
        let rows: Vec<Row> =
            serde_json::from_value(rows).map_err(|e| Error::Protocol(e.to_string()))?;
        let now = self.fetch_server_now(&session)?;
        Ok(rows
            .into_iter()
            .map(|r| DeviceSummary {
                status: DeviceStatus::classify(r.enforcing, r.last_seen_at, now),
                this_device: r.id == self.inner.device_id,
                id: r.id,
                name: r.name,
                platform: r.platform,
                last_seen_at: r.last_seen_at,
            })
            .collect())
    }

    /// Updates for the UI. Each call returns a new receiver.
    pub fn events(&self) -> Receiver<SyncEvent> {
        let (tx, rx) = channel();
        lock(&self.inner.subscribers).push(tx);
        rx
    }

    /// Keep the mirror in step with the backend on a background thread: Realtime pushes,
    /// a periodic re-read, status reports and reconnection with backoff.
    pub fn start_sync(&self, options: SyncOptions) -> SyncHandle {
        let stop = Arc::new(AtomicBool::new(false));
        let device = self.clone();
        let flag = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("cloister-sync".into())
            .spawn(move || device.sync_loop(&options, &flag))
            .expect("spawn sync thread");
        SyncHandle {
            stop,
            thread: Some(thread),
        }
    }

    fn sync_loop(&self, options: &SyncOptions, stop: &AtomicBool) {
        let mut backoff = Duration::from_secs(1);
        while !stop.load(Ordering::SeqCst) {
            match self.sync_connected(options, stop) {
                Ok(()) => break,
                Err(e) => {
                    self.set_status(SyncStatus::Offline);
                    if matches!(e, Error::Auth(_)) {
                        eprintln!("cloister-sync: {e}");
                    }
                }
            }
            let until = Instant::now() + backoff;
            while Instant::now() < until && !stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(100));
            }
            backoff = (backoff * 2).min(options.max_backoff);
        }
        self.set_status(SyncStatus::Offline);
    }

    /// One connected stretch. Returns Ok when asked to stop, Err when the connection is lost.
    fn sync_connected(&self, options: &SyncOptions, stop: &AtomicBool) -> Result<()> {
        let mut session = self.session()?;
        let mut channel = Channel::join(&self.inner.backend, &session)?;
        // Changes made before the subscription was confirmed aren't pushed: read them now.
        self.pull()?;
        self.register()?;
        self.set_status(SyncStatus::Online);
        let (mut last_pull, mut last_report) = (Instant::now(), Instant::now());
        while !stop.load(Ordering::SeqCst) {
            if let Event::Changed = channel.next()? {
                self.pull()?;
                last_pull = Instant::now();
            }
            if last_pull.elapsed() >= options.poll_every {
                self.pull()?;
                last_pull = Instant::now();
            }
            if last_report.elapsed() >= options.report_every {
                self.register()?;
                last_report = Instant::now();
            }
            let current = self.session()?;
            if current.access_token != session.access_token {
                channel.update_token(&current)?;
                session = current;
            }
        }
        Ok(())
    }

    /// Upsert this Device's row: name, platform and whether it is enforcing.
    fn register(&self) -> Result<()> {
        let session = self.session()?;
        let body = json!({
            "id": self.inner.device_id,
            "name": self.inner.info.name,
            "platform": self.inner.info.platform,
            "enforcing": self.inner.enforcing.load(Ordering::SeqCst),
        });
        self.online(self.inner.backend.rest_post(
            &session,
            "devices?on_conflict=id",
            body,
            Some("resolution=merge-duplicates,return=minimal"),
        ))
        .map(|_| ())
    }

    fn fetch_server_now(&self, session: &Session) -> Result<DateTime<Utc>> {
        let v = self.online(self.inner.backend.rest_post(
            session,
            "rpc/server_now",
            json!({}),
            None,
        ))?;
        serde_json::from_value(v).map_err(|e| Error::Protocol(e.to_string()))
    }

    /// The current session, refreshed first when it is about to expire.
    fn session(&self) -> Result<Session> {
        let mut guard = lock(&self.inner.session);
        let session = guard
            .clone()
            .ok_or_else(|| Error::Auth("not signed in".into()))?;
        if !session.expires_soon(Utc::now()) {
            return Ok(session);
        }
        let fresh = self.online(self.inner.backend.refresh(&session))?;
        *guard = Some(fresh.clone());
        drop(guard);
        self.store_session(&fresh)?;
        Ok(fresh)
    }

    fn store_session(&self, session: &Session) -> Result<()> {
        *lock(&self.inner.session) = Some(session.clone());
        let text = serde_json::to_string(session).expect("session serializes");
        lock(&self.inner.mirror).set(SESSION_KEY, &text)
    }

    /// Note an unreachable backend in the sync status.
    fn online<T>(&self, result: Result<T>) -> Result<T> {
        if let Err(Error::Offline(_)) = &result {
            self.set_status(SyncStatus::Offline);
        }
        result
    }

    fn set_status(&self, status: SyncStatus) {
        let changed = {
            let mut s = lock(&self.inner.status);
            std::mem::replace(&mut *s, status) != status
        };
        if changed {
            self.emit(SyncEvent::SyncStatusChanged(status));
        }
    }

    fn emit(&self, event: SyncEvent) {
        lock(&self.inner.subscribers).retain(|tx| tx.send(event).is_ok());
    }
}

/// Stops the sync thread when asked or dropped.
pub struct SyncHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl SyncHandle {
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for SyncHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// A fresh id for a Block started on this Device.
pub fn new_block_id() -> String {
    crate::new_uuid()
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}
