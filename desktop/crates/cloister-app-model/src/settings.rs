//! The Settings screen: Workdays, Workday Start, Shutdown Time, the time zone (set once, with
//! the first settings) and the weekly Deep Hours target. Saving sends the request; the rules
//! decide what applies now (anything stricter) and what waits (looser settings until tomorrow,
//! a lower target until next week). Waiting changes are listed with when they take effect.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use cloister_core::{settings_at, weekly_target_at, Command, State, WEEKLY_TARGET_HOURS};

use crate::day_plan::PlanBackend;
use crate::format::{day, Zone};
use crate::notice::Notice;
use crate::ritual::DEFAULT_WEEKLY_TARGET;

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// "Mon, Tue, Wed".
pub fn weekday_names(days: &[u8]) -> String {
    days.iter()
        .filter_map(|d| WEEKDAYS.get(usize::from(*d).wrapping_sub(1)))
        .copied()
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Request {
    workdays: BTreeSet<u8>,
    workday_start: String,
    shutdown_time: String,
    time_zone: String,
    weekly_target: Option<u16>,
}

/// The Settings form, opened on the latest request (pending changes included).
pub struct SettingsForm {
    opened: Request,
    edit: Request,
    configured: bool,
}

impl SettingsForm {
    pub fn open(backend: &impl PlanBackend) -> Self {
        let state = backend.state().unwrap_or_default();
        let now = backend.server_time();
        let requested = state
            .pending_settings
            .as_ref()
            .filter(|p| p.effective_at > now)
            .map(|p| p.settings())
            .or_else(|| settings_at(&state, now));
        let configured = requested.is_some();
        let (workdays, workday_start, shutdown_time) = match requested {
            Some(s) => (s.workdays, s.workday_start, s.shutdown_time),
            None => (vec![1, 2, 3, 4, 5], "09:00".into(), "17:00".into()),
        };
        let weekly_target = state
            .pending_weekly_target
            .filter(|p| p.effective_at > now)
            .map(|p| p.hours)
            .or_else(|| weekly_target_at(&state, now));
        let opened = Request {
            workdays: workdays.into_iter().collect(),
            workday_start,
            shutdown_time,
            time_zone: state.time_zone.clone(),
            weekly_target,
        };
        SettingsForm {
            edit: opened.clone(),
            opened,
            configured,
        }
    }

    /// The time zone is chosen with the first settings and can't change after.
    pub fn time_zone_editable(&self) -> bool {
        !self.configured
    }

    pub fn workdays(&self) -> Vec<u8> {
        self.edit.workdays.iter().copied().collect()
    }

    pub fn is_workday(&self, day: u8) -> bool {
        self.edit.workdays.contains(&day)
    }

    pub fn toggle_workday(&mut self, day: u8) {
        if !self.edit.workdays.remove(&day) && (1..=7).contains(&day) {
            self.edit.workdays.insert(day);
        }
    }

    pub fn workday_start(&self) -> &str {
        &self.edit.workday_start
    }

    pub fn set_workday_start(&mut self, hhmm: &str) {
        self.edit.workday_start = hhmm.trim().to_string();
    }

    pub fn shutdown_time(&self) -> &str {
        &self.edit.shutdown_time
    }

    pub fn set_shutdown_time(&mut self, hhmm: &str) {
        self.edit.shutdown_time = hhmm.trim().to_string();
    }

    pub fn time_zone(&self) -> &str {
        &self.edit.time_zone
    }

    pub fn set_time_zone(&mut self, tz: &str) {
        if self.time_zone_editable() {
            self.edit.time_zone = tz.trim().to_string();
        }
    }

    /// The weekly target, in hours (20 until one is set).
    pub fn weekly_target(&self) -> u16 {
        self.edit.weekly_target.unwrap_or(DEFAULT_WEEKLY_TARGET)
    }

    pub fn set_weekly_target(&mut self, hours: u16) {
        self.edit.weekly_target =
            Some(hours.clamp(*WEEKLY_TARGET_HOURS.start(), *WEEKLY_TARGET_HOURS.end()));
    }

    /// Move a "HH:MM" field by `minutes` (15-minute steps on screen), within the day.
    pub fn nudge(hhmm: &str, minutes: i64) -> String {
        let Some((h, m)) = hhmm.split_once(':') else {
            return hhmm.to_string();
        };
        let (Ok(h), Ok(m)) = (h.parse::<i64>(), m.parse::<i64>()) else {
            return hhmm.to_string();
        };
        let t = (h * 60 + m + minutes).clamp(0, 23 * 60 + 59);
        format!("{:02}:{:02}", t / 60, t % 60)
    }

    /// Whether anything differs from what was opened.
    pub fn is_changed(&self) -> bool {
        self.edit != self.opened || (!self.configured && self.edit.weekly_target.is_none())
    }

    /// Send what changed. Returns what was recorded, or what is so.
    pub fn save(&mut self, backend: &impl PlanBackend) -> Notice {
        let settings_changed = !self.configured
            || self.edit.workdays != self.opened.workdays
            || self.edit.workday_start != self.opened.workday_start
            || self.edit.shutdown_time != self.opened.shutdown_time;
        let mut commands = Vec::new();
        if settings_changed {
            commands.push(Command::ChangeSettings {
                workdays: self.workdays(),
                workday_start: self.edit.workday_start.clone(),
                shutdown_time: self.edit.shutdown_time.clone(),
                time_zone: (!self.configured).then(|| self.edit.time_zone.clone()),
            });
        }
        if self.edit.weekly_target != self.opened.weekly_target
            || self.opened.weekly_target.is_none()
        {
            commands.push(Command::SetWeeklyTarget {
                hours: self.weekly_target(),
            });
        }
        for command in &commands {
            match backend.send(command) {
                Ok(None) => {}
                Ok(Some(code)) => return refusal(&code),
                Err(_) => {
                    return Notice::what_is_so(
                        "The backend can't be reached. Settings stay as they are.",
                    )
                }
            }
        }
        let waiting = !self.pending(backend, &Zone::Utc).is_empty();
        *self = SettingsForm::open(backend);
        Notice::recorded(if waiting {
            "Saved. Looser settings take effect later."
        } else {
            "Saved."
        })
    }

    /// Changes still waiting to take effect, with when.
    pub fn pending(&self, backend: &impl PlanBackend, zone: &Zone) -> Vec<String> {
        pending_changes(
            &backend.state().unwrap_or_default(),
            backend.server_time(),
            zone,
        )
    }
}

/// Loosening changes still waiting, with when they take effect.
pub fn pending_changes(state: &State, now: DateTime<Utc>, zone: &Zone) -> Vec<String> {
    let mut lines = Vec::new();
    if let (Some(p), Some(in_force)) = (
        state
            .pending_settings
            .as_ref()
            .filter(|p| p.effective_at > now),
        settings_at(state, now),
    ) {
        let when = day(p.effective_at, zone);
        if p.workdays != in_force.workdays {
            lines.push(format!(
                "Workdays {} take effect {when}.",
                weekday_names(&p.workdays)
            ));
        }
        if p.workday_start != in_force.workday_start {
            lines.push(format!(
                "Workday Start {} takes effect {when}.",
                p.workday_start
            ));
        }
        if p.shutdown_time != in_force.shutdown_time {
            lines.push(format!(
                "Shutdown Time {} takes effect {when}.",
                p.shutdown_time
            ));
        }
    }
    if let Some(p) = state.pending_weekly_target.filter(|p| p.effective_at > now) {
        lines.push(format!(
            "A weekly target of {} Deep Hours takes effect {}.",
            p.hours,
            day(p.effective_at, zone)
        ));
    }
    lines
}

fn refusal(code: &str) -> Notice {
    Notice::refused(code, &cloister_core::Details::default())
}
