//! Cloister rules engine: the single source of truth for what must be enforced right now.
//!
//! One pure interface: [`decide`] takes the current [`State`], a [`Command`] and the
//! **server time**, and returns a [`Decision`]: accepted or rejected (with a [`Reason`]),
//! the new state, and the [`Enforcement`] to apply now. [`enforcement`] answers the
//! same question without a command, which is what enforcement services poll on every tick.
//!
//! The engine never reads a clock. Any client-supplied time in a command is ignored.
//! Behaviour is specified by the shared JSON scenarios in `fixtures/rules`.

mod blocklists;
mod catalog;
mod day_plan;
mod ritual;
mod settings;

use std::collections::BTreeSet;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

pub use blocklists::{
    distraction_ids_in_force, effective_entries, next_day_boundary, normalize_app,
    normalize_domain, BlocklistEdits, BlocklistEntry, BlocklistOptOut, BlocklistRemoval, EntryKind,
    DEFAULT_TIME_ZONE,
};
pub use catalog::{related_domains, Blocklist, BlocklistKind, Catalog};
pub use day_plan::{
    day_plan, planned_deep_minutes, planned_deep_sessions, schedule_plan, DayPlan, Schedule,
    ScheduleSlot, Slot, SlotInput,
};
pub use ritual::{
    next_workday_plan, ritual, ritual_due_at, ritual_screen, shutdown_delays_left,
    shutdown_ritual_at, LooseEnd, Ritual, RitualStatus, ShutdownDelay, MAX_LOOSE_END,
    SHUTDOWN_COMPLETE, SHUTDOWN_DELAY,
};
pub use settings::{
    lockdown, next_workday_start, settings_at, workday, Lockdown, PendingSettings, Settings,
    Workday,
};

/// Everything the engine knows. Grows as later rules arrive (Day Plans, settings, rituals...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    #[serde(default)]
    pub blocks: Vec<Block>,
    /// Saved Day Plans, sorted by date.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub day_plans: Vec<DayPlan>,
    /// Schedules, sorted by id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schedules: Vec<Schedule>,
    /// IANA time zone for day boundaries ("tomorrow"). Omitted when UTC.
    #[serde(
        default = "default_time_zone",
        skip_serializing_if = "is_default_time_zone"
    )]
    pub time_zone: String,
    /// Domains and apps the account added to Blocklists, sorted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocklist_entries: Vec<BlocklistEntry>,
    /// Removals from Blocklists, pending or in force, sorted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocklist_removals: Vec<BlocklistRemoval>,
    /// Distraction List categories opted out of, pending or in force, sorted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocklist_opt_outs: Vec<BlocklistOptOut>,
    /// Workdays, Workday Start and Shutdown Time in force (as of the last change). Omitted
    /// until first set; without them there is no Evening Lockdown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<Settings>,
    /// Requested settings that loosen something, waiting for the next day boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_settings: Option<PendingSettings>,
    /// The open Loose Ends, carried between days until closed. Sorted by capture, then id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loose_ends: Vec<LooseEnd>,
    /// Completed Shutdown Rituals, sorted by date.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rituals: Vec<Ritual>,
    /// Shutdown Delays used, sorted by date.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shutdown_delays: Vec<ShutdownDelay>,
}

impl Default for State {
    fn default() -> Self {
        State {
            blocks: Vec::new(),
            day_plans: Vec::new(),
            schedules: Vec::new(),
            time_zone: default_time_zone(),
            blocklist_entries: Vec::new(),
            blocklist_removals: Vec::new(),
            blocklist_opt_outs: Vec::new(),
            settings: None,
            pending_settings: None,
            loose_ends: Vec::new(),
            rituals: Vec::new(),
            shutdown_delays: Vec::new(),
        }
    }
}

fn default_time_zone() -> String {
    DEFAULT_TIME_ZONE.to_string()
}

fn is_default_time_zone(tz: &String) -> bool {
    tz == DEFAULT_TIME_ZONE
}

/// A period during which Blocklists are enforced. Its end can only move later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Block {
    pub id: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    /// Ids of the Blocklists enforced, sorted.
    pub blocklists: Vec<String>,
    /// Whether this Block is a Deep Session.
    pub deep: bool,
    /// What this Deep Session will produce.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    /// How deep the Deep Session was, 1 to 5, recorded once after it ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_rating: Option<u8>,
    /// Sites added to this Block only, normalized and sorted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_domains: Vec<String>,
    /// Apps added to this Block only, sorted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_apps: Vec<String>,
    /// Whether this is the 30-day Quit-Social Block.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub quit_social: bool,
}

