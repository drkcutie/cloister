//! Day Plans, Slots and Schedules.
//!
//! A Day Plan divides one day into labelled Slots. A deep Slot becomes a Deep Session at its
//! start time: it is never stored as a Block, it is derived from the plan whenever the
//! Enforcement is asked for. A day without a saved Day Plan falls back to its Schedule.
//!
//! The ratchet: a day's Planned Deep Minutes (the total length of its deep Slots) never go
//! down. Deep Slots can move, but a Deep Session that has started can't change, and no deep
//! Slot can be placed before the server time.

use std::collections::BTreeSet;

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::{Block, Catalog, Reason, State};

/// One day divided into Slots. `starts_at`/`ends_at` are the Workday's bounds (Workday Start
/// to Shutdown Time); until those settings exist (ticket #8) they come with the plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DayPlan {
    pub date: NaiveDate,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    /// The total length of the deep Slots, in minutes. Only goes up.
    pub planned_deep_minutes: i64,
    /// Sorted by start.
    pub slots: Vec<Slot>,
}

/// A labelled stretch of time on a Day Plan. A deep Slot is a scheduled Deep Session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    pub id: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub label: String,
    pub deep: bool,
    /// The Depth Rating of the Deep Session this deep Slot became.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_rating: Option<u8>,
}

impl Slot {
    fn minutes(&self) -> i64 {
        (self.ends_at - self.starts_at).num_minutes()
    }

    /// The same Slot, ignoring its Depth Rating.
    fn same_as(&self, other: &SlotInput) -> bool {
        self.id == other.id
            && self.starts_at == other.starts_at
            && self.ends_at == other.ends_at
            && self.label == other.label.trim()
            && self.deep == other.deep
    }

    /// The Deep Session this deep Slot becomes.
    pub fn deep_session(&self) -> Block {
        Block {
            id: self.id.clone(),
            starts_at: self.starts_at,
            ends_at: self.ends_at,
            blocklists: Catalog::builtin().distraction_ids().collect(),
            deep: true,
            intent: Some(self.label.clone()),
            depth_rating: self.depth_rating,
        }
    }
}

/// A Slot as written in a `write_day_plan` command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotInput {
    pub id: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub label: String,
    #[serde(default)]
    pub deep: bool,
}

/// A recurring template for the Day Plan of the given ISO weekdays (1 = Monday ... 7 = Sunday).
/// Times are local "HH:MM" in `time_zone` (an IANA name).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schedule {
    pub id: String,
    pub time_zone: String,
    /// Sorted.
    pub weekdays: Vec<u8>,
    pub starts: String,
    pub ends: String,
    /// Sorted by start.
    pub slots: Vec<ScheduleSlot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleSlot {
    pub starts: String,
    pub ends: String,
    pub label: String,
    #[serde(default)]
    pub deep: bool,
}

/// The longest a Day Plan may be.
pub const MAX_DAY: Duration = Duration::hours(24);

/// Planned Deep Minutes of a list of Slots.
pub fn planned_deep_minutes(slots: &[Slot]) -> i64 {
    slots.iter().filter(|s| s.deep).map(Slot::minutes).sum()
}

/// The Day Plan for `date`: the saved one, or else one filled in from its Schedule (the draft
/// the Shutdown Ritual starts from, and the plan a day without a saved one runs on).
pub fn day_plan(state: &State, date: NaiveDate) -> Option<DayPlan> {
    saved_plan(state, date)
        .cloned()
        .or_else(|| schedule_plan(state, date))
}

fn saved_plan(state: &State, date: NaiveDate) -> Option<&DayPlan> {
    state.day_plans.iter().find(|p| p.date == date)
}

/// The plan `date` gets from its Schedule, if a Schedule covers its weekday. Slot ids are
/// `<schedule id>-<date>-<n>`, n counting from 1 in order of start.
pub fn schedule_plan(state: &State, date: NaiveDate) -> Option<DayPlan> {
    let weekday = date.weekday().number_from_monday() as u8;
    let schedule = state
        .schedules
        .iter()
        .find(|s| s.weekdays.contains(&weekday))?;
    schedule_plan_of(schedule, date)
}

