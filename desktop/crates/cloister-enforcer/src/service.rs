//! The enforcement service: owns the rules-engine state, answers IPC requests and, on every
//! tick, asks the rules engine for the current Enforcement and applies it.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use cloister_core::{
    decide, enforcement, lockdown, next_workday_start, settings_at, shutdown_delays_left,
    shutdown_ritual_at, Block, BlocklistEdits, Command, Enforcement, Outcome, Reason, Refusal,
    RitualScreen, State, MAX_BLOCK, QUIT_SOCIAL_BLOCK,
};

use crate::account::{AccountLink, Fetched};
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
    /// The trusted time: what commands are decided at.
    fn now(&self) -> DateTime<Utc>;
    /// The wall clock, if it may be ahead of [`Clock::now`] (after the Device was off). The
    /// service enforces what is due at either time, so it is only ever stricter.
    fn wall_hint(&self) -> Option<DateTime<Utc>> {
        None
    }
    /// Server time from the backend (authenticated): re-anchor on it.
    fn set_server_time(&mut self, _server_time: DateTime<Utc>) {}
    /// Called on every tick, so the clock can save its progress.
    fn checkpoint(&mut self) {}
}

fn rejected(refusal: Refusal) -> Response {
    Response::Rejected {
        reason: refusal.code().into(),
    }
}

