//! Cloister rules engine: the single source of truth for what must be enforced right now.
//!
//! One pure interface: [`decide`] takes the current [`State`], a [`Command`] and the
//! **server time**, and returns a [`Decision`]: accepted or rejected (with a [`Reason`]),
//! the new state, and the [`Enforcement`] to apply now. [`enforcement`] answers the
//! same question without a command, which is what enforcement services poll on every tick.
//!
//! The engine never reads a clock. Any client-supplied time in a command is ignored.
//! Behaviour is specified by the shared JSON scenarios in `fixtures/rules`.

mod catalog;
mod day_plan;

use std::collections::BTreeSet;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

pub use catalog::{related_domains, Blocklist, BlocklistKind, Catalog};
pub use day_plan::{
    day_plan, planned_deep_minutes, planned_deep_sessions, schedule_plan, DayPlan, Schedule,
    ScheduleSlot, Slot, SlotInput,
};

/// Everything the engine knows. Grows as later rules arrive (Day Plans, settings, rituals...).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
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
}

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
        } => {
            check_length(Duration::minutes(*minutes), *confirm_long)?;
            let catalog = Catalog::builtin();
            if blocklists.iter().any(|id| catalog.get(id).is_none()) {
                return Err(Reason::UnknownBlocklist);
            }
            if state.blocks.iter().any(|b| &b.id == block_id)
                || state
                    .day_plans
                    .iter()
                    .any(|p| p.slots.iter().any(|s| &s.id == block_id))
            {
                return Err(Reason::DuplicateBlock);
            }
            let mut ids: BTreeSet<String> = catalog.distraction_ids().collect();
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
            });
        }
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

/// What must be enforced at `server_time`, given `state`.
pub fn enforcement(state: &State, server_time: DateTime<Utc>) -> Enforcement {
    let catalog = Catalog::builtin();
    let mut domains = BTreeSet::new();
    let mut apps = BTreeSet::new();
    let planned = planned_deep_sessions(state, server_time);
    for block in state
        .blocks
        .iter()
        .chain(&planned)
        .filter(|b| b.is_active(server_time))
    {
        for list in block.blocklists.iter().filter_map(|id| catalog.get(id)) {
            for domain in &list.domains {
                domains.insert(domain.clone());
                domains.extend(related_domains(domain).iter().cloned());
            }
            apps.extend(list.apps.iter().cloned());
        }
    }
    Enforcement {
        blocked_domains: domains.into_iter().collect(),
        blocked_apps: apps.into_iter().collect(),
        ritual: RitualScreen::None,
    }
}
