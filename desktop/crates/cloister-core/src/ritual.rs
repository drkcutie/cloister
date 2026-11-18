//! The Shutdown Ritual, Shutdown Delays, Missed Rituals and Loose Ends.
//!
//! Every Workday has a Shutdown Ritual. It becomes due at that day's Shutdown Time (30 minutes
//! later when a Shutdown Delay was used for it) and stays due until it is completed or the next
//! Workday Start, when it lapses as a Missed Ritual. Only completions are stored; a Missed
//! Ritual is a Workday whose ritual lapsed with no completion, so it is recorded by the
//! absence of one and every engine derives it the same way.
//!
//! A Shutdown Delay can be used once per ISO week (of the local date), on a Workday before its
//! Shutdown Time. It postpones both the ritual and Evening Lockdown by 30 minutes.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::settings::{lockdown, next_workday_start, workday, Workday};
use crate::{day_plan, Reason, RitualScreen, State};

/// How long a Shutdown Delay postpones the ritual and Evening Lockdown.
pub const SHUTDOWN_DELAY: Duration = Duration::minutes(30);
/// What must be typed, exactly, to complete the ritual.
pub const SHUTDOWN_COMPLETE: &str = "Shutdown complete";
/// The longest a Loose End may be, in characters.
pub const MAX_LOOSE_END: usize = 500;
/// How far back to look for the latest ritual that has become due.
const LOOKBACK_DAYS: i64 = 15;

/// An open task or commitment captured so it stays out of your head overnight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LooseEnd {
    pub id: String,
    pub text: String,
    pub captured_at: DateTime<Utc>,
}

/// A completed Shutdown Ritual, by the Workday it closes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ritual {
    pub date: NaiveDate,
    pub completed_at: DateTime<Utc>,
}

/// A used Shutdown Delay, by the Workday whose shutdown it postponed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShutdownDelay {
    pub date: NaiveDate,
    pub used_at: DateTime<Utc>,
}

/// The latest ritual that has become due by some time, and what it means then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RitualStatus {
    /// The Workday the ritual closes.
    pub date: NaiveDate,
    /// When it became due (Shutdown Time, plus a Shutdown Delay if one was used).
    pub due_at: DateTime<Utc>,
    /// The next Workday Start after `due_at`, when an incomplete ritual lapses. `None` if no
    /// Workday follows within two weeks.
    pub lapses_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

fn time_zone(state: &State) -> Tz {
    state.time_zone.parse().unwrap_or(Tz::UTC)
}

fn local_date(state: &State, at: DateTime<Utc>) -> NaiveDate {
    at.with_timezone(&time_zone(state)).date_naive()
}

/// The ISO week (year, week) of a local date.
fn iso_week(date: NaiveDate) -> (i32, u32) {
    let w = date.iso_week();
    (w.year(), w.week())
}

fn delay_for(state: &State, date: NaiveDate) -> Option<&ShutdownDelay> {
    state.shutdown_delays.iter().find(|d| d.date == date)
}

/// When the ritual of `date` becomes due, as seen at `now`. `None` if `date` isn't a Workday.
pub fn ritual_due_at(state: &State, date: NaiveDate, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    match workday(state, date, now) {
        Workday::Bounds(_, shutdown) => Some(
            shutdown
                + if delay_for(state, date).is_some() {
                    SHUTDOWN_DELAY
                } else {
                    Duration::zero()
                },
        ),
        _ => None,
    }
}

/// The latest ritual due at or before `now`, if any.
pub fn ritual(state: &State, now: DateTime<Utc>) -> Option<RitualStatus> {
    let today = local_date(state, now);
    (0..LOOKBACK_DAYS)
        .map(|n| today - Duration::days(n))
        .find_map(|date| {
            let due_at = ritual_due_at(state, date, now).filter(|due| *due <= now)?;
            Some(RitualStatus {
                date,
                due_at,
                lapses_at: next_workday_start(state, due_at),
                completed_at: state
                    .rituals
                    .iter()
                    .find(|r| r.date == date)
                    .map(|r| r.completed_at),
            })
        })
}

/// The ritual screen at `now`: `delayed` once today's Shutdown Delay is used until the ritual
/// is due; `due` from then until completed or lapsed; `completed` until the next Workday Start;
/// `missed` from a lapse until the next ritual is due.
pub fn ritual_screen(state: &State, now: DateTime<Utc>) -> RitualScreen {
    let today = local_date(state, now);
    if let (Some(delay), Some(due)) = (delay_for(state, today), ritual_due_at(state, today, now)) {
        if delay.used_at <= now && now < due {
            return RitualScreen::Delayed;
        }
    }
    let Some(status) = ritual(state, now) else {
        return RitualScreen::None;
    };
    let lapsed = status.lapses_at.is_some_and(|l| l <= now);
    match (status.completed_at.is_some(), lapsed) {
        (true, false) => RitualScreen::Completed,
        (true, true) => RitualScreen::None,
        (false, false) => RitualScreen::Due,
        (false, true) => RitualScreen::Missed,
    }
}

