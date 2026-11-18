//! Workdays, Workday Start and Shutdown Time, and the Evening Lockdown they define.
//!
//! Evening Lockdown runs on the clock alone: from Shutdown Time until the next Workday Start,
//! and all day on a Rest Day (a day that isn't a Workday). During it the Work List is blocked
//! as well as the Distraction List, and no Deep Session can start.
//!
//! Settings follow **tighten now, loosen tomorrow** (ADR 0001). A change is split per field:
//! the stricter part (fewer Workdays, a later Workday Start, an earlier Shutdown Time) applies
//! at once, and the requested settings as a whole wait in `pending_settings` until the next
//! day boundary.

use std::collections::BTreeSet;

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::day_plan::{local_instant, parse_hhmm};
use crate::{next_day_boundary, Reason, State};

/// The account's Workdays and Workday bounds. Times are local "HH:MM" in the state's
/// `time_zone`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// ISO weekdays (1 = Monday ... 7 = Sunday), sorted. Every other day is a Rest Day.
    pub workdays: Vec<u8>,
    pub workday_start: String,
    pub shutdown_time: String,
}

/// Requested settings that loosen something, in force from `effective_at`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingSettings {
    pub workdays: Vec<u8>,
    pub workday_start: String,
    pub shutdown_time: String,
    pub effective_at: DateTime<Utc>,
}

impl PendingSettings {
    pub fn settings(&self) -> Settings {
        Settings {
            workdays: self.workdays.clone(),
            workday_start: self.workday_start.clone(),
            shutdown_time: self.shutdown_time.clone(),
        }
    }

    fn new(settings: Settings, effective_at: DateTime<Utc>) -> Self {
        PendingSettings {
            workdays: settings.workdays,
            workday_start: settings.workday_start,
            shutdown_time: settings.shutdown_time,
            effective_at,
        }
    }
}

/// Settings that were in force until `until` (the history `workday` reads for days that have
/// begun). Sorted by `until`; each one covers the time from the previous one's `until`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PastSettings {
    pub workdays: Vec<u8>,
    pub workday_start: String,
    pub shutdown_time: String,
    pub until: DateTime<Utc>,
}

impl PastSettings {
    pub fn settings(&self) -> Settings {
        Settings {
            workdays: self.workdays.clone(),
            workday_start: self.workday_start.clone(),
            shutdown_time: self.shutdown_time.clone(),
        }
    }
}

impl Settings {
    /// The stricter of two settings, field by field: Workdays in both, the later Workday Start
    /// and the earlier Shutdown Time. If those times would leave no working time (Workday
    /// Start at or after Shutdown Time), `self`'s times are kept, so a Workday always has a
    /// Shutdown Ritual that lapses at the next Workday Start.
    pub fn tightest(&self, other: &Settings) -> Settings {
        let workday_start = self.workday_start.clone().max(other.workday_start.clone());
        let shutdown_time = self.shutdown_time.clone().min(other.shutdown_time.clone());
        let (workday_start, shutdown_time) = if workday_start < shutdown_time {
            (workday_start, shutdown_time)
        } else {
            (self.workday_start.clone(), self.shutdown_time.clone())
        };
        Settings {
            workdays: self
                .workdays
                .iter()
                .filter(|d| other.workdays.contains(d))
                .copied()
                .collect(),
            workday_start,
            shutdown_time,
        }
    }

    pub fn is_workday(&self, date: NaiveDate) -> bool {
        self.workdays
            .contains(&(date.weekday().number_from_monday() as u8))
    }

    fn times(&self) -> (NaiveTime, NaiveTime) {
        // Validated on the way in; a broken value locks the whole day rather than freeing it.
        (
            parse_hhmm(&self.workday_start).unwrap_or(NaiveTime::MIN),
            parse_hhmm(&self.shutdown_time).unwrap_or(NaiveTime::MIN),
        )
    }
}

/// Why the clock says no work now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lockdown {
    /// Between Shutdown Time and the next Workday Start on a Workday.
    EveningLockdown,
    /// A day that isn't a Workday.
    RestDay,
}

impl Lockdown {
    /// The rejection a Deep Session start gets now.
    pub fn reason(self) -> Reason {
        match self {
            Lockdown::EveningLockdown => Reason::EveningLockdown,
            Lockdown::RestDay => Reason::RestDay,
        }
    }
}

fn time_zone(state: &State) -> Tz {
    state.time_zone.parse().unwrap_or(Tz::UTC)
}

/// The settings in force at `at`: from the history for a time before the last change, else the
/// pending ones once their day has come.
pub fn settings_at(state: &State, at: DateTime<Utc>) -> Option<Settings> {
    if let Some(past) = state.past_settings.iter().find(|p| at < p.until) {
        return Some(past.settings());
    }
    match &state.pending_settings {
        Some(p) if p.effective_at <= at => Some(p.settings()),
        _ => state.settings.clone(),
    }
}

