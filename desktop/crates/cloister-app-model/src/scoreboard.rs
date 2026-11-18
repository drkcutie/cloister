//! The Deep Hours scoreboard: the week's Deep Hours filling a well toward the weekly target,
//! a bar per day, the depth trend, and the week's record (Missed Rituals, Shutdown Delays used,
//! Breaches). Figures come from the rules engine's `week_summary`. No percentages are ever
//! written, and there are no streaks.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use cloister_core::{week_summary, BreachKind, State};

use crate::format::{clock, hours, Zone};

/// One day's bar.
#[derive(Debug, Clone, PartialEq)]
pub struct DayBar {
    /// "Mon".
    pub label: String,
    pub date: NaiveDate,
    pub minutes: i64,
    /// "1h 30m".
    pub text: String,
    /// Height against the week's longest day, 0 to 1 (for drawing only).
    pub fill: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordKind {
    MissedRitual,
    ShutdownDelayUsed,
    Breach,
}

/// One line of the week's record, shown with a brass tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub kind: RecordKind,
    /// "Missed Ritual", "Shutdown Delay used", "Breach".
    pub tag: &'static str,
    /// "Tue 6 Jan." / "VPN turned off, Wed 7 Jan, 10:00."
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scoreboard {
    pub week_start: NaiveDate,
    pub deep_minutes: i64,
    pub target_hours: Option<u16>,
    pub next_week_target_hours: Option<u16>,
    /// "12.5 of 20 Deep Hours", or "12.5 Deep Hours" without a target.
    pub hours_text: String,
    /// How full the well is toward the target, 0 to 1 (for drawing only).
    pub well_fill: f32,
    pub days: Vec<DayBar>,
    /// Recent Depth Ratings, oldest first.
    pub depth_trend: Vec<u8>,
    pub records: Vec<Record>,
}

/// "12.5": hours with at most one decimal, none when whole.
pub fn deep_hours_figure(minutes: i64) -> String {
    let tenths = (minutes * 10 + 30) / 60;
    if tenths % 10 == 0 {
        format!("{}", tenths / 10)
    } else {
        format!("{}.{}", tenths / 10, tenths % 10)
    }
}

fn date_text(date: NaiveDate) -> String {
    date.format("%a %-d %b").to_string()
}

fn when(t: DateTime<Utc>, zone: &Zone) -> String {
    let date = match zone {
        Zone::Local => t.with_timezone(&chrono::Local).date_naive(),
        Zone::Utc => t.date_naive(),
    };
    format!("{}, {}", date_text(date), clock(t, zone))
}

/// The scoreboard for the week holding `now`.
pub fn scoreboard(state: &State, now: DateTime<Utc>, zone: &Zone) -> Scoreboard {
    let summary = week_summary(state, now);
    let longest = summary
        .deep_minutes_by_day
        .iter()
        .copied()
        .max()
        .unwrap_or(0);
    let days = summary
        .deep_minutes_by_day
        .iter()
        .enumerate()
        .map(|(i, &minutes)| {
            let date = summary.week_start + Duration::days(i as i64);
            DayBar {
                label: date.format("%a").to_string(),
                date,
                minutes,
                text: hours(Duration::minutes(minutes)),
                fill: if longest > 0 {
                    minutes as f32 / longest as f32
                } else {
                    0.0
                },
            }
        })
        .collect();
    let figure = deep_hours_figure(summary.deep_minutes);
    let (hours_text, well_fill) = match summary.target_hours {
        Some(t) => (
            format!("{figure} of {t} Deep Hours"),
            (summary.deep_minutes as f32 / (f32::from(t) * 60.0)).min(1.0),
        ),
        None => (format!("{figure} Deep Hours"), 0.0),
    };
    let mut records: Vec<Record> = summary
        .missed_rituals
        .iter()
        .map(|d| Record {
            kind: RecordKind::MissedRitual,
            tag: "Missed Ritual",
            text: format!("{}.", date_text(*d)),
        })
        .collect();
    records.extend(summary.shutdown_delays.iter().map(|d| Record {
        kind: RecordKind::ShutdownDelayUsed,
        tag: "Shutdown Delay used",
        text: format!("{}.", date_text(*d)),
    }));
    records.extend(summary.breaches.iter().map(|b| Record {
        kind: RecordKind::Breach,
        tag: "Breach",
        text: format!(
            "{}, {}.",
            match b.kind {
                BreachKind::VpnTurnedOff => "VPN turned off",
                BreachKind::AppBlockerTurnedOff => "App blocker turned off",
            },
            when(b.occurred_at, zone)
        ),
    }));
    Scoreboard {
        week_start: summary.week_start,
        deep_minutes: summary.deep_minutes,
        target_hours: summary.target_hours,
        next_week_target_hours: summary.next_week_target_hours,
        hours_text,
        well_fill,
        days,
        depth_trend: summary.depth_trend,
        records,
    }
}