/// How many Shutdown Delays are left in the ISO week of `now` (local): 1 or 0.
pub fn shutdown_delays_left(state: &State, now: DateTime<Utc>) -> u8 {
    let week = iso_week(local_date(state, now));
    u8::from(
        !state
            .shutdown_delays
            .iter()
            .any(|d| iso_week(d.date) == week),
    )
}

/// Whether a Shutdown Delay holds back Evening Lockdown at local `t` on its Workday: from
/// Shutdown Time until 30 minutes later (local wall time, so it may run past midnight).
pub(crate) fn delay_holds_lockdown(state: &State, t: NaiveDateTime, shutdown: NaiveTime) -> bool {
    let date = t.date();
    delay_for(state, date).is_some() && t < date.and_time(shutdown) + SHUTDOWN_DELAY
}

pub(crate) fn capture_loose_end(
    next: &mut State,
    id: &str,
    text: &str,
    now: DateTime<Utc>,
) -> Result<(), Reason> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > MAX_LOOSE_END || id.trim().is_empty() {
        return Err(Reason::InvalidLooseEnd);
    }
    if next.loose_ends.iter().any(|l| l.id == id) {
        return Err(Reason::DuplicateLooseEnd);
    }
    next.loose_ends.push(LooseEnd {
        id: id.to_string(),
        text: text.to_string(),
        captured_at: now,
    });
    next.loose_ends
        .sort_by(|a, b| (a.captured_at, &a.id).cmp(&(b.captured_at, &b.id)));
    Ok(())
}

pub(crate) fn close_loose_end(next: &mut State, id: &str) -> Result<(), Reason> {
    let before = next.loose_ends.len();
    next.loose_ends.retain(|l| l.id != id);
    if next.loose_ends.len() == before {
        return Err(Reason::UnknownLooseEnd);
    }
    Ok(())
}

pub(crate) fn use_shutdown_delay(next: &mut State, now: DateTime<Utc>) -> Result<(), Reason> {
    if crate::settings_at(next, now).is_none() {
        return Err(Reason::NoShutdownTime);
    }
    if let Some(lockdown) = lockdown(next, now) {
        return Err(lockdown.reason());
    }
    if shutdown_delays_left(next, now) == 0 {
        return Err(Reason::ShutdownDelayUsed);
    }
    next.shutdown_delays.push(ShutdownDelay {
        date: local_date(next, now),
        used_at: now,
    });
    next.shutdown_delays.sort_by_key(|d| d.date);
    Ok(())
}

pub(crate) fn complete_ritual(
    next: &mut State,
    typed: &str,
    now: DateTime<Utc>,
) -> Result<(), Reason> {
    let status = ritual(next, now).ok_or(Reason::RitualNotDue)?;
    if status.completed_at.is_some() {
        return Err(Reason::RitualAlreadyCompleted);
    }
    if status.lapses_at.is_some_and(|l| l <= now) {
        return Err(Reason::RitualNotDue);
    }
    if !next
        .loose_ends
        .iter()
        .any(|l| l.captured_at >= status.due_at)
    {
        return Err(Reason::LooseEndRequired);
    }
    if let Some(start) = status.lapses_at {
        tomorrow_is_planned(next, local_date(next, start))?;
    }
    if typed != SHUTDOWN_COMPLETE {
        return Err(Reason::ConfirmationMismatch);
    }
    next.rituals.push(Ritual {
        date: status.date,
        completed_at: now,
    });
    next.rituals.sort_by_key(|r| r.date);
    Ok(())
}

/// The next Workday has a saved Day Plan with a deep Slot, whose Slots cover it end to end.
fn tomorrow_is_planned(state: &State, date: NaiveDate) -> Result<(), Reason> {
    let plan = state
        .day_plans
        .iter()
        .find(|p| p.date == date)
        .ok_or(Reason::DayPlanRequired)?;
    if !plan.slots.iter().any(|s| s.deep) {
        return Err(Reason::DeepSlotRequired);
    }
    let mut reached = plan.starts_at;
    for slot in &plan.slots {
        if slot.starts_at != reached {
            return Err(Reason::WorkdayNotCovered);
        }
        reached = slot.ends_at;
    }
    if reached != plan.ends_at {
        return Err(Reason::WorkdayNotCovered);
    }
    Ok(())
}