impl Block {
    pub fn is_active(&self, now: DateTime<Utc>) -> bool {
        self.starts_at <= now && now < self.ends_at
    }
}

/// A request to change the state. Serialized as `{"type": "<snake_case>", ...fields}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    /// Start a Block now (at server time). The Distraction List is always included;
    /// `blocklists` adds extra Blocklists.
    StartBlock {
        block_id: String,
        minutes: i64,
        #[serde(default = "default_true")]
        deep: bool,
        #[serde(default)]
        blocklists: Vec<String>,
        /// Required for Blocks longer than 4 hours.
        #[serde(default)]
        confirm_long: bool,
        /// The Device's own clock. Recorded for diagnosis only; never trusted.
        #[serde(default)]
        client_time: Option<DateTime<Utc>>,
        /// The Deep Session's Intent. Trimmed; blank means none.
        #[serde(default)]
        intent: Option<String>,
        /// Sites blocked for this Block only.
        #[serde(default)]
        extra_domains: Vec<String>,
        /// Apps blocked for this Block only.
        #[serde(default)]
        extra_apps: Vec<String>,
    },
    /// Move the end of an active Block later. Moving it earlier is rejected.
    ExtendBlock {
        block_id: String,
        ends_at: DateTime<Utc>,
        /// Required when the Block becomes longer than 4 hours.
        #[serde(default)]
        confirm_long: bool,
    },
    /// Always rejected: a Block can't be cancelled. Exists so the refusal is explicit.
    CancelBlock { block_id: String },
    /// Record the Depth Rating (1 to 5) of a Deep Session that has ended. Once only.
    RecordDepthRating { block_id: String, rating: u8 },
    /// Add a domain or app to a Blocklist. Applies immediately.
    AddBlocklistEntry {
        blocklist: String,
        kind: EntryKind,
        entry: String,
    },
    /// Remove a domain or app from a Blocklist. Takes effect at the next day boundary.
    RemoveBlocklistEntry {
        blocklist: String,
        kind: EntryKind,
        entry: String,
    },
    /// Stop including a Distraction List category in new Blocks, from the next day boundary.
    OptOutOfBlocklist { blocklist: String },
    /// Include a Distraction List category again. Applies immediately.
    OptInToBlocklist { blocklist: String },
    /// Start the 30-day Quit-Social Block (Social only). Needs `confirm_long`.
    StartQuitSocialBlock {
        block_id: String,
        #[serde(default)]
        confirm_long: bool,
    },
    /// Save the Day Plan for `date`, replacing any saved one. Planned Deep Minutes can't go
    /// down, a started Deep Session can't change, and no deep Slot can start in the past.
    WriteDayPlan {
        date: chrono::NaiveDate,
        starts_at: DateTime<Utc>,
        ends_at: DateTime<Utc>,
        slots: Vec<SlotInput>,
    },
    /// Create or replace a Schedule. A day already running on the old Schedule keeps its plan.
    PutSchedule {
        schedule_id: String,
        time_zone: String,
        weekdays: Vec<u8>,
        starts: String,
        ends: String,
        slots: Vec<ScheduleSlot>,
    },
    /// Set Workdays (ISO 1-7), Workday Start and Shutdown Time ("HH:MM", local). What
    /// tightens applies now, what loosens from the next day boundary. `time_zone` (IANA) is
    /// taken with the first settings only.
    ChangeSettings {
        workdays: Vec<u8>,
        workday_start: String,
        shutdown_time: String,
        #[serde(default)]
        time_zone: Option<String>,
    },
    /// Add a Loose End (text trimmed, 1 to 500 characters).
    CaptureLooseEnd { loose_end_id: String, text: String },
    /// Close a Loose End: it leaves the list.
    CloseLooseEnd { loose_end_id: String },
    /// Postpone today's ritual and Evening Lockdown by 30 minutes. Once per ISO week, on a
    /// Workday before its Shutdown Time.
    UseShutdownDelay {},
    /// Complete the due Shutdown Ritual. `typed` must be exactly "Shutdown complete".
    CompleteRitual { typed: String },
}

/// How long the Quit-Social Block lasts: the one exception to the 24-hour cap.
pub const QUIT_SOCIAL_BLOCK: Duration = Duration::days(30);
/// The Blocklist the Quit-Social Block covers.
pub const QUIT_SOCIAL_BLOCKLIST: &str = "social";

fn default_true() -> bool {
    true
}

