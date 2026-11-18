//! The Shutdown Ritual's five steps, the Shutdown Time warnings and the Shutdown Delay.
//!
//! Each step's requirement is decided by the rules engine on the state last synced (the same
//! rules the backend applies to `complete_ritual`), so a step holds exactly when completing
//! would be refused for its reason.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use cloister_core::{
    day_plan, decide, ritual, ritual_due_at, shutdown_delays_left, Command, DayPlan, LooseEnd,
    Outcome, State, SHUTDOWN_COMPLETE,
};

use crate::day_plan::PlanBackend;
use crate::format::{clock, day, Zone};
use crate::notice::Notice;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RitualStep {
    LooseEnds,
    Review,
    Draft,
    Plan,
    Commit,
}

impl RitualStep {
    pub const ALL: [RitualStep; 5] = [
        RitualStep::LooseEnds,
        RitualStep::Review,
        RitualStep::Draft,
        RitualStep::Plan,
        RitualStep::Commit,
    ];

    /// The brass step number: "01" to "05".
    pub fn number(self) -> &'static str {
        match self {
            RitualStep::LooseEnds => "01",
            RitualStep::Review => "02",
            RitualStep::Draft => "03",
            RitualStep::Plan => "04",
            RitualStep::Commit => "05",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            RitualStep::LooseEnds => "Loose Ends",
            RitualStep::Review => "Review today",
            RitualStep::Draft => "Tomorrow's draft",
            RitualStep::Plan => "Plan tomorrow",
            RitualStep::Commit => "Shutdown complete",
        }
    }

    fn next(self) -> Option<RitualStep> {
        Self::ALL.get(self as usize + 1).copied()
    }

    fn previous(self) -> Option<RitualStep> {
        (self as usize).checked_sub(1).map(|i| Self::ALL[i])
    }
}

/// One Deep Session of today, for the review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewSession {
    /// The Intent, or the deep Slot's label.
    pub intent: String,
    /// "09:00–12:00".
    pub time: String,
    pub depth_rating: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    pub sessions: Vec<ReviewSession>,
    pub deep_hours_this_week: Duration,
}

/// The Shutdown Ritual as it is walked on one screen.
pub struct RitualFlow {
    step: RitualStep,
    typed: String,
    zone: Zone,
}

impl RitualFlow {
    pub fn open(_backend: &impl PlanBackend, zone: Zone) -> Self {
        RitualFlow {
            step: RitualStep::LooseEnds,
            typed: String::new(),
            zone,
        }
    }

    pub fn step(&self) -> RitualStep {
        self.step
    }

    pub fn back(&mut self) {
        if let Some(previous) = self.step.previous() {
            self.step = previous;
        }
    }

    /// Move on, unless this step's requirement isn't met yet (then: what it needs).
    pub fn next(&mut self, backend: &impl PlanBackend) -> Result<(), Notice> {
        if let Some(code) = self.holding_reason(backend) {
            let held = matches!(
                (self.step, code.as_str()),
                (RitualStep::LooseEnds, "loose_end_required")
                    | (
                        RitualStep::Plan,
                        "day_plan_required" | "deep_slot_required" | "workday_not_covered"
                    )
            );
            if held {
                return Err(self.notice(backend, &code));
            }
        }
        if let Some(next) = self.step.next() {
            self.step = next;
        }
        Ok(())
    }

    /// Why completing now would be refused, as decided by the rules engine.
    fn holding_reason(&self, backend: &impl PlanBackend) -> Option<String> {
        let state = backend.state().unwrap_or_default();
        let command = Command::CompleteRitual {
            typed: SHUTDOWN_COMPLETE.into(),
        };
        match decide(&state, &command, backend.server_time()).outcome {
            Outcome::Accepted => None,
            Outcome::Rejected { reason } => Some(reason.code().to_string()),
        }
    }

    /// Every open Loose End, oldest first (carried from earlier days too).
    pub fn loose_ends(&self, backend: &impl PlanBackend) -> Vec<LooseEnd> {
        backend.state().unwrap_or_default().loose_ends
    }

    /// Capture a Loose End. `None` when recorded, else what is so.
    pub fn capture(&mut self, backend: &impl PlanBackend, text: &str) -> Option<Notice> {
        let command = Command::CaptureLooseEnd {
            loose_end_id: cloister_sync::new_block_id(),
            text: text.into(),
        };
        self.send(backend, &command)
    }

    pub fn close(&mut self, backend: &impl PlanBackend, id: &str) -> Option<Notice> {
        let command = Command::CloseLooseEnd {
            loose_end_id: id.into(),
        };
        self.send(backend, &command)
    }

