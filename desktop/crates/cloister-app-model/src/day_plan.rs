//! The Day Plan editor: one day's Slots on a timeline, checked by the rules engine as they are
//! edited and saved through the backend (which applies the same rules).

use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Utc};
use cloister_core::{decide, Command, Outcome, Slot, SlotInput, State, Workday};

use crate::format::{clock, Zone};
use crate::notice::Notice;

/// Where Day Plans are read from and written to: the backend (through a Device's mirror).
pub trait PlanBackend {
    /// The account's state as last synced.
    fn state(&self) -> Result<State, String>;
    /// The best estimate of server time.
    fn server_time(&self) -> DateTime<Utc>;
    /// Send a command; `Ok(None)` when accepted, `Ok(Some(reason code))` when refused.
    fn send(&self, command: &Command) -> Result<Option<String>, String>;
}

impl PlanBackend for cloister_sync::Device {
    fn state(&self) -> Result<State, String> {
        cloister_sync::Device::state(self).map_err(|e| e.to_string())
    }

    fn server_time(&self) -> DateTime<Utc> {
        cloister_sync::Device::server_time(self)
    }

    fn send(&self, command: &Command) -> Result<Option<String>, String> {
        match cloister_sync::Device::send(self, command).map_err(|e| e.to_string())? {
            cloister_sync::CommandOutcome::Accepted => Ok(None),
            cloister_sync::CommandOutcome::Rejected { reason } => Ok(Some(reason)),
        }
    }
}

/// The Workday used only before any settings exist, for a day with neither a saved Day Plan
/// nor a Schedule. Once Workday Start and Shutdown Time are set, they give every day's bounds.
pub const DEFAULT_WORKDAY: (u32, u32) = (9, 17);
/// Slots move and resize in steps of this many minutes.
pub const STEP_MINUTES: i64 = 15;

/// A Slot being edited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorSlot {
    pub id: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub label: String,
    pub deep: bool,
}

impl From<&Slot> for EditorSlot {
    fn from(s: &Slot) -> Self {
        EditorSlot {
            id: s.id.clone(),
            starts_at: s.starts_at,
            ends_at: s.ends_at,
            label: s.label.clone(),
            deep: s.deep,
        }
    }
}

pub struct DayPlanEditor {
    date: NaiveDate,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    slots: Vec<EditorSlot>,
    /// The Slots of the plan in force when opened (for locks and the ratchet's floor).
    in_force: Vec<Slot>,
    floor: i64,
    saved: bool,
    rest_day: bool,
    zone: Zone,
}

impl DayPlanEditor {
    /// Open `date`: its saved Day Plan, or the draft its Schedule fills in, or an empty
    /// default Workday.
    pub fn open(backend: &impl PlanBackend, date: NaiveDate, zone: Zone) -> Self {
        let state = backend.state().unwrap_or_default();
        let now = backend.server_time();
        let mut editor = Self::open_plan(&state, now, date, zone);
        match cloister_core::workday(&state, date, now) {
            Workday::NotConfigured => {}
            Workday::RestDay => editor.rest_day = true,
            // A saved plan keeps its bounds; a draft or empty day is fitted to the settings.
            Workday::Bounds(start, end) if !editor.saved => {
                editor.starts_at = start;
                editor.ends_at = end;
            }
            Workday::Bounds(..) => {}
        }
        editor
    }

    fn open_plan(state: &State, now: DateTime<Utc>, date: NaiveDate, zone: Zone) -> Self {
        let saved = state.day_plans.iter().find(|p| p.date == date).cloned();
        let plan = saved
            .clone()
            .or_else(|| cloister_core::schedule_plan(state, date));
        match plan {
            Some(plan) => {
                let in_force = saved.is_some() || plan.starts_at <= now;
                DayPlanEditor {
                    date,
                    starts_at: plan.starts_at,
                    ends_at: plan.ends_at,
                    slots: plan.slots.iter().map(EditorSlot::from).collect(),
                    floor: if in_force {
                        plan.planned_deep_minutes
                    } else {
                        0
                    },
                    in_force: if in_force { plan.slots } else { vec![] },
                    saved: saved.is_some(),
                    rest_day: false,
                    zone,
                }
            }
            None => {
                let at = |h| zone.at(date, NaiveTime::from_hms_opt(h, 0, 0).expect("hour"));
                DayPlanEditor {
                    date,
                    starts_at: at(DEFAULT_WORKDAY.0),
                    ends_at: at(DEFAULT_WORKDAY.1),
                    slots: vec![],
                    in_force: vec![],
                    floor: 0,
                    saved: false,
                    rest_day: false,
                    zone,
                }
            }
        }
    }