/// Why a command was rejected. Serialized as a stable snake_case code shared by every engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    InvalidLength,
    BlockTooLong,
    ConfirmationRequired,
    DuplicateBlock,
    UnknownBlock,
    UnknownBlocklist,
    BlockCannotBeShortened,
    BlockCannotBeCancelled,
    BlockHasEnded,
    NotADeepSession,
    DeepSessionNotEnded,
    InvalidDepthRating,
    DepthRatingAlreadyRecorded,
    InvalidEntry,
    UnknownEntry,
    CannotOptOut,
    InvalidDayPlan,
    InvalidSlot,
    SlotsOverlap,
    DuplicateSlot,
    DeepSessionHasStarted,
    SlotInThePast,
    SlotCannotBeRemoved,
    InvalidSchedule,
    UnknownTimeZone,
    SchedulesOverlap,
    EveningLockdown,
    RestDay,
    InvalidSettings,
    TimeZoneCannotChange,
    InvalidLooseEnd,
    DuplicateLooseEnd,
    UnknownLooseEnd,
    NoShutdownTime,
    ShutdownDelayUsed,
    RitualNotDue,
    RitualAlreadyCompleted,
    LooseEndRequired,
    DayPlanRequired,
    DeepSlotRequired,
    WorkdayNotCovered,
    ConfirmationMismatch,
}

impl Reason {
    pub fn code(&self) -> &'static str {
        match self {
            Reason::InvalidLength => "invalid_length",
            Reason::BlockTooLong => "block_too_long",
            Reason::ConfirmationRequired => "confirmation_required",
            Reason::DuplicateBlock => "duplicate_block",
            Reason::UnknownBlock => "unknown_block",
            Reason::UnknownBlocklist => "unknown_blocklist",
            Reason::BlockCannotBeShortened => "block_cannot_be_shortened",
            Reason::BlockCannotBeCancelled => "block_cannot_be_cancelled",
            Reason::BlockHasEnded => "block_has_ended",
            Reason::NotADeepSession => "not_a_deep_session",
            Reason::DeepSessionNotEnded => "deep_session_not_ended",
            Reason::InvalidDepthRating => "invalid_depth_rating",
            Reason::DepthRatingAlreadyRecorded => "depth_rating_already_recorded",
            Reason::InvalidEntry => "invalid_entry",
            Reason::UnknownEntry => "unknown_entry",
            Reason::CannotOptOut => "cannot_opt_out",
            Reason::InvalidDayPlan => "invalid_day_plan",
            Reason::InvalidSlot => "invalid_slot",
            Reason::SlotsOverlap => "slots_overlap",
            Reason::DuplicateSlot => "duplicate_slot",
            Reason::DeepSessionHasStarted => "deep_session_has_started",
            Reason::SlotInThePast => "slot_in_the_past",
            Reason::SlotCannotBeRemoved => "slot_cannot_be_removed",
            Reason::InvalidSchedule => "invalid_schedule",
            Reason::UnknownTimeZone => "unknown_time_zone",
            Reason::SchedulesOverlap => "schedules_overlap",
            Reason::EveningLockdown => "evening_lockdown",
            Reason::RestDay => "rest_day",
            Reason::InvalidSettings => "invalid_settings",
            Reason::TimeZoneCannotChange => "time_zone_cannot_change",
            Reason::InvalidLooseEnd => "invalid_loose_end",
            Reason::DuplicateLooseEnd => "duplicate_loose_end",
            Reason::UnknownLooseEnd => "unknown_loose_end",
            Reason::NoShutdownTime => "no_shutdown_time",
            Reason::ShutdownDelayUsed => "shutdown_delay_used",
            Reason::RitualNotDue => "ritual_not_due",
            Reason::RitualAlreadyCompleted => "ritual_already_completed",
            Reason::LooseEndRequired => "loose_end_required",
            Reason::DayPlanRequired => "day_plan_required",
            Reason::DeepSlotRequired => "deep_slot_required",
            Reason::WorkdayNotCovered => "workday_not_covered",
            Reason::ConfirmationMismatch => "confirmation_mismatch",
        }
    }
}

/// The longest a Block may be.
pub const MAX_BLOCK: Duration = Duration::hours(24);
/// Blocks longer than this need explicit confirmation.
pub const CONFIRM_LONG_AFTER: Duration = Duration::hours(4);

fn check_length(length: Duration, confirm_long: bool) -> Result<(), Reason> {
    if length <= Duration::zero() {
        Err(Reason::InvalidLength)
    } else if length > MAX_BLOCK {
        Err(Reason::BlockTooLong)
    } else if length > CONFIRM_LONG_AFTER && !confirm_long {
        Err(Reason::ConfirmationRequired)
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Accepted,
    Rejected { reason: Reason },
}

/// What a Device must enforce right now.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Enforcement {
    /// Sorted, deduplicated. Each entry also blocks all of its subdomains.
    pub blocked_domains: Vec<String>,
    /// Sorted, deduplicated app identifiers.
    pub blocked_apps: Vec<String>,
    pub ritual: RitualScreen,
}