    /// Today's Deep Sessions (deep Slots of today's plan and deep Blocks) and the week's Deep
    /// Hours so far.
    pub fn review(&self, backend: &impl PlanBackend) -> Review {
        let state = backend.state().unwrap_or_default();
        let now = backend.server_time();
        let date = ritual(&state, now)
            .map(|r| r.date)
            .unwrap_or_else(|| local_date(&state, now));
        let mut sessions: Vec<(DateTime<Utc>, ReviewSession)> = Vec::new();
        if let Some(plan) = day_plan(&state, date) {
            for slot in plan.slots.iter().filter(|s| s.deep && s.starts_at <= now) {
                sessions.push((
                    slot.starts_at,
                    ReviewSession {
                        intent: slot.label.clone(),
                        time: self.span(slot.starts_at, slot.ends_at),
                        depth_rating: slot.depth_rating,
                    },
                ));
            }
        }
        for block in state
            .blocks
            .iter()
            .filter(|b| b.deep && local_date(&state, b.starts_at) == date && b.starts_at <= now)
        {
            sessions.push((
                block.starts_at,
                ReviewSession {
                    intent: block
                        .intent
                        .clone()
                        .unwrap_or_else(|| "Deep Session".into()),
                    time: self.span(block.starts_at, block.ends_at),
                    depth_rating: block.depth_rating,
                },
            ));
        }
        sessions.sort_by_key(|(at, _)| *at);
        Review {
            sessions: sessions.into_iter().map(|(_, s)| s).collect(),
            deep_hours_this_week: deep_hours_this_week(&state, now, &self.zone),
        }
    }

    /// The Workday the ritual plans: the day of the next Workday Start.
    pub fn tomorrow(&self, backend: &impl PlanBackend) -> Option<NaiveDate> {
        let state = backend.state().unwrap_or_default();
        let now = backend.server_time();
        let start = ritual(&state, now)
            .and_then(|r| r.lapses_at)
            .or_else(|| cloister_core::next_workday_start(&state, now))?;
        Some(local_date(&state, start))
    }

    /// Tomorrow's plan as it stands: saved, or filled from its Schedule.
    pub fn draft(&self, backend: &impl PlanBackend) -> Option<DayPlan> {
        let state = backend.state().unwrap_or_default();
        day_plan(&state, self.tomorrow(backend)?)
    }

    pub fn typed(&self) -> &str {
        &self.typed
    }

    pub fn set_typed(&mut self, typed: &str) {
        self.typed = typed.to_string();
    }

    /// Whether the typed text is exactly "Shutdown complete" (the hold is enabled then).
    pub fn typed_matches(&self) -> bool {
        self.typed == SHUTDOWN_COMPLETE
    }

    /// Complete the ritual (after the hold). Ok: the brass record of it.
    pub fn complete(&mut self, backend: &impl PlanBackend) -> Result<Notice, Notice> {
        let command = Command::CompleteRitual {
            typed: self.typed.clone(),
        };
        if let Some(notice) = self.send(backend, &command) {
            return Err(notice);
        }
        let state = backend.state().unwrap_or_default();
        let next = cloister_core::next_workday_start(&state, backend.server_time());
        Ok(Notice::recorded(match next {
            Some(t) => format!(
                "Shutdown complete. Workday Start {}, {}.",
                day(t, &self.zone),
                clock(t, &self.zone)
            ),
            None => "Shutdown complete.".into(),
        }))
    }

    fn send(&self, backend: &impl PlanBackend, command: &Command) -> Option<Notice> {
        match backend.send(command) {
            Ok(None) => None,
            Ok(Some(code)) => Some(self.notice(backend, &code)),
            Err(_) => Some(Notice::what_is_so(
                "The backend can't be reached. The ritual stays until it can.",
            )),
        }
    }

    fn notice(&self, backend: &impl PlanBackend, code: &str) -> Notice {
        let state = backend.state().unwrap_or_default();
        let bounds = self.tomorrow(backend).and_then(|date| {
            match cloister_core::workday(&state, date, backend.server_time()) {
                cloister_core::Workday::Bounds(a, b) => Some(self.span(a, b).replace('–', " to ")),
                _ => None,
            }
        });
        Notice::what_is_so(match code {
            "loose_end_required" => "Capture at least one Loose End.".to_string(),
            "invalid_loose_end" => "A Loose End needs a few words, 500 characters at most.".into(),
            "day_plan_required" => "Save tomorrow's Day Plan.".into(),
            "deep_slot_required" => "Tomorrow needs at least one deep Slot.".into(),
            "workday_not_covered" => match bounds {
                Some(b) => format!("Cover the whole Workday with Slots, {b}."),
                None => "Cover the whole Workday with Slots.".into(),
            },
            "confirmation_mismatch" => format!("Type \"{SHUTDOWN_COMPLETE}\" exactly."),
            "ritual_not_due" => "No Shutdown Ritual is due.".into(),
            "ritual_already_completed" => "Shutdown is already complete.".into(),
            other => return Notice::refusal(other, None, &self.zone),
        })
    }

