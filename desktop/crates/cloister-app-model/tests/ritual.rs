//! The Shutdown Ritual flow and the Shutdown Delay through their public interface, against an
//! in-memory backend that decides with the rules engine (the backend's Postgres rules agree
//! with it on every shared fixture).

use std::cell::RefCell;

use chrono::{DateTime, Duration, Utc};
use cloister_app_model::{
    shutdown_delay, shutdown_warning, DayPlanEditor, NoticeKind, PlanBackend, RitualFlow,
    RitualStep, Zone,
};
use cloister_core::{decide, Command, Outcome, ScheduleSlot, State};

struct Backend {
    state: RefCell<State>,
    now: RefCell<DateTime<Utc>>,
}

impl Backend {
    /// Weekdays 09:00 to 17:00 UTC, with a Schedule that has a morning deep Slot.
    fn at(now: &str) -> Self {
        let backend = Backend {
            state: RefCell::default(),
            now: RefCell::new("2026-01-01T12:00:00Z".parse().unwrap()),
        };
        for cmd in [
            Command::ChangeSettings {
                workdays: vec![1, 2, 3, 4, 5],
                workday_start: "09:00".into(),
                shutdown_time: "17:00".into(),
                time_zone: None,
            },
            Command::PutSchedule {
                schedule_id: "weekdays".into(),
                time_zone: "UTC".into(),
                weekdays: vec![1, 2, 3, 4, 5],
                starts: "09:00".into(),
                ends: "17:00".into(),
                slots: vec![
                    ScheduleSlot {
                        starts: "09:00".into(),
                        ends: "12:00".into(),
                        label: "Deep work".into(),
                        deep: true,
                    },
                    ScheduleSlot {
                        starts: "12:00".into(),
                        ends: "17:00".into(),
                        label: "Admin".into(),
                        deep: false,
                    },
                ],
            },
        ] {
            assert_eq!(backend.send(&cmd).unwrap(), None);
        }
        backend.advance_to(now);
        backend
    }

    fn advance_to(&self, now: &str) {
        *self.now.borrow_mut() = now.parse().unwrap();
    }