impl Enforcement {
    pub fn is_empty(&self) -> bool {
        self.blocked_domains.is_empty() && self.blocked_apps.is_empty()
    }
}

/// The Shutdown Ritual screen state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RitualScreen {
    #[default]
    None,
    Due,
    Delayed,
    Completed,
    Missed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub outcome: Outcome,
    pub state: State,
    pub enforcement: Enforcement,
}

/// Apply `command` to `state` at `server_time`.
pub fn decide(state: &State, command: &Command, server_time: DateTime<Utc>) -> Decision {
    match apply(state, command, server_time) {
        Ok(new_state) => Decision {
            enforcement: enforcement(&new_state, server_time),
            state: new_state,
            outcome: Outcome::Accepted,
        },
        Err(reason) => Decision {
            enforcement: enforcement(state, server_time),
            state: state.clone(),
            outcome: Outcome::Rejected { reason },
        },
    }
}

fn apply(state: &State, command: &Command, now: DateTime<Utc>) -> Result<State, Reason> {
    let mut next = state.clone();
    match command {
        Command::StartBlock {
            block_id,
            minutes,
            deep,
            blocklists,
            confirm_long,
            client_time: _,
            intent,
            extra_domains,
            extra_apps,
        } => {
            check_length(Duration::minutes(*minutes), *confirm_long)?;
            let catalog = Catalog::builtin();
            if blocklists.iter().any(|id| catalog.get(id).is_none()) {
                return Err(Reason::UnknownBlocklist);
            }
            let extra_domains = blocklists::normalize_all(EntryKind::Domain, extra_domains)?;
            let extra_apps = blocklists::normalize_all(EntryKind::App, extra_apps)?;
            if state.blocks.iter().any(|b| &b.id == block_id) || is_slot_id(state, block_id) {
                return Err(Reason::DuplicateBlock);
            }
            if *deep {
                if let Some(lockdown) = lockdown(state, now) {
                    return Err(lockdown.reason());
                }
            }
            let mut ids = distraction_ids_in_force(state, now);
            ids.extend(blocklists.iter().cloned());
            next.blocks.push(Block {
                id: block_id.clone(),
                starts_at: now,
                ends_at: now + Duration::minutes(*minutes),
                blocklists: ids.into_iter().collect(),
                deep: *deep,
                intent: intent
                    .as_deref()
                    .map(str::trim)
                    .filter(|i| !i.is_empty())
                    .map(String::from),
                depth_rating: None,
                extra_domains,
                extra_apps,
                quit_social: false,
            });
        }
        Command::StartQuitSocialBlock {
            block_id,
            confirm_long,
        } => {
            if !confirm_long {
                return Err(Reason::ConfirmationRequired);
            }
            if state.blocks.iter().any(|b| &b.id == block_id) || is_slot_id(state, block_id) {
                return Err(Reason::DuplicateBlock);
            }
            next.blocks.push(Block {
                id: block_id.clone(),
                starts_at: now,
                ends_at: now + QUIT_SOCIAL_BLOCK,
                blocklists: vec![QUIT_SOCIAL_BLOCKLIST.to_string()],
                deep: false,
                intent: None,
                depth_rating: None,
                extra_domains: Vec::new(),
                extra_apps: Vec::new(),
                quit_social: true,
            });
        }
        Command::AddBlocklistEntry {
            blocklist,
            kind,
            entry,
        } => blocklists::add_entry(&mut next, blocklist, *kind, entry)?,
        Command::RemoveBlocklistEntry {
            blocklist,
            kind,
            entry,
        } => blocklists::remove_entry(&mut next, blocklist, *kind, entry, now)?,
        Command::OptOutOfBlocklist { blocklist } => blocklists::opt_out(&mut next, blocklist, now)?,
        Command::OptInToBlocklist { blocklist } => blocklists::opt_in(&mut next, blocklist)?,
        Command::ExtendBlock {
            block_id,
            ends_at,
            confirm_long,
        } => {
            let block = find(&mut next, block_id)?;
            if now >= block.ends_at {
                return Err(Reason::BlockHasEnded);
            }
            if *ends_at < block.ends_at {
                return Err(Reason::BlockCannotBeShortened);
            }
            if *ends_at == block.ends_at {
                return Ok(next);
            }
            check_length(*ends_at - block.starts_at, *confirm_long)?;
            block.ends_at = *ends_at;
        }
        Command::CancelBlock { block_id } => {
            find(&mut next, block_id)?;
            return Err(Reason::BlockCannotBeCancelled);
        }
        Command::RecordDepthRating { block_id, rating } => {
            // A Deep Session is a deep Block or a deep Slot of a saved Day Plan.
            let (deep, ends_at, recorded) = match find(&mut next, block_id) {
                Ok(block) => (block.deep, block.ends_at, &mut block.depth_rating),
                Err(e) => match day_plan::find_slot(&mut next, block_id) {
                    Some(slot) => (slot.deep, slot.ends_at, &mut slot.depth_rating),
                    None => return Err(e),
                },
            };
            if !deep {
                return Err(Reason::NotADeepSession);
            }
            if now < ends_at {
                return Err(Reason::DeepSessionNotEnded);
            }
            if !(1..=5).contains(rating) {
                return Err(Reason::InvalidDepthRating);
            }
            if recorded.is_some() {
                return Err(Reason::DepthRatingAlreadyRecorded);
            }
            *recorded = Some(*rating);
        }
        Command::WriteDayPlan {
            date,
            starts_at,
            ends_at,
            slots,
        } => day_plan::write_day_plan(&mut next, *date, *starts_at, *ends_at, slots, now)?,
        Command::PutSchedule {
            schedule_id,
            time_zone,
            weekdays,
            starts,
            ends,
            slots,
        } => day_plan::put_schedule(
            &mut next,
            schedule_id,
            time_zone,
            weekdays,
            starts,
            ends,
            slots,
            now,
        )?,
        Command::ChangeSettings {
            workdays,
            workday_start,
            shutdown_time,
            time_zone,
        } => settings::change_settings(
            &mut next,
            workdays,
            workday_start,
            shutdown_time,
            time_zone.as_deref(),
            now,
        )?,
        Command::CaptureLooseEnd { loose_end_id, text } => {
            ritual::capture_loose_end(&mut next, loose_end_id, text, now)?
        }
        Command::CloseLooseEnd { loose_end_id } => {
            ritual::close_loose_end(&mut next, loose_end_id)?
        }
        Command::UseShutdownDelay {} => ritual::use_shutdown_delay(&mut next, now)?,
        Command::CompleteRitual { typed } => ritual::complete_ritual(&mut next, typed, now)?,
    }
    Ok(next)
}

