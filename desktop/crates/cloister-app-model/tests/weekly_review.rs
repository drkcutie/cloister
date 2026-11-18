//! The Weekly Review step of the Shutdown Ritual and the Deep Hours scoreboard, through their
//! public interface, against an in-memory backend that decides with the rules engine.

use std::cell::RefCell;

use chrono::{DateTime, Utc};
use cloister_app_model::{
    scoreboard, DayPlanEditor, NoticeKind, PlanBackend, RecordKind, RitualFlow, RitualStep, Zone,
};
use cloister_core::{decide, Command, Outcome, ScheduleSlot, State};

struct Backend {
    state: RefCell<State>,
    now: RefCell<DateTime<Utc>>,
}

impl Backend {
    /// Weekdays 09:00 to 17:00 UTC (set on Thu 1 Jan), a morning deep Slot by Schedule, and a
    /// weekly target of 20 Deep Hours.
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
            Command::SetWeeklyTarget { hours: 20 },
        ] {
            assert_eq!(backend.send(&cmd).unwrap(), None);
        }
        backend.advance_to(now);
        backend
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

/// Capture a Loose End, walk to the Plan step and save tomorrow's Schedule draft.
fn walk_to_plan_and_save(backend: &Backend, flow: &mut RitualFlow) {
    flow.capture(backend, "Send the invoice");
    for _ in 0..3 {
        flow.next(backend).unwrap();
    }
    assert_eq!(flow.step(), RitualStep::Plan);
    let mut editor = DayPlanEditor::open(backend, flow.tomorrow(backend).unwrap(), Zone::Utc);
    editor.save(backend);
    flow.next(backend).unwrap();
}

#[test]
fn a_ritual_before_the_last_workday_has_no_weekly_review() {
    let backend = Backend::at("2026-01-08T17:00:00Z");
    let mut flow = RitualFlow::open(&backend, Zone::Utc);
    assert_eq!(flow.steps().len(), 5);
    walk_to_plan_and_save(&backend, &mut flow);
    assert_eq!(flow.step(), RitualStep::Commit);
    assert_eq!(flow.number(), "05");
    flow.set_typed("Shutdown complete");
    assert_eq!(flow.complete(&backend).unwrap().kind, NoticeKind::Recorded);
}

#[test]
fn the_last_workday_adds_the_weekly_review_which_sets_next_weeks_target() {
    let backend = Backend::at("2026-01-09T17:00:00Z");
    let mut flow = RitualFlow::open(&backend, Zone::Utc);
    assert_eq!(
        flow.steps(),
        &[
            RitualStep::LooseEnds,
            RitualStep::Review,
            RitualStep::Draft,
            RitualStep::Plan,
            RitualStep::WeeklyReview,
            RitualStep::Commit,
        ]
    );
    walk_to_plan_and_save(&backend, &mut flow);
    assert_eq!(flow.step(), RitualStep::WeeklyReview);
    assert_eq!(flow.number(), "05");
    // Starts from the target in force; lowering waits for next week.
    assert_eq!(flow.weekly_target(), 20);
    flow.set_weekly_target(15);
    assert_eq!(
        flow.target_note(&backend),
        "A lower target takes effect Mon 12 Jan."
    );
    flow.set_weekly_target(0);
    assert_eq!(flow.weekly_target(), 1);
    flow.set_weekly_target(15);
    flow.next(&backend).unwrap();
    assert_eq!(flow.step(), RitualStep::Commit);
    assert_eq!(flow.number(), "06");
    flow.set_typed("Shutdown complete");
    flow.complete(&backend).unwrap();
    let state = backend.state().unwrap();
    assert_eq!(state.weekly_target.unwrap().hours, 20);
    assert_eq!(state.pending_weekly_target.unwrap().hours, 15);
}

#[test]
fn raising_the_target_in_the_weekly_review_applies_now() {
    let backend = Backend::at("2026-01-09T17:00:00Z");
    let mut flow = RitualFlow::open(&backend, Zone::Utc);
    walk_to_plan_and_save(&backend, &mut flow);
    flow.set_weekly_target(25);
    assert_eq!(flow.target_note(&backend), "A higher target applies now.");
    flow.next(&backend).unwrap();
    flow.set_typed("Shutdown complete");
    flow.complete(&backend).unwrap();
    assert_eq!(backend.state().unwrap().weekly_target.unwrap().hours, 25);
}

#[test]
fn the_scoreboard_shows_deep_hours_by_day_the_depth_trend_and_the_record() {
    let backend = Backend::at("2026-01-05T13:00:00Z");
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
    backend.advance_to("2026-01-05T14:00:00Z");
    backend
        .send(&Command::RecordDepthRating {
            block_id: "b1".into(),
            rating: 4,
        })
        .unwrap();
    backend.advance_to("2026-01-06T10:00:00Z");
    assert_eq!(backend.send(&Command::UseShutdownDelay {}).unwrap(), None);
    // Tuesday 10:30: Monday's ritual lapsed at 09:00 (Missed), today's deep Slot is 1h 30m in.
    backend.advance_to("2026-01-06T10:30:00Z");
    let board = scoreboard(&backend.state().unwrap(), backend.server_time(), &Zone::Utc);
    // Monday: the 3-hour deep Slot plus the 30-minute Deep Session. Tuesday: 1h 30m so far.
    assert_eq!(board.hours_text, "5 of 20 Deep Hours");
    assert_eq!(board.deep_minutes, 300);
    let labels: Vec<_> = board.days.iter().map(|d| d.label.as_str()).collect();
    assert_eq!(labels, ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]);
    assert_eq!(board.days[0].minutes, 210);
    assert_eq!(board.days[0].fill, 1.0);
    assert_eq!(board.days[1].text, "1h 30m");
    assert!((board.days[1].fill - 90.0 / 210.0).abs() < 1e-6);
    assert!((board.well_fill - 0.25).abs() < 1e-6);
    assert_eq!(board.depth_trend, vec![4]);
    let records: Vec<_> = board
        .records
        .iter()
        .map(|r| (r.kind, r.text.as_str()))
        .collect();
    assert_eq!(
        records,
        [
            (RecordKind::MissedRitual, "Mon 5 Jan."),
            (RecordKind::ShutdownDelayUsed, "Tue 6 Jan."),
        ]
    );
    assert!(board.records.iter().all(|r| !r.tag.contains('%')));
}