    pub fn date(&self) -> NaiveDate {
        self.date
    }

    pub fn starts_at(&self) -> DateTime<Utc> {
        self.starts_at
    }

    pub fn ends_at(&self) -> DateTime<Utc> {
        self.ends_at
    }

    /// Whether this date is a Rest Day under the settings (it can't have a Day Plan).
    pub fn is_rest_day(&self) -> bool {
        self.rest_day
    }

    /// Whether this day has a saved Day Plan (otherwise it's a draft).
    pub fn is_saved(&self) -> bool {
        self.saved
    }

    pub fn slots(&self) -> &[EditorSlot] {
        &self.slots
    }

    /// Indices of the Slots in order of start.
    pub fn in_time_order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.slots.len()).collect();
        order.sort_by_key(|&i| (self.slots[i].starts_at, self.slots[i].id.clone()));
        order
    }

    /// Planned Deep Minutes of the plan as edited.
    pub fn planned_deep_minutes(&self) -> i64 {
        self.slots
            .iter()
            .filter(|s| s.deep)
            .map(|s| (s.ends_at - s.starts_at).num_minutes())
            .sum()
    }

    /// The day's Planned Deep Minutes so far: the floor the plan can't go below.
    pub fn floor(&self) -> i64 {
        self.floor
    }

    /// A deep Slot whose Deep Session has started can't change.
    pub fn is_locked(&self, i: usize, now: DateTime<Utc>) -> bool {
        let id = &self.slots[i].id;
        self.in_force
            .iter()
            .any(|o| &o.id == id && o.deep && o.starts_at <= now)
    }

    /// Add an hour-long Slot (shorter if the gap is) in the first free gap of the day.
    /// Returns its index.
    pub fn add_slot(&mut self) -> usize {
        let mut taken: Vec<(DateTime<Utc>, DateTime<Utc>)> = self
            .slots
            .iter()
            .map(|s| (s.starts_at, s.ends_at))
            .collect();
        taken.sort();
        let step = Duration::minutes(STEP_MINUTES);
        let mut at = self.starts_at;
        let mut place = None;
        for (a, b) in taken.iter().copied().chain([(self.ends_at, self.ends_at)]) {
            if a - at >= step {
                place = Some((at, (at + Duration::hours(1)).min(a)));
                break;
            }
            at = at.max(b);
        }
        let (starts_at, ends_at) =
            place.unwrap_or((self.ends_at - Duration::hours(1), self.ends_at));
        self.slots.push(EditorSlot {
            id: cloister_sync::new_block_id(),
            starts_at,
            ends_at,
            label: String::new(),
            deep: false,
        });
        self.slots.len() - 1
    }

    /// Move a Slot by `minutes`, keeping it inside the Workday.
    pub fn move_slot(&mut self, i: usize, minutes: i64) {
        let (lo, hi) = (self.starts_at, self.ends_at);
        let s = &mut self.slots[i];
        let length = s.ends_at - s.starts_at;
        let starts = (s.starts_at + Duration::minutes(minutes))
            .min(hi - length)
            .max(lo);
        s.starts_at = starts;
        s.ends_at = starts + length;
    }

    /// Move a Slot's end by `minutes`, keeping at least one step and inside the Workday.
    pub fn resize_slot(&mut self, i: usize, minutes: i64) {
        let hi = self.ends_at;
        let s = &mut self.slots[i];
        s.ends_at = (s.ends_at + Duration::minutes(minutes))
            .min(hi)
            .max(s.starts_at + Duration::minutes(STEP_MINUTES));
    }

    pub fn set_label(&mut self, i: usize, label: &str) {
        self.slots[i].label = label.to_string();
    }

    pub fn set_deep(&mut self, i: usize, deep: bool) {
        self.slots[i].deep = deep;
    }

    pub fn remove(&mut self, i: usize) {
        self.slots.remove(i);
    }

    /// The command that saves the plan as edited.
    pub fn command(&self) -> Command {
        Command::WriteDayPlan {
            date: self.date,
            starts_at: self.starts_at,
            ends_at: self.ends_at,
            slots: self
                .slots
                .iter()
                .map(|s| SlotInput {
                    id: s.id.clone(),
                    starts_at: s.starts_at,
                    ends_at: s.ends_at,
                    label: s.label.clone(),
                    deep: s.deep,
                })
                .collect(),
        }
    }

    /// Would the plan as edited be accepted? `None` if so, else the calm notice why not.
    /// Decided locally by the rules engine, on the state last synced.
    pub fn check(&self, backend: &impl PlanBackend) -> Option<Notice> {
        let state = backend.state().unwrap_or_default();
        let now = backend.server_time();
        match decide(&state, &self.command(), now).outcome {
            Outcome::Accepted => None,
            Outcome::Rejected { reason } => Some(self.refusal(reason.code(), now)),
        }
    }

    /// Save the plan. Refused plans are never sent. Returns what to tell the user.
    pub fn save(&mut self, backend: &impl PlanBackend) -> Option<Notice> {
        if let Some(notice) = self.check(backend) {
            return Some(notice);
        }
        match backend.send(&self.command()) {
            Ok(None) => {
                *self = DayPlanEditor::open(backend, self.date, self.zone);
                Some(Notice::recorded(format!(
                    "Day Plan saved. Planned Deep Minutes: {}.",
                    self.floor
                )))
            }
            Ok(Some(code)) => Some(self.refusal(&code, backend.server_time())),
            Err(_) => Some(Notice::what_is_so(
                "The Day Plan wasn't saved: the backend can't be reached.",
            )),
        }
    }

    fn refusal(&self, code: &str, now: DateTime<Utc>) -> Notice {
        // The started Deep Session the refusal is about.
        let started_end = self
            .in_force
            .iter()
            .filter(|o| o.deep && o.starts_at <= now)
            .find(|o| {
                !self.slots.iter().any(|s| {
                    s.id == o.id
                        && s.starts_at == o.starts_at
                        && s.ends_at == o.ends_at
                        && s.deep
                        && s.label.trim() == o.label
                })
            })
            .map(|o| o.ends_at);
        Notice::day_plan_refusal(code, started_end, now, &self.zone)
    }
}

