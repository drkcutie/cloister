//! The weekly Deep Hours target, the Weekly Review and the scoreboard.
//!
//! The target follows **tighten now, loosen tomorrow** at the scale of a week: raising it applies
//! at once, lowering it waits for the next week boundary (local Monday midnight). The Weekly
//! Review is an extra required step of the last Workday's Shutdown Ritual: it shows the week and
//! sets next week's target.
//!
//! [`week_summary`] is the scoreboard: Deep Hours per day against the target, the depth trend,
//! and the week's Missed Rituals, Shutdown Delays and Breaches. It is pinned by the shared
//! scenarios in `fixtures/review`.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::settings::{workday, Workday};
use crate::{ritual, Breach, Reason, State};

/// The lowest and highest weekly Deep Hours target.
pub const WEEKLY_TARGET_HOURS: std::ops::RangeInclusive<u16> = 1..=100;
/// How many recent Depth Ratings the depth trend shows.
pub const DEPTH_TREND_LENGTH: usize = 10;

/// The weekly Deep Hours target in force (as of the last change).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklyTarget {
    pub hours: u16,
}

/// A lower target, in force from `effective_at` (the next week boundary when it was set).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingWeeklyTarget {
    pub hours: u16,
    pub effective_at: DateTime<Utc>,
}

fn time_zone(state: &State) -> Tz {
    state.time_zone.parse().unwrap_or(Tz::UTC)
}

fn local_date(state: &State, at: DateTime<Utc>) -> NaiveDate {
    at.with_timezone(&time_zone(state)).date_naive()
}

/// The start of local `date` in the account's time zone (a skipped midnight maps to the first
/// instant after it).
fn day_start(state: &State, date: NaiveDate) -> DateTime<Utc> {
    let tz = time_zone(state);
    let midnight = date.and_time(NaiveTime::MIN);
    match tz.from_local_datetime(&midnight).earliest() {
        Some(t) => t.with_timezone(&Utc),
        None => tz
            .from_local_datetime(&(midnight - Duration::hours(1)))
            .earliest()
            .map(|t| t.with_timezone(&Utc) + Duration::hours(1))
            .unwrap_or_else(|| midnight.and_utc()),
    }
}

/// The Monday of the ISO week holding `date`.
pub fn week_start(date: NaiveDate) -> NaiveDate {
    date - Duration::days(i64::from(date.weekday().num_days_from_monday()))
}

/// The first local Monday midnight after `now`.
pub fn next_week_boundary(state: &State, now: DateTime<Utc>) -> DateTime<Utc> {
    day_start(
        state,
        week_start(local_date(state, now)) + Duration::days(7),
    )
}

/// The weekly target in force at `at`: the pending one once its week has come.
pub fn weekly_target_at(state: &State, at: DateTime<Utc>) -> Option<u16> {
    match &state.pending_weekly_target {
        Some(p) if p.effective_at <= at => Some(p.hours),
        _ => state.weekly_target.map(|t| t.hours),
    }
}

pub(crate) fn set_weekly_target(
    next: &mut State,
    hours: u16,
    now: DateTime<Utc>,
) -> Result<(), Reason> {
    if !WEEKLY_TARGET_HOURS.contains(&hours) {
        return Err(Reason::InvalidWeeklyTarget);
    }
    let in_force = weekly_target_at(next, now).map_or(hours, |base| base.max(hours));
    next.weekly_target = Some(WeeklyTarget { hours: in_force });
    next.pending_weekly_target = (in_force != hours).then(|| PendingWeeklyTarget {
        hours,
        effective_at: next_week_boundary(next, now),
    });
    Ok(())
}

/// Whether `date` is the last Workday of its ISO week (no later day of that week is a Workday),
/// as seen at `now`. Its ritual holds the Weekly Review.
pub fn is_last_workday_of_week(state: &State, date: NaiveDate, now: DateTime<Utc>) -> bool {
    let sunday = week_start(date) + Duration::days(6);
    date.iter_days()
        .skip(1)
        .take_while(|d| *d <= sunday)
        .all(|d| !matches!(workday(state, d, now), Workday::Bounds(..)))
}

/// The Weekly Review part of `complete_ritual` for the ritual of `date`: required (with next
/// week's target) on the last Workday of the week, refused on other days.
pub(crate) fn check_weekly_review(
    state: &State,
    date: NaiveDate,
    weekly_target: Option<u16>,
    now: DateTime<Utc>,
) -> Result<(), Reason> {
    match (is_last_workday_of_week(state, date, now), weekly_target) {
        (true, None) => Err(Reason::WeeklyReviewRequired),
        (false, Some(_)) => Err(Reason::WeeklyReviewNotDue),
        (true, Some(h)) if !WEEKLY_TARGET_HOURS.contains(&h) => Err(Reason::InvalidWeeklyTarget),
        _ => Ok(()),
    }
}

