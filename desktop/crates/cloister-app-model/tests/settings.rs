//! The Settings screen through its public interface: Workdays, Workday Start, Shutdown Time,
//! time zone and the weekly target, and the loosening changes still waiting to take effect.

use std::cell::RefCell;

use chrono::{DateTime, Utc};
use cloister_app_model::{NoticeKind, PlanBackend, SettingsForm, Zone};
use cloister_core::{decide, Command, Outcome, State};

struct Backend {
    state: RefCell<State>,
    now: RefCell<DateTime<Utc>>,
}

impl Backend {
    fn at(now: &str) -> Self {
        Backend {
            state: RefCell::default(),
            now: RefCell::new(now.parse().unwrap()),
        }
    }

    fn advance_to(&self, now: &str) {
        *self.now.borrow_mut() = now.parse().unwrap();
    }
}

impl PlanBackend for Backend {
    fn state(&self) -> Result<State, String> {
        Ok(self.state.borrow().clone())
    }

    fn server_time(&self) -> DateTime<Utc> {
        *self.now.borrow()
    }

    fn send(&self, command: &Command) -> Result<Option<String>, String> {
        let decision = decide(&self.state.borrow(), command, self.server_time());
        *self.state.borrow_mut() = decision.state;
        Ok(match decision.outcome {
            Outcome::Accepted => None,
            Outcome::Rejected { reason } => Some(reason.code().to_string()),
        })
    }
}

#[test]
fn first_settings_apply_at_once_with_their_time_zone() {
    let backend = Backend::at("2026-01-05T10:00:00Z");
    let mut form = SettingsForm::open(&backend);
    assert!(form.time_zone_editable());
    assert_eq!(form.workdays(), vec![1, 2, 3, 4, 5]);
    form.set_time_zone("Europe/Paris");
    form.toggle_workday(5);
    form.set_shutdown_time("16:30");
    form.set_weekly_target(18);
    let saved = form.save(&backend);
    assert_eq!(saved.kind, NoticeKind::Recorded);
    let state = backend.state().unwrap();
    assert_eq!(state.time_zone, "Europe/Paris");
    let settings = state.settings.unwrap();
    assert_eq!(settings.workdays, vec![1, 2, 3, 4]);
    assert_eq!(settings.shutdown_time, "16:30");
    assert_eq!(state.weekly_target.unwrap().hours, 18);
    assert!(form.pending(&backend, &Zone::Utc).is_empty());
    assert!(!SettingsForm::open(&backend).time_zone_editable());
}

#[test]
fn loosening_changes_wait_and_are_listed_with_when_they_take_effect() {
    let backend = Backend::at("2026-01-05T10:00:00Z");
    let mut form = SettingsForm::open(&backend);
    form.set_weekly_target(20);
    form.save(&backend);
    backend.advance_to("2026-01-05T12:00:00Z");
    let mut form = SettingsForm::open(&backend);
    form.set_shutdown_time("18:00");
    form.set_workday_start("08:30");
    form.toggle_workday(6);
    form.set_weekly_target(15);
    let saved = form.save(&backend);
    assert_eq!(saved.text, "Saved. Looser settings take effect later.");
    assert_eq!(
        form.pending(&backend, &Zone::Utc),
        vec![
            "Workdays Mon, Tue, Wed, Thu, Fri, Sat take effect Tue 6 Jan.".to_string(),
            "Workday Start 08:30 takes effect Tue 6 Jan.".to_string(),
            "Shutdown Time 18:00 takes effect Tue 6 Jan.".to_string(),
            "A weekly target of 15 Deep Hours takes effect Mon 12 Jan.".to_string(),
        ]
    );
    // The form shows what was asked for, not what is in force tonight.
    let reopened = SettingsForm::open(&backend);
    assert_eq!(reopened.shutdown_time(), "18:00");
    assert_eq!(reopened.weekly_target(), 15);
    // Tightening applies now and leaves nothing pending.
    let mut form = SettingsForm::open(&backend);
    form.set_shutdown_time("17:00");
    form.set_workday_start("09:00");
    form.toggle_workday(6);
    form.set_weekly_target(22);
    assert_eq!(form.save(&backend).text, "Saved.");
    assert!(form.pending(&backend, &Zone::Utc).is_empty());
}

#[test]
fn invalid_settings_are_refused_calmly() {
    let backend = Backend::at("2026-01-05T10:00:00Z");
    let mut form = SettingsForm::open(&backend);
    form.set_shutdown_time("08:00");
    let refused = form.save(&backend);
    assert_eq!(refused.kind, NoticeKind::WhatIsSo);
    assert_eq!(refused.text, "Workday Start comes before Shutdown Time.");
    assert!(backend.state().unwrap().settings.is_none());
}