fn find<'a>(state: &'a mut State, block_id: &str) -> Result<&'a mut Block, Reason> {
    state
        .blocks
        .iter_mut()
        .find(|b| b.id == block_id)
        .ok_or(Reason::UnknownBlock)
}

/// Whether `id` names a Slot of a saved Day Plan (Block ids and Slot ids share one space).
fn is_slot_id(state: &State, id: &str) -> bool {
    state
        .day_plans
        .iter()
        .any(|p| p.slots.iter().any(|s| s.id == id))
}

/// What must be enforced at `server_time`, given `state`.
pub fn enforcement(state: &State, server_time: DateTime<Utc>) -> Enforcement {
    let mut lists = BTreeSet::new();
    let mut domains = BTreeSet::new();
    let mut apps = BTreeSet::new();
    let planned = planned_deep_sessions(state, server_time);
    for block in state
        .blocks
        .iter()
        .chain(&planned)
        .filter(|b| b.is_active(server_time))
    {
        lists.extend(block.blocklists.iter().cloned());
        // Opting back in to a category applies at once, also to a running Block.
        if !block.quit_social {
            lists.extend(distraction_ids_in_force(state, server_time));
        }
        domains.extend(block.extra_domains.iter().cloned());
        apps.extend(block.extra_apps.iter().cloned());
    }
    // Evening Lockdown and Rest Days: the Work List and the Distraction List, on the clock.
    if lockdown(state, server_time).is_some() {
        lists.extend(Catalog::builtin().work_ids());
        lists.extend(distraction_ids_in_force(state, server_time));
    }
    for list in &lists {
        domains.extend(effective_entries(
            state,
            list,
            EntryKind::Domain,
            server_time,
        ));
        apps.extend(effective_entries(state, list, EntryKind::App, server_time));
    }
    let related: Vec<String> = domains
        .iter()
        .flat_map(|d| related_domains(d).iter().cloned())
        .collect();
    domains.extend(related);
    Enforcement {
        blocked_domains: domains.into_iter().collect(),
        blocked_apps: apps.into_iter().collect(),
        ritual: ritual_screen(state, server_time),
    }
}