pub struct Service<P, C> {
    store: StateStore,
    state: State,
    platform: P,
    clock: C,
    domains: SharedDomains,
    ritual_flag: Option<PathBuf>,
    account: Option<AccountLink>,
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
            ritual_flag: None,
            account: None,
        };
        service.tick();
        Ok(service)
    }

    /// Keep a file at `path` while the Shutdown Ritual is due (and only then). The ritual
    /// window's launch agent is kept alive by launchd while that file exists, so the window is
    /// shown at login and relaunched within a second if it is killed.
    pub fn set_ritual_flag(&mut self, path: Option<PathBuf>) {
        self.ritual_flag = path;
    }

    /// Fetch the account from the backend when a Device hands over its access token. Without
    /// it, nothing that lifts enforcement is ever taken.
    pub fn set_account(&mut self, account: AccountLink) {
        self.account = Some(account);
    }

    /// The Enforcement due now: at the trusted time and, when the wall clock is later, at
    /// that time too (the union, so it is only ever stricter).
    fn current_enforcement(&self) -> Enforcement {
        let now = self.clock.now();
        let at_now = enforcement(&self.state, now);
        match self.clock.wall_hint() {
            Some(wall) if wall > now => union(at_now, enforcement(&self.state, wall)),
            _ => at_now,
        }
    }

    /// Re-apply the Enforcement for now, computed by the rules engine from the service's own
    /// persisted state (Blocks, Blocklist edits, settings, Day Plans and Schedules), so
    /// Evening Lockdown and planned Deep Sessions hold with no app or Device sync running.
    /// Returns it.
    pub fn tick(&mut self) -> Enforcement {
        self.clock.checkpoint();
        let current = self.current_enforcement();
        let set = DomainSet::new(&current.blocked_domains);
        match self.domains.write() {
            Ok(mut guard) => *guard = set,
            Err(poisoned) => *poisoned.into_inner() = set,
        }
        if let Err(e) = self.platform.apply(&current) {
            eprintln!("cloisterd: applying enforcement failed: {e}");
        }
        if let Some(path) = &self.ritual_flag {
            if let Err(e) = set_flag(path, current.ritual == RitualScreen::Due) {
                eprintln!("cloisterd: ritual flag {}: {e}", path.display());
            }
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
                    enforcement: self.current_enforcement(),
                    blocklists: self.state.blocklist_edits(),
                    lockdown: lockdown(&self.state, now),
                    next_workday_start: next_workday_start(&self.state, now),
                    settings: settings_at(&self.state, now),
                    ritual_due_at: shutdown_ritual_at(&self.state, now),
                    shutdown_delays_left: shutdown_delays_left(&self.state, now),
                }
            }
            Request::Command { command } => self.command(command),
            Request::Sync {
                blocks,
                blocklists,
                planning,
                access_token,
            } => self.sync(blocks, blocklists, planning, access_token),
        }
    }

    /// Fold in what a Device knows of the account. Unauthenticated, so it only tightens (see
    /// [`merge_blocks`] and [`merge_planning`]). With an access token, the service then fetches
    /// the account from the backend itself and takes it as the backend has it.
    fn sync(
        &mut self,
        blocks: Vec<Block>,
        blocklists: Option<BlocklistEdits>,
        planning: Option<Planning>,
        access_token: Option<String>,
    ) -> Response {
        let Some(mut merged) = merge_blocks(&self.state, &blocks) else {
            return rejected(Refusal::InvalidBlock);
        };
        let news = blocklists
            .as_ref()
            .is_some_and(|b| *b != self.state.blocklist_edits())
            || planning
                .as_ref()
                .is_some_and(|p| *p != Planning::of(&self.state));
        if let Some(edits) = &blocklists {
            merged.merge_blocklist_entries(edits);
        }
        if let Some(planning) = planning {
            merge_planning(&mut merged, &planning, self.clock.now());
        }
        if let (Some(token), Some(account)) = (access_token, self.account.as_mut()) {
            if let Some(fetched) = account.fetch(&token, news) {
                self.clock.set_server_time(fetched.server_time);
                merged = adopt_account(&merged, fetched);
            }
        }
        if merged != self.state {
            if let Err(e) = self.store.save(&merged) {
                eprintln!("cloisterd: saving state failed: {e}");
                return rejected(Refusal::StorageUnavailable);
            }
            self.state = merged;
        }
        Response::Accepted {
            enforcement: self.tick(),
        }
    }

    fn command(&mut self, command: Command) -> Response {
        if let Some(refusal) = not_taken_here(&command) {
            return rejected(refusal);
        }
        let decision = decide(&self.state, &command, self.clock.now());
        match decision.outcome {
            Outcome::Rejected { reason } => Response::Rejected {
                reason: reason.code().into(),
            },
            Outcome::Accepted => {
                if let Err(e) = self.store.save(&decision.state) {
                    eprintln!("cloisterd: saving state failed: {e}");
                    return rejected(Refusal::StorageUnavailable);
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
/// loosen it goes through the backend (ADR 0001, 0004): this says why it isn't taken here.
fn not_taken_here(command: &Command) -> Option<Refusal> {
    match command {
        Command::StartBlock { .. }
        | Command::ExtendBlock { .. }
        | Command::RecordDepthRating { .. }
        | Command::RecordBreach { .. }
        | Command::StartQuitSocialBlock { .. }
        | Command::AddBlocklistEntry { .. }
        | Command::OptInToBlocklist { .. }
        | Command::CaptureLooseEnd { .. } => None,
        // Nowhere: a Block can't be cancelled.
        Command::CancelBlock { .. } => Some(Refusal::Rule(Reason::BlockCannotBeCancelled)),
        // A removal, opt-out or settings change loosens from tomorrow; moving a deep Slot or
        // changing a Schedule can loosen too. The weekly target lives in the account only.
        Command::RemoveBlocklistEntry { .. }
        | Command::OptOutOfBlocklist { .. }
        | Command::WriteDayPlan { .. }
        | Command::PutSchedule { .. }
        | Command::ChangeSettings { .. }
        | Command::SetWeeklyTarget { .. } => Some(Refusal::TakesEffectTomorrow),
        // A Shutdown Delay or a completed ritual lifts enforcement: the backend decides and
        // the service fetches the record from it.
        Command::CloseLooseEnd { .. }
        | Command::UseShutdownDelay {}
        | Command::CompleteRitual { .. } => Some(Refusal::AccountRequired),
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

/// Fold planning from an unauthenticated sender into `state` at `now`, so that it only
/// tightens: the stricter part of the settings applies (no loosening is scheduled), a new
/// Schedule is added through the rules engine (a known one isn't changed), and a Day Plan is
/// taken only if it keeps every started Deep Session and doesn't reduce the deep time still to
/// come. Shutdown Delays, completed rituals and the settings history are only taken from the
/// backend (see [`adopt_account`]).
pub fn merge_planning(state: &mut State, planning: &Planning, now: DateTime<Utc>) {
    state.tighten_settings(
        &planning.time_zone,
        planning.settings.as_ref(),
        planning.pending_settings.as_ref(),
        now,
    );
    for schedule in &planning.schedules {
        if state.schedules.iter().any(|s| s.id == schedule.id) {
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

/// `local` with the account as the backend has it (authenticated, and authoritative): its
/// settings, history, Day Plans, Schedules, Blocklist edits and ritual records are taken as
/// they are; Blocks, Blocklist additions and Breaches only known here are kept.
pub fn adopt_account(local: &State, fetched: Fetched) -> State {
    let mut next = fetched.state;
    next.blocks = merge_blocks(local, &fetched.blocks)
        .unwrap_or_else(|| local.clone())
        .blocks;
    next.merge_blocklist_entries(&local.blocklist_edits());
    for breach in &local.breaches {
        if !next.breaches.iter().any(|b| b.id == breach.id) {
            next.breaches.push(breach.clone());
        }
    }
    next.breaches
        .sort_by(|a, b| (a.occurred_at, &a.id).cmp(&(b.occurred_at, &b.id)));
    next
}

/// Two Enforcements at once: everything either blocks, and the ritual screen that asks more
/// (due before the others).
fn union(a: Enforcement, b: Enforcement) -> Enforcement {
    let mut domains: Vec<String> = a.blocked_domains;
    domains.extend(b.blocked_domains);
    domains.sort();
    domains.dedup();
    let mut apps: Vec<String> = a.blocked_apps;
    apps.extend(b.blocked_apps);
    apps.sort();
    apps.dedup();
    let ritual = if b.ritual == RitualScreen::Due {
        b.ritual
    } else {
        a.ritual
    };
    Enforcement {
        blocked_domains: domains,
        blocked_apps: apps,
        ritual,
    }
}

/// Create (with the ritual's state) or remove the ritual flag file. Idempotent.
fn set_flag(path: &std::path::Path, due: bool) -> io::Result<()> {
    if due {
        if !path.exists() {
            std::fs::write(path, b"due\n")?;
        }
        Ok(())
    } else {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

/// A shared, empty domain set to hand to [`Service::open`] and the DNS filter.
pub fn shared_domains() -> SharedDomains {
    Arc::default()
}
