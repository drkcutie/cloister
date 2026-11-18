//! The enforcement service: owns the rules-engine state, answers IPC requests and, on every
//! tick, asks the rules engine for the current Enforcement and applies it.

use std::io;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use cloister_core::{
    decide, enforcement, lockdown, next_workday_start, settings_at, Block, BlocklistEdits, Command,
    Enforcement, Outcome, State, MAX_BLOCK, QUIT_SOCIAL_BLOCK,
};

use crate::dns::SharedDomains;
use crate::domains::DomainSet;
use crate::ipc::{Planning, Request, Response};
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
/// Rejection code for a synced Block that no rule could have produced.
pub const INVALID_BLOCK: &str = "invalid_block";
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

    /// Re-apply the Enforcement for now, computed by the rules engine from the service's own
    /// persisted state (Blocks, Blocklist edits, settings, Day Plans and Schedules), so
    /// Evening Lockdown and planned Deep Sessions hold with no app or Device sync running.
    /// Returns it.
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
            Request::Status => {
                let now = self.clock.now();
                Response::Status {
                    server_time: now,
                    blocks: self.state.blocks.clone(),
                    enforcement: enforcement(&self.state, now),
                    blocklists: self.state.blocklist_edits(),
                    lockdown: lockdown(&self.state, now),
                    next_workday_start: next_workday_start(&self.state, now),
                    settings: settings_at(&self.state, now),
                }
            }
            Request::Command { command } => self.command(command),
            Request::Sync {
                blocks,
                blocklists,
                planning,
            } => self.sync(blocks, blocklists, planning),
        }
    }

    /// Fold in Blocks from the backend. Only tightens (see [`merge_blocks`]).
    fn sync(
        &mut self,
        blocks: Vec<Block>,
        blocklists: Option<BlocklistEdits>,
        planning: Option<Planning>,
    ) -> Response {
        let Some(mut merged) = merge_blocks(&self.state, &blocks) else {
            return Response::Rejected {
                reason: INVALID_BLOCK.into(),
            };
        };
        if let Some(edits) = blocklists {
            merged.merge_blocklist_edits(&edits, self.clock.now());
        }
        if let Some(planning) = planning {
            merge_planning(&mut merged, &planning, self.clock.now());
        }
        if merged != self.state {
            if let Err(e) = self.store.save(&merged) {
                eprintln!("cloisterd: saving state failed: {e}");
                return Response::Rejected {
                    reason: STORAGE_UNAVAILABLE.into(),
                };
            }
            self.state = merged;
        }
        Response::Accepted {
            enforcement: self.tick(),
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
        | Command::RecordDepthRating { .. }
        | Command::RecordBreach { .. }
        | Command::StartQuitSocialBlock { .. }
        | Command::AddBlocklistEntry { .. }
        | Command::OptInToBlocklist { .. } => true,
        Command::CancelBlock { .. }
        | Command::RemoveBlocklistEntry { .. }
        | Command::OptOutOfBlocklist { .. } => false,
        // Moving a deep Slot or changing a Schedule can loosen today: the backend decides.
        // Settings can loosen tomorrow; the backend decides and the Sync hands them over.
        Command::WriteDayPlan { .. }
        | Command::PutSchedule { .. }
        | Command::ChangeSettings { .. } => false,
    }
}

/// `state` with `incoming` folded in: new Blocks are added and a later end is taken; nothing
/// is shortened or removed. A known Intent or Depth Rating is kept. `None` if an incoming Block
/// is impossible (empty, or over 24 hours unless it is a 30-day Quit-Social Block).
pub fn merge_blocks(state: &State, incoming: &[Block]) -> Option<State> {
    let mut next = state.clone();
    for block in incoming {
        let length = block.ends_at - block.starts_at;
        let cap = if block.quit_social {
            QUIT_SOCIAL_BLOCK
        } else {
            MAX_BLOCK
        };
        if length <= chrono::Duration::zero() || length > cap {
            return None;
        }
        match next.blocks.iter_mut().find(|b| b.id == block.id) {
            Some(known) => {
                known.ends_at = known.ends_at.max(block.ends_at);
                if known.intent.is_none() {
                    known.intent.clone_from(&block.intent);
                }
                if known.depth_rating.is_none() {
                    known.depth_rating = block.depth_rating;
                }
            }
            None => next.blocks.push(block.clone()),
        }
    }
    Some(next)
}

/// Fold the account's planning into `state` at `now` without loosening anything before
/// tomorrow: settings tighten now and loosen from the next day boundary, a Schedule change goes
/// through the rules engine (a running day keeps its plan), and a Day Plan is taken only if it
/// keeps every started Deep Session and doesn't reduce the deep time still to come.
pub fn merge_planning(state: &mut State, planning: &Planning, now: DateTime<Utc>) {
    state.merge_settings(
        &planning.time_zone,
        planning.settings.as_ref(),
        planning.pending_settings.as_ref(),
        now,
    );
    for schedule in &planning.schedules {
        if state.schedules.contains(schedule) {
            continue;
        }
        let command = Command::PutSchedule {
            schedule_id: schedule.id.clone(),
            time_zone: schedule.time_zone.clone(),
            weekdays: schedule.weekdays.clone(),
            starts: schedule.starts.clone(),
            ends: schedule.ends.clone(),
            slots: schedule.slots.clone(),
        };
        let decision = decide(state, &command, now);
        if decision.outcome == Outcome::Accepted {
            *state = decision.state;
        }
    }
    for plan in &planning.day_plans {
        state.merge_day_plan(plan, now);
    }
}

/// A shared, empty domain set to hand to [`Service::open`] and the DNS filter.
pub fn shared_domains() -> SharedDomains {
    Arc::default()
}