/// Whether the clock locks work at `at`, and why. `None` before any settings exist.
pub fn lockdown(state: &State, at: DateTime<Utc>) -> Option<Lockdown> {
    let settings = settings_at(state, at)?;
    let local = at.with_timezone(&time_zone(state));
    if !settings.is_workday(local.date_naive()) {
        return Some(Lockdown::RestDay);
    }
    let (start, shutdown) = settings.times();
    let time = local.time();
    let after_shutdown = time >= shutdown
        && !crate::ritual::delay_holds_lockdown(state, local.naive_local(), shutdown);
    (time < start || after_shutdown).then_some(Lockdown::EveningLockdown)
}

/// The start of local `date` in the account's time zone.
fn day_start(state: &State, date: NaiveDate) -> DateTime<Utc> {
    local_instant(time_zone(state), date, NaiveTime::MIN)
        .unwrap_or_else(|| date.and_time(NaiveTime::MIN).and_utc())
}

/// What the settings say about local `date`, as seen at `now`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workday {
    /// No settings yet: a Day Plan brings its own bounds.
    NotConfigured,
    RestDay,
    /// Workday Start to Shutdown Time.
    Bounds(DateTime<Utc>, DateTime<Utc>),
}

/// The Workday bounds of `date`. A day still to come uses the settings in force when it
/// begins. A day that has begun keeps the Workday it was when it began (removing a Workday
/// never erases that day's ritual or a past Missed Ritual); today takes the settings in force
/// now if they keep it a Workday, so a later Workday Start or earlier Shutdown Time applies at
/// once.
pub fn workday(state: &State, date: NaiveDate, now: DateTime<Utc>) -> Workday {
    let begins = day_start(state, date);
    let Some(current) = settings_at(state, now.max(begins)) else {
        return Workday::NotConfigured;
    };
    let today = now.with_timezone(&time_zone(state)).date_naive() == date;
    let settings = if begins > now || (today && current.is_workday(date)) {
        current
    } else {
        settings_at(state, begins).unwrap_or(current)
    };
    if !settings.is_workday(date) {
        return Workday::RestDay;
    }
    let tz = time_zone(state);
    let (start, shutdown) = settings.times();
    match (
        local_instant(tz, date, start),
        local_instant(tz, date, shutdown),
    ) {
        (Some(a), Some(b)) => Workday::Bounds(a, b),
        _ => Workday::RestDay,
    }
}

/// When work is next allowed after `now`: the next Workday Start that ends a lockdown.
/// `None` without settings, or when no day in the coming two weeks is a Workday.
pub fn next_workday_start(state: &State, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let today = now.with_timezone(&time_zone(state)).date_naive();
    (0..15)
        .map(|n| today + Duration::days(n))
        .filter_map(|date| match workday(state, date, now) {
            Workday::Bounds(start, end) if start < end && start > now => Some(start),
            _ => None,
        })
        .next()
}

/// Validate and normalize requested settings.
fn checked(workdays: &[u8], workday_start: &str, shutdown_time: &str) -> Result<Settings, Reason> {
    let days: BTreeSet<u8> = workdays.iter().copied().collect();
    let (Some(start), Some(shutdown)) = (parse_hhmm(workday_start), parse_hhmm(shutdown_time))
    else {
        return Err(Reason::InvalidSettings);
    };
    if days.len() != workdays.len()
        || days.iter().any(|d| !(1..=7).contains(d))
        || shutdown <= start
    {
        return Err(Reason::InvalidSettings);
    }
    Ok(Settings {
        workdays: days.into_iter().collect(),
        workday_start: workday_start.to_string(),
        shutdown_time: shutdown_time.to_string(),
    })
}

pub(crate) fn change_settings(
    next: &mut State,
    workdays: &[u8],
    workday_start: &str,
    shutdown_time: &str,
    time_zone: Option<&str>,
    now: DateTime<Utc>,
) -> Result<(), Reason> {
    if let Some(tz) = time_zone {
        tz.parse::<Tz>().map_err(|_| Reason::UnknownTimeZone)?;
    }
    let requested = checked(workdays, workday_start, shutdown_time)?;
    let configured = next.settings.is_some() || next.pending_settings.is_some();
    if let Some(tz) = time_zone {
        if tz != next.time_zone {
            if configured {
                return Err(Reason::TimeZoneCannotChange);
            }
            next.time_zone = tz.to_string();
        }
    }
    let Some(base) = settings_at(next, now) else {
        next.settings = Some(requested);
        next.pending_settings = None;
        next.settings_since.get_or_insert(now);
        return Ok(());
    };
    let in_force = base.tightest(&requested);
    next.record_settings_change(&in_force, now);
    next.pending_settings = (in_force != requested)
        .then(|| PendingSettings::new(requested, next_day_boundary(next, now)));
    next.settings = Some(in_force);
    Ok(())
}