fn schedule_plan_of(schedule: &Schedule, date: NaiveDate) -> Option<DayPlan> {
    let tz: Tz = schedule.time_zone.parse().ok()?;
    let at = |hhmm: &str| local(tz, date, parse_hhmm(hhmm)?);
    let slots = schedule
        .slots
        .iter()
        .enumerate()
        .map(|(i, s)| {
            Some(Slot {
                id: format!("{}-{date}-{}", schedule.id, i + 1),
                starts_at: at(&s.starts)?,
                ends_at: at(&s.ends)?,
                label: s.label.clone(),
                deep: s.deep,
                depth_rating: None,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(DayPlan {
        date,
        starts_at: at(&schedule.starts)?,
        ends_at: at(&schedule.ends)?,
        planned_deep_minutes: planned_deep_minutes(&slots),
        slots,
    })
}

/// Local wall time to UTC. An ambiguous time takes the earlier instant; a time skipped by a
/// clock change moves an hour later.
fn local(tz: Tz, date: NaiveDate, time: NaiveTime) -> Option<DateTime<Utc>> {
    let naive = date.and_time(time);
    tz.from_local_datetime(&naive)
        .earliest()
        .or_else(|| {
            tz.from_local_datetime(&(naive + Duration::hours(1)))
                .earliest()
        })
        .map(|t| t.with_timezone(&Utc))
}

/// Strict "HH:MM", 00:00 to 23:59.
fn parse_hhmm(s: &str) -> Option<NaiveTime> {
    let b = s.as_bytes();
    if b.len() != 5
        || b[2] != b':'
        || !s
            .chars()
            .enumerate()
            .all(|(i, c)| i == 2 || c.is_ascii_digit())
    {
        return None;
    }
    NaiveTime::parse_from_str(s, "%H:%M").ok()
}

/// The plan in force for `date` at `now`: the saved one, or the Schedule's once its day has
/// begun (before that, the Schedule is only a draft).
fn plan_in_force(state: &State, date: NaiveDate, now: DateTime<Utc>) -> Option<DayPlan> {
    match saved_plan(state, date) {
        Some(plan) => Some(plan.clone()),
        None => schedule_plan(state, date).filter(|p| p.starts_at <= now),
    }
}

/// Every Deep Session that deep Slots become around `now`: all deep Slots of saved Day Plans,
/// plus those of the Schedule-filled plan of the current day in each Schedule's time zone (when
/// that day has no saved plan). Sorted by start.
pub fn planned_deep_sessions(state: &State, now: DateTime<Utc>) -> Vec<Block> {
    let mut slots: Vec<Slot> = state
        .day_plans
        .iter()
        .flat_map(|p| p.slots.iter().cloned())
        .collect();
    let dates: BTreeSet<NaiveDate> = state
        .schedules
        .iter()
        .filter_map(|s| s.time_zone.parse::<Tz>().ok())
        .map(|tz| now.with_timezone(&tz).date_naive())
        .collect();
    for date in dates {
        if saved_plan(state, date).is_none() {
            if let Some(plan) = schedule_plan(state, date) {
                slots.extend(plan.slots);
            }
        }
    }
    let mut sessions: Vec<Block> = slots
        .iter()
        .filter(|s| s.deep)
        .map(Slot::deep_session)
        .collect();
    sessions.sort_by(|a, b| (a.starts_at, &a.id).cmp(&(b.starts_at, &b.id)));
    sessions
}

pub(crate) fn write_day_plan(
    state: &mut State,
    date: NaiveDate,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    input: &[SlotInput],
    now: DateTime<Utc>,
) -> Result<(), Reason> {
    if ends_at <= starts_at || ends_at - starts_at > MAX_DAY {
        return Err(Reason::InvalidDayPlan);
    }
    let mut input: Vec<&SlotInput> = input.iter().collect();
    for s in &input {
        if s.label.trim().is_empty()
            || s.ends_at <= s.starts_at
            || s.starts_at < starts_at
            || s.ends_at > ends_at
        {
            return Err(Reason::InvalidSlot);
        }
    }
    input.sort_by(|a, b| (a.starts_at, &a.id).cmp(&(b.starts_at, &b.id)));
    if input.windows(2).any(|w| w[1].starts_at < w[0].ends_at) {
        return Err(Reason::SlotsOverlap);
    }
    let mut ids = BTreeSet::new();
    for s in &input {
        let elsewhere = state
            .day_plans
            .iter()
            .filter(|p| p.date != date)
            .any(|p| p.slots.iter().any(|o| o.id == s.id))
            || state.blocks.iter().any(|b| b.id == s.id);
        if !ids.insert(s.id.as_str()) || elsewhere {
            return Err(Reason::DuplicateSlot);
        }
    }
    let old = plan_in_force(state, date, now);
    let old_slots = old.as_ref().map(|p| p.slots.as_slice()).unwrap_or_default();
    let unchanged = |s: &SlotInput| old_slots.iter().find(|o| o.same_as(s));
    for o in old_slots.iter().filter(|o| o.deep && o.starts_at <= now) {
        if !input.iter().any(|s| o.same_as(s)) {
            return Err(Reason::DeepSessionHasStarted);
        }
    }
    if input
        .iter()
        .any(|s| s.deep && s.starts_at < now && unchanged(s).is_none_or(|o| !o.deep))
    {
        return Err(Reason::SlotInThePast);
    }
    let slots: Vec<Slot> = input
        .iter()
        .map(|s| Slot {
            id: s.id.clone(),
            starts_at: s.starts_at,
            ends_at: s.ends_at,
            label: s.label.trim().to_string(),
            deep: s.deep,
            depth_rating: unchanged(s).and_then(|o| o.depth_rating),
        })
        .collect();
    let minutes = planned_deep_minutes(&slots);
    if minutes < old.map_or(0, |p| p.planned_deep_minutes) {
        return Err(Reason::SlotCannotBeRemoved);
    }
    let plan = DayPlan {
        date,
        starts_at,
        ends_at,
        planned_deep_minutes: minutes,
        slots,
    };
    match state.day_plans.iter_mut().find(|p| p.date == date) {
        Some(saved) => *saved = plan,
        None => {
            state.day_plans.push(plan);
            state.day_plans.sort_by_key(|p| p.date);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn put_schedule(
    state: &mut State,
    id: &str,
    time_zone: &str,
    weekdays: &[u8],
    starts: &str,
    ends: &str,
    input: &[ScheduleSlot],
    now: DateTime<Utc>,
) -> Result<(), Reason> {
    time_zone
        .parse::<Tz>()
        .map_err(|_| Reason::UnknownTimeZone)?;
    let days: BTreeSet<u8> = weekdays.iter().copied().collect();
    let (Some(day_starts), Some(day_ends)) = (parse_hhmm(starts), parse_hhmm(ends)) else {
        return Err(Reason::InvalidSchedule);
    };
    if days.len() != weekdays.len()
        || days.iter().any(|d| !(1..=7).contains(d))
        || day_ends <= day_starts
    {
        return Err(Reason::InvalidSchedule);
    }
    let mut slots = Vec::with_capacity(input.len());
    for s in input {
        let (Some(a), Some(b)) = (parse_hhmm(&s.starts), parse_hhmm(&s.ends)) else {
            return Err(Reason::InvalidSlot);
        };
        if s.label.trim().is_empty() || b <= a || a < day_starts || b > day_ends {
            return Err(Reason::InvalidSlot);
        }
        slots.push((a, s));
    }
    slots.sort_by_key(|(a, _)| *a);
    if slots
        .windows(2)
        .any(|w| w[1].0 < parse_hhmm(&w[0].1.ends).expect("checked"))
    {
        return Err(Reason::SlotsOverlap);
    }
    if state
        .schedules
        .iter()
        .any(|s| s.id != id && s.weekdays.iter().any(|d| days.contains(d)))
    {
        return Err(Reason::SchedulesOverlap);
    }
    // A change to a Schedule takes effect from its next day: a day already running on it
    // keeps its plan, which is saved as that day's Day Plan.
    if let Some(old) = state.schedules.iter().find(|s| s.id == id) {
        if let Ok(old_tz) = old.time_zone.parse::<Tz>() {
            let today = now.with_timezone(&old_tz).date_naive();
            let weekday = today.weekday().number_from_monday() as u8;
            if old.weekdays.contains(&weekday) && saved_plan(state, today).is_none() {
                if let Some(plan) = schedule_plan_of(old, today) {
                    if plan.starts_at <= now && now < plan.ends_at {
                        state.day_plans.push(plan);
                        state.day_plans.sort_by_key(|p| p.date);
                    }
                }
            }
        }
    }
    let schedule = Schedule {
        id: id.to_string(),
        time_zone: time_zone.to_string(),
        weekdays: days.into_iter().collect(),
        starts: starts.to_string(),
        ends: ends.to_string(),
        slots: slots
            .into_iter()
            .map(|(_, s)| ScheduleSlot {
                starts: s.starts.clone(),
                ends: s.ends.clone(),
                label: s.label.trim().to_string(),
                deep: s.deep,
            })
            .collect(),
    };
    match state.schedules.iter_mut().find(|s| s.id == id) {
        Some(saved) => *saved = schedule,
        None => {
            state.schedules.push(schedule);
            state.schedules.sort_by(|a, b| a.id.cmp(&b.id));
        }
    }
    Ok(())
}

/// A deep Slot of a saved Day Plan, by id (for its Depth Rating).
pub(crate) fn find_slot<'a>(state: &'a mut State, id: &str) -> Option<&'a mut Slot> {
    state
        .day_plans
        .iter_mut()
        .flat_map(|p| p.slots.iter_mut())
        .find(|s| s.id == id)
}