impl Notice {
    /// The notice for a Day Plan refusal `code`. `started_end` is the end of the started Deep
    /// Session the refusal is about, if any.
    pub fn day_plan_refusal(
        code: &str,
        started_end: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
        zone: &Zone,
    ) -> Self {
        Notice::what_is_so(match code {
            "slot_cannot_be_removed" => "This Slot can be moved, not removed.".to_string(),
            "deep_session_has_started" => match started_end {
                Some(t) => format!("Ends at {}. Can't be changed.", clock(t, zone)),
                None => "This Deep Session has started. It can't be changed.".into(),
            },
            "slot_in_the_past" => {
                format!("Deep Slots start at {} at the earliest.", clock(now, zone))
            }
            "slots_overlap" => "Two Slots overlap. Move one.".into(),
            "invalid_slot" => "Every Slot needs a label and a time inside the Workday.".into(),
            "invalid_day_plan" => "The Workday runs from Workday Start to Shutdown Time.".into(),
            "rest_day" => "A Rest Day has no Day Plan.".into(),
            other => return Notice::refusal(other, None, zone),
        })
    }
}

impl Zone {
    /// `time` on `date` in this zone, as an instant.
    pub fn at(&self, date: NaiveDate, time: NaiveTime) -> DateTime<Utc> {
        use chrono::{Local, TimeZone};
        let naive = date.and_time(time);
        match self {
            Zone::Local => Local
                .from_local_datetime(&naive)
                .earliest()
                .map(|t| t.with_timezone(&Utc))
                .unwrap_or_else(|| naive.and_utc()),
            Zone::Utc => naive.and_utc(),
        }
    }
}