impl State {
    /// Keep the settings history before `in_force` replaces the settings at `now`: the stored
    /// settings until pending ones took over, and the settings in force until now.
    fn record_settings_change(&mut self, in_force: &Settings, now: DateTime<Utc>) {
        let push = |past: &mut Vec<PastSettings>, s: Settings, until: DateTime<Utc>| {
            if past.last().is_some_and(|p| p.until >= until) {
                return;
            }
            past.push(PastSettings {
                workdays: s.workdays,
                workday_start: s.workday_start,
                shutdown_time: s.shutdown_time,
                until,
            });
        };
        if let (Some(old), Some(p)) = (&self.settings, &self.pending_settings) {
            if p.effective_at <= now && *old != p.settings() {
                push(&mut self.past_settings, old.clone(), p.effective_at);
            }
        }
        if let Some(base) = settings_at(self, now) {
            if base != *in_force {
                push(&mut self.past_settings, base, now);
            }
        }
    }

    /// Fold in when the account's settings began, before [`State::merge_settings`]. Only
    /// moves earlier (more rituals count); taken as-is while this state has no settings yet.
    /// Unknown (`None`) here, with settings, already counts every Workday and stays.
    pub fn merge_settings_since(&mut self, since: Option<DateTime<Utc>>) {
        let configured = self.settings.is_some() || self.pending_settings.is_some();
        self.settings_since = match (self.settings_since, since) {
            _ if !configured => since.or(self.settings_since),
            (Some(ours), Some(theirs)) => Some(ours.min(theirs)),
            (ours, _) => ours,
        };
    }

    /// Fold in settings from a sender that may not be trusted, at `now`: only their stricter
    /// part applies, at once. Nothing is scheduled to loosen (pending settings here stay as
    /// they are, and incoming ones only say which settings the sender has in force now). The
    /// time zone and the settings themselves are taken only while this state has none.
    pub fn tighten_settings(
        &mut self,
        time_zone: &str,
        settings: Option<&Settings>,
        pending: Option<&PendingSettings>,
        now: DateTime<Utc>,
    ) {
        let incoming = State {
            time_zone: time_zone.to_string(),
            settings: settings.cloned(),
            pending_settings: pending.cloned(),
            ..State::default()
        };
        let Some(incoming_now) = settings_at(&incoming, now) else {
            return;
        };
        if checked(
            &incoming_now.workdays,
            &incoming_now.workday_start,
            &incoming_now.shutdown_time,
        )
        .is_err()
        {
            return;
        }
        let Some(base) = settings_at(self, now) else {
            if self.settings.is_none() && self.pending_settings.is_none() {
                if time_zone.parse::<Tz>().is_ok() {
                    self.time_zone = time_zone.to_string();
                }
                self.settings = Some(incoming_now);
            }
            return;
        };
        let in_force = base.tightest(&incoming_now);
        if in_force == base {
            return;
        }
        self.record_settings_change(&in_force, now);
        if self
            .pending_settings
            .as_ref()
            .is_some_and(|p| p.effective_at <= now)
        {
            self.pending_settings = None;
        }
        self.settings = Some(in_force);
    }

    /// Fold in settings from the backend at `now` so that nothing loosens sooner than tomorrow
    /// from here. The time zone is taken only with the first settings.
    pub fn merge_settings(
        &mut self,
        time_zone: &str,
        settings: Option<&Settings>,
        pending: Option<&PendingSettings>,
        now: DateTime<Utc>,
    ) {
        let incoming = State {
            time_zone: time_zone.to_string(),
            settings: settings.cloned(),
            pending_settings: pending.cloned(),
            ..State::default()
        };
        let Some(incoming_now) = settings_at(&incoming, now) else {
            return; // The backend never drops settings; keep ours.
        };
        let target = match pending {
            Some(p) if p.effective_at > now => p.settings(),
            _ => incoming_now.clone(),
        };
        let Some(base) = settings_at(self, now) else {
            if time_zone.parse::<Tz>().is_ok() {
                self.time_zone = time_zone.to_string();
            }
            self.settings = Some(incoming_now);
            self.pending_settings = pending.filter(|p| p.effective_at > now).cloned();
            return;
        };
        let in_force = base.tightest(&incoming_now);
        let boundary = next_day_boundary(self, now);
        self.pending_settings = (in_force != target).then(|| {
            let effective_at = match &self.pending_settings {
                Some(known) if known.settings() == target && known.effective_at > now => {
                    known.effective_at
                }
                _ => pending
                    .filter(|p| p.effective_at > now)
                    .map_or(boundary, |p| p.effective_at.max(boundary)),
            };
            PendingSettings::new(target, effective_at)
        });
        self.settings = Some(in_force);
    }
}
