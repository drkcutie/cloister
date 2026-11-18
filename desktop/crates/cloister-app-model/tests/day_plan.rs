//! The Day Plan editor through its public interface, against an in-memory backend that decides
//! with the rules engine (the backend's Postgres rules agree with it on every shared fixture).

use std::cell::RefCell;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use cloister_app_model::{DayPlanEditor, NoticeKind, PlanBackend, Zone};
use cloister_core::{decide, Command, Outcome, ScheduleSlot, State};

struct Backend {
    state: RefCell<State>,
    now: RefCell<DateTime<Utc>>,
    sent: RefCell<usize>,
}

impl Backend {
    fn at(now: &str) -> Self {
        Backend {
            state: RefCell::default(),
            now: RefCell::new(now.parse().unwrap()),
            sent: RefCell::new(0),
        }
    }

    fn advance_to(&self, now: &str) {
        *self.now.borrow_mut() = now.parse().unwrap();
    }

    fn weekday_schedule(self) -> Self {
        let cmd = Command::PutSchedule {
            schedule_id: "weekdays".into(),
            time_zone: "UTC".into(),
            weekdays: vec![1, 2, 3, 4, 5],
            starts: "08:00".into(),
            ends: "17:00".into(),
            slots: vec![ScheduleSlot {
                starts: "08:00".into(),
                ends: "11:00".into(),
                label: "Deep work".into(),
                deep: true,
            }],
        };
        assert_eq!(self.send(&cmd).unwrap(), None);
        *self.sent.borrow_mut() = 0;
        self
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
        *self.sent.borrow_mut() += 1;
        let decision = decide(&self.state.borrow(), command, self.server_time());
        *self.state.borrow_mut() = decision.state;
        Ok(match decision.outcome {
            Outcome::Accepted => None,
            Outcome::Rejected { reason } => Some(reason.code().to_string()),
        })
    }
}

fn monday() -> NaiveDate {
    "2026-01-05".parse().unwrap()
}

fn t(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

#[test]
fn tomorrow_starts_from_its_schedule_and_saving_it_sets_the_floor() {
    let backend = Backend::at("2026-01-04T20:00:00Z").weekday_schedule();
    let mut editor = DayPlanEditor::open(&backend, monday(), Zone::Utc);
    assert_eq!(editor.slots().len(), 1);
    assert_eq!(editor.slots()[0].label, "Deep work");
    assert!(editor.slots()[0].deep);
    assert_eq!(editor.planned_deep_minutes(), 180);
    // Not saved yet, and the day hasn't begun: the Schedule is only a draft.
    assert_eq!(editor.floor(), 0);

    let i = editor.add_slot();
    editor.set_label(i, "Review notes");
    editor.set_deep(i, true);
    assert_eq!(editor.planned_deep_minutes(), 240);
    let notice = editor.save(&backend).expect("a notice");
    assert_eq!(notice.kind, NoticeKind::Recorded);
    assert_eq!(editor.floor(), 240);
    let plan = backend.state().unwrap().day_plans[0].clone();
    assert_eq!(plan.planned_deep_minutes, 240);
    assert_eq!(plan.slots[1].starts_at, t("2026-01-05T11:00:00Z"));
}

#[test]
fn shrinking_deep_time_is_refused_before_anything_is_sent() {
    let backend = Backend::at("2026-01-04T20:00:00Z").weekday_schedule();
    let mut editor = DayPlanEditor::open(&backend, monday(), Zone::Utc);
    editor.save(&backend);
    let sent = *backend.sent.borrow();

    editor.resize_slot(0, -30);
    assert_eq!(editor.planned_deep_minutes(), 150);
    let notice = editor.check(&backend).expect("refused");
    assert_eq!(notice.text, "This Slot can be moved, not removed.");
    assert_eq!(notice.kind, NoticeKind::WhatIsSo);
    assert_eq!(
        editor.save(&backend).unwrap().text,
        "This Slot can be moved, not removed."
    );
    assert_eq!(*backend.sent.borrow(), sent, "nothing was sent");

    // Moving it instead is fine.
    editor.resize_slot(0, 30);
    editor.move_slot(0, 60);
    assert_eq!(editor.check(&backend), None);
    assert_eq!(editor.save(&backend).unwrap().kind, NoticeKind::Recorded);
    assert_eq!(
        backend.state().unwrap().day_plans[0].slots[0].starts_at,
        t("2026-01-05T09:00:00Z")
    );
}

#[test]
fn a_started_deep_session_is_locked_and_slots_stay_inside_the_workday() {
    let backend = Backend::at("2026-01-04T20:00:00Z").weekday_schedule();
    let mut editor = DayPlanEditor::open(&backend, monday(), Zone::Utc);
    editor.move_slot(0, -60);
    assert_eq!(editor.slots()[0].starts_at, t("2026-01-05T08:00:00Z"));
    editor.move_slot(0, 24 * 60);
    assert_eq!(editor.slots()[0].ends_at, t("2026-01-05T17:00:00Z"));
    editor.move_slot(0, -24 * 60);
    editor.save(&backend);

    backend.advance_to("2026-01-05T09:00:00Z");
    let mut editor = DayPlanEditor::open(&backend, monday(), Zone::Utc);
    assert!(editor.is_locked(0, backend.server_time()));
    editor.move_slot(0, 15);
    assert_eq!(
        editor.check(&backend).unwrap().text,
        "Ends at 11:00. Can't be changed."
    );
}

#[test]
fn a_day_without_a_plan_or_schedule_opens_empty_on_the_default_workday() {
    let backend = Backend::at("2026-01-04T20:00:00Z");
    let mut editor = DayPlanEditor::open(&backend, monday(), Zone::Utc);
    assert!(editor.slots().is_empty());
    assert_eq!(editor.starts_at(), t("2026-01-05T09:00:00Z"));
    assert_eq!(editor.ends_at(), t("2026-01-05T17:00:00Z"));
    let a = editor.add_slot();
    let b = editor.add_slot();
    assert_eq!(editor.slots()[a].ends_at, editor.slots()[b].starts_at);
    assert_eq!(
        editor.slots()[b].ends_at - editor.slots()[b].starts_at,
        Duration::hours(1)
    );
    assert_eq!(
        editor.check(&backend).unwrap().text,
        "Every Slot needs a label and a time inside the Workday."
    );
}