/// The scoreboard and Weekly Review for one ISO week. Minutes, not percentages; no streaks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeekSummary {
    /// The Monday the week starts on (local).
    pub week_start: NaiveDate,
    /// The target in force at the time asked (hours), if any.
    pub target_hours: Option<u16>,
    /// The target that will be in force next week (hours), if any.
    pub next_week_target_hours: Option<u16>,
    /// Deep minutes per local day, Monday first (7 entries). Only time already worked counts.
    pub deep_minutes_by_day: Vec<i64>,
    /// The sum of `deep_minutes_by_day`.
    pub deep_minutes: i64,
    /// Depth Ratings of the latest rated Deep Sessions up to the time asked (any week),
    /// oldest first, at most [`DEPTH_TREND_LENGTH`].
    pub depth_trend: Vec<u8>,
    /// Workdays of this week whose ritual lapsed with no completion.
    pub missed_rituals: Vec<NaiveDate>,
    /// Workdays of this week a Shutdown Delay was used on.
    pub shutdown_delays: Vec<NaiveDate>,
    /// Breaches recorded this week.
    pub breaches: Vec<Breach>,
}

/// One Deep Session as the scoreboard sees it.
struct DeepSession {
    id: String,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    depth_rating: Option<u8>,
}

/// Every Deep Session known around `[from, to)`: deep Blocks, and deep Slots of saved or
/// Schedule-filled Day Plans of those days. Sorted by start; an id counts once.
fn deep_sessions(state: &State, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<DeepSession> {
    let mut sessions: Vec<DeepSession> = state
        .blocks
        .iter()
        .filter(|b| b.deep)
        .map(|b| DeepSession {
            id: b.id.clone(),
            starts_at: b.starts_at,
            ends_at: b.ends_at,
            depth_rating: b.depth_rating,
        })
        .collect();
    let first = local_date(state, from) - Duration::days(1);
    let last = local_date(state, to);
    for date in first.iter_days().take_while(|d| *d <= last) {
        if let Some(plan) = crate::day_plan(state, date) {
            sessions.extend(plan.slots.iter().filter(|s| s.deep).map(|s| DeepSession {
                id: s.id.clone(),
                starts_at: s.starts_at,
                ends_at: s.ends_at,
                depth_rating: s.depth_rating,
            }));
        }
    }
    sessions.sort_by(|a, b| (a.starts_at, &a.id).cmp(&(b.starts_at, &b.id)));
    sessions.dedup_by(|a, b| a.id == b.id);
    sessions
}

/// The scoreboard for the ISO week (local) holding `now`, counting time up to `now`.
pub fn week_summary(state: &State, now: DateTime<Utc>) -> WeekSummary {
    let monday = week_start(local_date(state, now));
    let days: Vec<NaiveDate> = (0..7).map(|n| monday + Duration::days(n)).collect();
    let bounds: Vec<DateTime<Utc>> = days
        .iter()
        .map(|d| day_start(state, *d))
        .chain(std::iter::once(day_start(
            state,
            monday + Duration::days(7),
        )))
        .collect();
    let week_from = bounds[0];
    let week_to = bounds[7];
    let sessions = deep_sessions(state, week_from, week_to);
    let deep_minutes_by_day: Vec<i64> = (0..7)
        .map(|i| {
            let (from, to) = (bounds[i], bounds[i + 1].min(now));
            sessions
                .iter()
                .map(|s| {
                    (s.ends_at.min(to) - s.starts_at.max(from))
                        .num_minutes()
                        .max(0)
                })
                .sum()
        })
        .collect();
    let since = now - Duration::days(30);
    let mut trend: Vec<u8> = deep_sessions(state, since, now)
        .iter()
        .filter(|s| since <= s.starts_at && s.starts_at <= now)
        .filter_map(|s| s.depth_rating)
        .collect();
    if trend.len() > DEPTH_TREND_LENGTH {
        trend.drain(..trend.len() - DEPTH_TREND_LENGTH);
    }
    let missed_rituals = days
        .iter()
        .copied()
        .filter(|d| is_missed(state, *d, now))
        .collect();
    WeekSummary {
        week_start: monday,
        target_hours: weekly_target_at(state, now),
        next_week_target_hours: weekly_target_at(state, week_to),
        deep_minutes: deep_minutes_by_day.iter().sum(),
        deep_minutes_by_day,
        depth_trend: trend,
        missed_rituals,
        shutdown_delays: state
            .shutdown_delays
            .iter()
            .filter(|d| days.contains(&d.date))
            .map(|d| d.date)
            .collect(),
        breaches: state
            .breaches
            .iter()
            .filter(|b| week_from <= b.occurred_at && b.occurred_at < week_to)
            .cloned()
            .collect(),
    }
}

/// Whether the ritual of `date` lapsed, by `now`, with no completion.
fn is_missed(state: &State, date: NaiveDate, now: DateTime<Utc>) -> bool {
    let Some(due) = crate::ritual_due_at(state, date, now) else {
        return false;
    };
    if due > now || state.rituals.iter().any(|r| r.date == date) {
        return false;
    }
    ritual::lapses_at(state, due).is_some_and(|l| l <= now)
}
