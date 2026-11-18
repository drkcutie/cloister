//! The enforcement service: owns the rules-engine state, answers IPC requests and, on every
//! tick, asks the rules engine for the current Enforcement and applies it.

use std::io;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use cloister_core::{decide, enforcement, Command, Enforcement, Outcome, State};

use crate::dns::SharedDomains;
use crate::domains::DomainSet;
use crate::ipc::{Request, Response};
use crate::store::StateStore;

/// The privileged layer: make the operating system enforce `enforcement`. Called on every
/// tick, so implementations must be idempotent and re-assert anything that was undone.
pub trait Platform: Send {
    fn apply(&mut self, enforcement: &Enforcement) -> io::Result<()>;
}

impl<P: Platform + ?Sized> Platform for Box<P> {
    fn apply(&mut self, enforcement: &Enforcement) -> io::Result<()> {
        (**self).apply(enforcement)
    }
}

/// The service's source of "server time".
pub trait Clock: Send {
    fn now(&self) -> DateTime<Utc>;
}

/// Rejection code for commands the service never accepts over IPC.
pub const LOOSENING_NOT_ACCEPTED: &str = "loosening_not_accepted";
/// Rejection code when an accepted command couldn't be saved.
pub const STORAGE_UNAVAILABLE: &str = "storage_unavailable";

pub struct Service<P, C> {
    store: StateStore,
    state: State,
    platform: P,
    clock: C,
    domains: SharedDomains,
}

impl<P: Platform, C: Clock> Service<P, C> {
    /// Load the saved state and apply it immediately.
    pub fn open(
        store: StateStore,
        platform: P,
        clock: C,
        domains: SharedDomains,
    ) -> io::Result<Self> {
        let state = store.load()?;
        let mut service = Service {
            store,
            state,
            platform,
            clock,
            domains,
        };
        service.tick();
        Ok(service)
    }

    /// Re-apply the Enforcement for now. Returns it.
    pub fn tick(&mut self) -> Enforcement {
        let current = enforcement(&self.state, self.clock.now());
        let set = DomainSet::new(&current.blocked_domains);
        match self.domains.write() {
            Ok(mut guard) => *guard = set,
            Err(poisoned) => *poisoned.into_inner() = set,
        }
        if let Err(e) = self.platform.apply(&current) {
            eprintln!("cloisterd: applying enforcement failed: {e}");
        }
        current
    }

    pub fn handle(&mut self, request: Request) -> Response {
        match request {
            Request::Status => Response::Status {
                server_time: self.clock.now(),
                blocks: self.state.blocks.clone(),
                enforcement: enforcement(&self.state, self.clock.now()),
            },
            Request::Command { command } => self.command(command),
        }
    }

    fn command(&mut self, command: Command) -> Response {
        if !tightens(&command) {
            return Response::Rejected {
                reason: LOOSENING_NOT_ACCEPTED.into(),
            };
        }
        let decision = decide(&self.state, &command, self.clock.now());
        match decision.outcome {
            Outcome::Rejected { reason } => Response::Rejected {
                reason: reason.code().into(),
            },
            Outcome::Accepted => {
                if let Err(e) = self.store.save(&decision.state) {
                    eprintln!("cloisterd: saving state failed: {e}");
                    return Response::Rejected {
                        reason: STORAGE_UNAVAILABLE.into(),
                    };
                }
                self.state = decision.state;
                Response::Accepted {
                    enforcement: self.tick(),
                }
            }
        }
    }
}

/// Only commands that can't loosen enforcement are accepted locally: those that make it
/// stricter, and records such as a Depth Rating that don't touch it. Anything that could
/// loosen it goes through the backend, which applies it tomorrow (ADR 0001).
fn tightens(command: &Command) -> bool {
    match command {
        Command::StartBlock { .. }
        | Command::ExtendBlock { .. }
        | Command::RecordDepthRating { .. } => true,
        Command::CancelBlock { .. } => false,
    }
}

/// A shared, empty domain set to hand to [`Service::open`] and the DNS filter.
pub fn shared_domains() -> SharedDomains {
    Arc::default()
}