    fn ritual(&self) -> cloister_core::RitualScreen {
        cloister_core::enforcement(&self.state.borrow(), self.server_time()).ritual
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
fn the_ritual_walks_its_five_steps_and_each_one_holds_until_done() {
    let backend = Backend::at("2026-01-05T17:00:00Z");
    let mut flow = RitualFlow::open(&backend, Zone::Utc);
    assert_eq!(flow.step(), RitualStep::LooseEnds);
    assert_eq!(flow.number(), "01");

    // 01: at least one Loose End captured tonight.
    let held = flow.next(&backend).unwrap_err();
    assert_eq!(held.text, "Capture at least one Loose End.");
    assert_eq!(held.kind, NoticeKind::WhatIsSo);
    assert!(flow.capture(&backend, "  ").is_some());
    assert_eq!(flow.capture(&backend, "Send the invoice"), None);
    assert_eq!(flow.loose_ends(&backend).len(), 1);
    flow.next(&backend).unwrap();

    // 02: review today. 03: tomorrow's draft, filled from its Schedule.
    assert_eq!(flow.step(), RitualStep::Review);
    flow.next(&backend).unwrap();
    assert_eq!(flow.step(), RitualStep::Draft);
    let draft = flow.draft(&backend).expect("tomorrow has a Schedule");
    assert_eq!(draft.date.to_string(), "2026-01-06");
    assert_eq!(draft.slots[0].label, "Deep work");
    flow.next(&backend).unwrap();

    // 04: tomorrow must be saved, with a deep Slot, covering the Workday.
    assert_eq!(flow.step(), RitualStep::Plan);
    assert_eq!(
        flow.next(&backend).unwrap_err().text,
        "Save tomorrow's Day Plan."
    );
    let mut editor = DayPlanEditor::open(&backend, flow.tomorrow(&backend).unwrap(), Zone::Utc);
    editor.save(&backend);
    flow.next(&backend).unwrap();

    // 05: "Shutdown complete", typed exactly, then held.
    assert_eq!(flow.step(), RitualStep::Commit);
    flow.set_typed("Shutdown Complete");
    assert!(!flow.typed_matches());
    assert_eq!(
        flow.complete(&backend).unwrap_err().text,
        "Type \"Shutdown complete\" exactly."
    );
    flow.set_typed("Shutdown complete");
    assert!(flow.typed_matches());
    let done = flow.complete(&backend).unwrap();
    assert_eq!(done.kind, NoticeKind::Recorded);
    assert_eq!(
        done.text,
        "Shutdown complete. Workday Start Tue 6 Jan, 09:00."
    );
    assert_eq!(backend.ritual(), cloister_core::RitualScreen::Completed);
}

#[test]
fn a_gap_in_tomorrow_holds_the_plan_step() {
    let backend = Backend::at("2026-01-05T17:00:00Z");
    let mut flow = RitualFlow::open(&backend, Zone::Utc);
    flow.capture(&backend, "Call the bank");
    let mut editor = DayPlanEditor::open(&backend, flow.tomorrow(&backend).unwrap(), Zone::Utc);
    editor.resize_slot(1, -60);
    editor.save(&backend);
    for _ in 0..3 {
        flow.next(&backend).unwrap();
    }
    assert_eq!(
        flow.next(&backend).unwrap_err().text,
        "Cover the whole Workday with Slots, 09:00 to 17:00."
    );
}

#[test]
fn going_back_is_always_possible_and_a_loose_end_can_be_closed() {
    let backend = Backend::at("2026-01-05T17:00:00Z");
    let mut flow = RitualFlow::open(&backend, Zone::Utc);
    flow.capture(&backend, "Book the venue");
    flow.next(&backend).unwrap();
    flow.back();
    assert_eq!(flow.step(), RitualStep::LooseEnds);
    let id = flow.loose_ends(&backend)[0].id.clone();
    assert_eq!(flow.close(&backend, &id), None);
    assert!(flow.loose_ends(&backend).is_empty());
}

#[test]
fn the_review_shows_todays_sessions_and_deep_hours_this_week() {
    let backend = Backend::at("2026-01-05T09:00:00Z");
    // Monday's planned deep Slot from 09:00 to 12:00, plus a 30-minute Deep Session after.
    backend.advance_to("2026-01-05T13:00:00Z");
    backend
        .send(&Command::StartBlock {
            block_id: "b1".into(),
            minutes: 30,
            deep: true,
            blocklists: vec![],
            confirm_long: false,
            client_time: None,
            intent: Some("Fix the build".into()),
            extra_domains: vec![],
            extra_apps: vec![],
        })
        .unwrap();
    backend.advance_to("2026-01-05T17:00:00Z");
    backend
        .send(&Command::RecordDepthRating {
            block_id: "b1".into(),
            rating: 4,
        })
        .unwrap();
    let review = RitualFlow::open(&backend, Zone::Utc).review(&backend);
    assert_eq!(review.sessions.len(), 2);
    assert_eq!(review.sessions[0].intent, "Deep work");
    assert_eq!(review.sessions[0].time, "09:00–12:00");
    assert_eq!(review.sessions[1].intent, "Fix the build");
    assert_eq!(review.sessions[1].depth_rating, Some(4));
    assert_eq!(review.deep_hours_this_week, Duration::minutes(210));
}

#[test]
fn shutdown_warnings_come_at_15_and_5_minutes() {
    let backend = Backend::at("2026-01-05T16:44:59Z");
    let state = backend.state().unwrap();
    let at = |s: &str| s.parse::<DateTime<Utc>>().unwrap();
    assert_eq!(
        shutdown_warning(&state, at("2026-01-05T16:44:59Z"), &Zone::Utc),
        None
    );
    let w = shutdown_warning(&state, at("2026-01-05T16:45:00Z"), &Zone::Utc).unwrap();
    assert_eq!(w.minutes, 15);
    assert_eq!(w.text, "Shutdown Time in 15 minutes, at 17:00.");
    assert_eq!(
        shutdown_warning(&state, at("2026-01-05T16:56:00Z"), &Zone::Utc)
            .unwrap()
            .minutes,
        5
    );
    assert_eq!(
        shutdown_warning(&state, at("2026-01-05T17:00:00Z"), &Zone::Utc),
        None
    );
}

#[test]
fn one_shutdown_delay_a_week_moves_the_warnings_and_then_says_none_is_left() {
    let backend = Backend::at("2026-01-05T16:40:00Z");
    let view = shutdown_delay(&backend.state().unwrap(), backend.server_time());
    assert_eq!(view.left, 1);
    assert_eq!(view.text, "1 Shutdown Delay left this week.");
    assert_eq!(
        cloister_app_model::use_shutdown_delay(&backend, &Zone::Utc).text,
        "Shutdown delayed to 17:30. Logged."
    );
    let state = backend.state().unwrap();
    assert_eq!(
        shutdown_warning(&state, "2026-01-05T17:20:00Z".parse().unwrap(), &Zone::Utc)
            .unwrap()
            .text,
        "Shutdown Time in 15 minutes, at 17:30."
    );
    let view = shutdown_delay(&state, backend.server_time());
    assert_eq!(view.left, 0);
    assert_eq!(view.text, "No Shutdown Delay left this week.");
    assert_eq!(
        cloister_app_model::use_shutdown_delay(&backend, &Zone::Utc).text,
        "No Shutdown Delay left this week."
    );
}