/// Tomorrow's draft for the ritual's planning steps: the Day Plan of the Workday after the
/// ritual due at `now` (saved, or filled from its Schedule).
pub fn next_workday_plan(state: &State, now: DateTime<Utc>) -> Option<crate::DayPlan> {
    let start = ritual(state, now)
        .and_then(|r| r.lapses_at)
        .or_else(|| next_workday_start(state, now))?;
    day_plan(state, local_date(state, start))
}

impl State {
    /// Fold in Shutdown Delays and completed rituals from the backend at `now`, for an
    /// enforcement service that holds its own state. Both lift enforcement, so each is taken
    /// only where the rules could have produced it: a delay for a Workday, used before its
    /// Shutdown Time, in an ISO week with no other delay; a completion of a ritual that had
    /// become due, with the next Workday planned (deep Slot, whole Workday covered) in this
    /// state's Day Plans. Known records are never dropped. Returns whether anything changed.
    pub fn merge_ritual_records(
        &mut self,
        delays: &[ShutdownDelay],
        rituals: &[Ritual],
        now: DateTime<Utc>,
    ) -> bool {
        let mut changed = false;
        for delay in delays {
            if delay_for(self, delay.date).is_some() {
                continue;
            }
            let week = iso_week(delay.date);
            let week_free = !self
                .shutdown_delays
                .iter()
                .any(|d| iso_week(d.date) == week);
            let before_shutdown = matches!(
                workday(self, delay.date, now),
                Workday::Bounds(_, shutdown) if delay.used_at < shutdown && delay.used_at <= now
            );
            if week_free && before_shutdown {
                self.shutdown_delays.push(delay.clone());
                self.shutdown_delays.sort_by_key(|d| d.date);
                changed = true;
            }
        }
        for record in rituals {
            if self.rituals.iter().any(|r| r.date == record.date) {
                continue;
            }
            let Some(due_at) = ritual_due_at(self, record.date, now) else {
                continue;
            };
            if record.completed_at < due_at || due_at > now {
                continue;
            }
            let planned = next_workday_start(self, due_at)
                .is_none_or(|start| tomorrow_is_planned(self, local_date(self, start)).is_ok());
            if planned {
                self.rituals.push(record.clone());
                self.rituals.sort_by_key(|r| r.date);
                changed = true;
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weekdays() -> State {
        serde_json::from_str(
            r#"{"blocks": [], "settings": {"workdays": [1,2,3,4,5], "workday_start": "09:00", "shutdown_time": "17:00"}}"#,
        )
        .unwrap()
    }

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn a_synced_delay_is_taken_once_per_week_and_only_before_shutdown() {
        let mut state = weekdays();
        let now = at("2026-01-05T16:10:00Z");
        let late = ShutdownDelay {
            date: "2026-01-05".parse().unwrap(),
            used_at: at("2026-01-05T17:05:00Z"),
        };
        assert!(!state.merge_ritual_records(&[late], &[], at("2026-01-05T17:06:00Z")));
        let ok = ShutdownDelay {
            date: "2026-01-05".parse().unwrap(),
            used_at: at("2026-01-05T16:00:00Z"),
        };
        let again = ShutdownDelay {
            date: "2026-01-07".parse().unwrap(),
            used_at: at("2026-01-07T10:00:00Z"),
        };
        assert!(state.merge_ritual_records(&[ok.clone(), again], &[], at("2026-01-07T11:00:00Z")));
        assert_eq!(state.shutdown_delays, vec![ok]);
        assert_eq!(ritual_screen(&state, now), RitualScreen::Delayed);
    }

    #[test]
    fn a_synced_completion_needs_the_next_workday_planned_here() {
        let mut state = weekdays();
        let now = at("2026-01-05T17:20:00Z");
        let done = Ritual {
            date: "2026-01-05".parse().unwrap(),
            completed_at: at("2026-01-05T17:15:00Z"),
        };
        assert!(!state.merge_ritual_records(&[], &[done.clone()], now));
        assert_eq!(ritual_screen(&state, now), RitualScreen::Due);
        let plan = crate::Command::WriteDayPlan {
            date: "2026-01-06".parse().unwrap(),
            starts_at: at("2026-01-06T09:00:00Z"),
            ends_at: at("2026-01-06T17:00:00Z"),
            slots: vec![crate::SlotInput {
                id: "t1".into(),
                starts_at: at("2026-01-06T09:00:00Z"),
                ends_at: at("2026-01-06T17:00:00Z"),
                label: "Write".into(),
                deep: true,
            }],
        };
        state = crate::decide(&state, &plan, now).state;
        assert!(state.merge_ritual_records(&[], &[done], now));
        assert_eq!(ritual_screen(&state, now), RitualScreen::Completed);
    }
}
