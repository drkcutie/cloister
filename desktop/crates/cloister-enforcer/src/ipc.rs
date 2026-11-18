//! Local IPC: one JSON request per line over a Unix socket, one JSON response per line back.
//!
//! The socket is world-writable on purpose: the service only accepts commands and syncs that
//! tighten enforcement, so any local user may start a Block but nobody can end one.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

use chrono::{DateTime, Utc};
use cloister_core::{
    Block, BlocklistEdits, Command, DayPlan, Enforcement, Lockdown, PendingSettings, Ritual,
    Schedule, Settings, ShutdownDelay, State,
};
use serde::{Deserialize, Serialize};

use crate::service::{Clock, Platform, Service};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "snake_case")]
pub enum Request {
    /// A rules-engine command (same JSON form as in the shared fixtures).
    Command {
        command: Command,
    },
    /// Blocks known from the backend (a Device's mirror). Merged so that it only tightens:
    /// new Blocks are added, later ends taken, and nothing is shortened or dropped.
    Sync {
        blocks: Vec<Block>,
        /// The account's Blocklist edits, when the sender knows them. Merged tighten-only
        /// (see [`cloister_core::State::merge_blocklist_edits`]).
        #[serde(default)]
        blocklists: Option<BlocklistEdits>,
        /// The account's settings, Day Plans and Schedules, when the sender knows them, so the
        /// service enforces Evening Lockdown and planned Deep Sessions on its own. Merged so
        /// nothing loosens before tomorrow (see [`cloister_core::State::merge_settings`] and
        /// [`cloister_core::State::merge_day_plan`]).
        #[serde(default)]
        planning: Option<Planning>,
    },
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "response", rename_all = "snake_case")]
pub enum Response {
    Accepted {
        enforcement: Enforcement,
    },
    Rejected {
        reason: String,
    },
    Status {
        server_time: DateTime<Utc>,
        blocks: Vec<Block>,
        enforcement: Enforcement,
        /// The Blocklist edits the service enforces.
        #[serde(default)]
        blocklists: BlocklistEdits,
        /// Whether the clock locks work now (Evening Lockdown or a Rest Day).
        #[serde(default)]
        lockdown: Option<Lockdown>,
        /// When work is next allowed, if settings exist.
        #[serde(default)]
        next_workday_start: Option<DateTime<Utc>>,
        /// The settings in force now.
        #[serde(default)]
        settings: Option<Settings>,
        /// When the latest Shutdown Ritual became due, or today's will (Shutdown Time, later
        /// with a Shutdown Delay). The screen state itself is `enforcement.ritual`.
        #[serde(default)]
        ritual_due_at: Option<DateTime<Utc>>,
        /// Shutdown Delays left this ISO week (0 or 1).
        #[serde(default)]
        shutdown_delays_left: u8,
    },
}

/// The planning part of the account's state, as handed to the service.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Planning {
    /// IANA; taken only with the first settings.
    #[serde(default = "utc")]
    pub time_zone: String,
    #[serde(default)]
    pub settings: Option<Settings>,
    #[serde(default)]
    pub pending_settings: Option<PendingSettings>,
    #[serde(default)]
    pub day_plans: Vec<DayPlan>,
    #[serde(default)]
    pub schedules: Vec<Schedule>,
    /// Shutdown Delays used (from any Device). Merged only where the rules allow them.
    #[serde(default)]
    pub shutdown_delays: Vec<ShutdownDelay>,
    /// Completed Shutdown Rituals. Merged only where the rules allow them.
    #[serde(default)]
    pub rituals: Vec<Ritual>,
}

fn utc() -> String {
    cloister_core::DEFAULT_TIME_ZONE.to_string()
}

impl Planning {
    /// The planning part of `state`.
    pub fn of(state: &State) -> Self {
        Planning {
            time_zone: state.time_zone.clone(),
            settings: state.settings.clone(),
            pending_settings: state.pending_settings.clone(),
            day_plans: state.day_plans.clone(),
            schedules: state.schedules.clone(),
            shutdown_delays: state.shutdown_delays.clone(),
            rituals: state.rituals.clone(),
        }
    }
}

/// Serve `service` on a Unix socket at `path` on a background thread.
pub fn serve<P, C>(path: &Path, service: Arc<Mutex<Service<P, C>>>) -> io::Result<()>
where
    P: Platform + 'static,
    C: Clock + 'static,
{
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666))?;
    thread::Builder::new().name("ipc".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            let service = Arc::clone(&service);
            thread::spawn(move || {
                let _ = handle_connection(stream, &service);
            });
        }
    })?;
    Ok(())
}

fn handle_connection<P: Platform, C: Clock>(
    stream: UnixStream,
    service: &Mutex<Service<P, C>>,
) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let response = match serde_json::from_str::<Request>(&line) {
        Ok(request) => match service.lock() {
            Ok(mut s) => s.handle(request),
            Err(poisoned) => poisoned.into_inner().handle(request),
        },
        Err(e) => Response::Rejected {
            reason: format!("bad_request: {e}"),
        },
    };
    let mut stream = stream;
    serde_json::to_writer(&mut stream, &response)?;
    stream.write_all(b"\n")
}

/// Send one request to the service at `path` and wait for its response.
pub fn send(path: &Path, request: &Request) -> io::Result<Response> {
    let mut stream = UnixStream::connect(path)?;
    serde_json::to_writer(&mut stream, request)?;
    stream.write_all(b"\n")?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    serde_json::from_str(&line).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