    fn span(&self, a: DateTime<Utc>, b: DateTime<Utc>) -> String {
        format!("{}–{}", clock(a, &self.zone), clock(b, &self.zone))
    }
}

fn local_date(state: &State, at: DateTime<Utc>) -> NaiveDate {
    let tz: chrono_tz::Tz = state.time_zone.parse().unwrap_or(chrono_tz::Tz::UTC);
    at.with_timezone(&tz).date_naive()
}

/// Deep Hours since Monday 00:00 (in `zone`): deep Blocks plus deep Slots of saved or
/// Schedule-filled plans, counted up to `now`.
pub fn deep_hours_this_week(state: &State, now: DateTime<Utc>, zone: &Zone) -> Duration {
    let since = zone.week_start(now);
    let counted =
        |a: DateTime<Utc>, b: DateTime<Utc>| (b.min(now) - a.max(since)).max(Duration::zero());
    let blocks = state
        .blocks
        .iter()
        .filter(|b| b.deep)
        .map(|b| counted(b.starts_at, b.ends_at));
    let first = local_date(state, since);
    let last = local_date(state, now);
    let slots = (0..=(last - first).num_days().max(0))
        .filter_map(|n| day_plan(state, first + Duration::days(n)))
        .flat_map(|p| p.slots)
        .filter(|s| s.deep)
        .map(|s| counted(s.starts_at, s.ends_at));
    blocks.chain(slots).fold(Duration::zero(), |a, b| a + b)
}

/// A warning before Shutdown Time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownWarning {
    /// 15 or 5.
    pub minutes: i64,
    pub due_at: DateTime<Utc>,
    pub text: String,
}

/// The warning to show at `now`: 15 minutes before today's Shutdown Time (or the delayed one)
/// until 5 minutes before, then the 5-minute one until it is due.
pub fn shutdown_warning(state: &State, now: DateTime<Utc>, zone: &Zone) -> Option<ShutdownWarning> {
    let due_at = ritual_due_at(state, local_date(state, now), now)?;
    let left = due_at - now;
    let minutes = if left <= Duration::zero() || left > Duration::minutes(15) {
        return None;
    } else if left > Duration::minutes(5) {
        15
    } else {
        5
    };
    Some(ShutdownWarning {
        minutes,
        due_at,
        text: format!(
            "Shutdown Time in {minutes} minutes, at {}.",
            clock(due_at, zone)
        ),
    })
}

/// How many Shutdown Delays are left this week, in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelayView {
    pub left: u8,
    pub text: String,
}

pub fn shutdown_delay(state: &State, now: DateTime<Utc>) -> DelayView {
    let left = shutdown_delays_left(state, now);
    DelayView {
        left,
        text: if left == 0 {
            "No Shutdown Delay left this week.".into()
        } else {
            format!("{left} Shutdown Delay left this week.")
        },
    }
}

/// Use this week's Shutdown Delay. Returns what was recorded, or what is so.
pub fn use_shutdown_delay(backend: &impl PlanBackend, zone: &Zone) -> Notice {
    let outcome = backend.send(&Command::UseShutdownDelay {});
    let state = backend.state().unwrap_or_default();
    let now = backend.server_time();
    match outcome {
        Ok(None) => match ritual_due_at(&state, local_date(&state, now), now) {
            Some(t) => Notice::recorded(format!("Shutdown delayed to {}. Logged.", clock(t, zone))),
            None => Notice::recorded("Shutdown delayed by 30 minutes. Logged."),
        },
        Ok(Some(code)) if code == "shutdown_delay_used" => {
            Notice::what_is_so("No Shutdown Delay left this week.")
        }
        Ok(Some(code)) if code == "evening_lockdown" || code == "rest_day" => {
            Notice::what_is_so("A Shutdown Delay is used before Shutdown Time, on a Workday.")
        }
        Ok(Some(code)) if code == "no_shutdown_time" => {
            Notice::what_is_so("Set a Shutdown Time first.")
        }
        Ok(Some(code)) => Notice::refusal(&code, None, zone),
        Err(_) => Notice::what_is_so("The backend can't be reached. Shutdown Time stands."),
    }
}
