use chrono::{DateTime, Duration, Utc};
use cloister_core::{Block, Command, Lockdown};
use cloister_enforcer::ipc::{Request, Response};

use crate::begin::BeginForm;
use crate::enforcer::Enforcer;
use crate::format::{clock, day, Zone};
use crate::notice::Notice;

/// The app's live picture of today, refreshed from the enforcement service. The service is
/// the only source of truth: the app starts Blocks only by asking it, so quitting the app
/// changes nothing and reopening it shows whatever is running.
pub struct Today<E> {
    enforcer: E,
    zone: Zone,
    blocks: Vec<Block>,
    reachable: bool,
    /// Server time minus local time at the last refresh.
    offset: Duration,
    /// Evening Lockdown or a Rest Day, as the service saw it at the last refresh.
    lockdown: Option<Lockdown>,
    next_workday_start: Option<DateTime<Utc>>,
}

/// What the screen should show now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    /// Whether the last refresh reached the enforcement service.
    pub reachable: bool,
    pub active: Option<ActiveSession>,
    /// The latest ended Deep Session without a Depth Rating. Stays until answered.
    pub rating: Option<RatingPrompt>,
    /// Deep Sessions started this ISO week, at full length.
    pub deep_hours_this_week: Duration,
    /// The running Quit-Social Block, if any: when it ends. It runs alongside everything
    /// else, so it is shown as a tag, never as the hero.
    pub quit_social_ends: Option<DateTime<Utc>>,
    /// Set during Evening Lockdown and on Rest Days: the workday has ended.
    pub ended: Option<WorkdayEnded>,
}

impl View {
    /// Whether the app wears its Night theme: during Evening Lockdown and on Rest Days.
    pub fn night(&self) -> bool {
        self.ended.is_some()
    }
}

/// The "workday has ended" screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkdayEnded {
    pub headline: &'static str,
    /// A Rest Day rather than an evening.
    pub rest_day: bool,
    pub next_workday_start: Option<DateTime<Utc>>,
    /// "Workday Start Tue 6 Jan, 09:00." (or a note that no Workday is set).
    pub next_start_text: String,
}

/// The running Block (a Deep Session unless `deep` is false).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveSession {
    pub block_id: String,
    pub deep: bool,
    pub intent: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub remaining: Duration,
}

impl ActiveSession {
    /// How much has elapsed, from 0 to 1.
    pub fn progress(&self) -> f32 {
        let total = (self.ends_at - self.starts_at).num_milliseconds().max(1) as f32;
        (1.0 - self.remaining.num_milliseconds() as f32 / total).clamp(0.0, 1.0)
    }
}

/// A Deep Session waiting for its Depth Rating.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RatingPrompt {
    pub block_id: String,
    pub intent: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

/// What committing the Begin form would mean, shown before the hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub length: Duration,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub deep_hours_after: Duration,
}

impl<E: Enforcer> Today<E> {
    pub fn new(enforcer: E) -> Self {
        Today {
            enforcer,
            zone: Zone::Local,
            blocks: vec![],
            reachable: false,
            offset: Duration::zero(),
            lockdown: None,
            next_workday_start: None,
        }
    }

    /// Show times in `zone` instead of this Device's local time.
    pub fn in_zone(mut self, zone: Zone) -> Self {
        self.zone = zone;
        self
    }

    pub fn zone(&self) -> Zone {
        self.zone
    }

    /// Ask the service for the current Blocks. `local_now` is this Device's clock, used
    /// only to keep the countdown moving between refreshes.
    pub fn refresh(&mut self, local_now: DateTime<Utc>) {
        match self.enforcer.send(&Request::Status) {
            Ok(Response::Status {
                server_time,
                blocks,
                lockdown,
                next_workday_start,
                ..
            }) => {
                self.offset = server_time - local_now;
                self.blocks = blocks;
                self.lockdown = lockdown;
                self.next_workday_start = next_workday_start;
                self.reachable = true;
            }
            _ => self.reachable = false,
        }
    }

    pub fn server_now(&self, local_now: DateTime<Utc>) -> DateTime<Utc> {
        local_now + self.offset
    }

    pub fn view(&self, local_now: DateTime<Utc>) -> View {
        let now = self.server_now(local_now);
        let active = self
            .blocks
            .iter()
            .filter(|b| b.is_active(now) && !b.quit_social)
            .max_by_key(|b| b.ends_at)
            .map(|b| ActiveSession {
                block_id: b.id.clone(),
                deep: b.deep,
                intent: b.intent.clone(),
                starts_at: b.starts_at,
                ends_at: b.ends_at,
                remaining: b.ends_at - now,
            });
        let rating = self
            .blocks
            .iter()
            .filter(|b| b.deep && b.ends_at <= now)
            .max_by_key(|b| b.ends_at)
            .filter(|b| b.depth_rating.is_none())
            .map(|b| RatingPrompt {
                block_id: b.id.clone(),
                intent: b.intent.clone(),
                starts_at: b.starts_at,
                ends_at: b.ends_at,
            });
        View {
            reachable: self.reachable,
            active,
            rating,
            deep_hours_this_week: self.deep_hours_since(self.zone.week_start(now)),
            quit_social_ends: self
                .blocks
                .iter()
                .filter(|b| b.quit_social && b.is_active(now))
                .map(|b| b.ends_at)
                .max(),
            ended: self.ended(now),
        }
    }

    fn ended(&self, now: DateTime<Utc>) -> Option<WorkdayEnded> {
        let lockdown = self.lockdown?;
        // The service's answer is at most a refresh old; once Workday Start passes, it's Day.
        if self.next_workday_start.is_some_and(|start| start <= now) {
            return None;
        }
        Some(WorkdayEnded {
            headline: "The workday has ended.",
            rest_day: lockdown == Lockdown::RestDay,
            next_workday_start: self.next_workday_start,
            next_start_text: match self.next_workday_start {
                Some(t) => format!(
                    "Workday Start {}, {}.",
                    day(t, &self.zone),
                    clock(t, &self.zone)
                ),
                None => "No Workday is set.".into(),
            },
        })
    }

    fn deep_hours_since(&self, since: DateTime<Utc>) -> Duration {
        self.blocks
            .iter()
            .filter(|b| b.deep && b.starts_at >= since)
            .map(|b| b.ends_at - b.starts_at)
            .fold(Duration::zero(), |a, b| a + b)
    }

    /// Start, end and Deep Hours after, if `form` were committed now.
    pub fn summary(&self, form: &BeginForm, local_now: DateTime<Utc>) -> Option<Summary> {
        let length = Duration::minutes(form.minutes()?);
        let starts_at = self.server_now(local_now);
        let before = self.view(local_now).deep_hours_this_week;
        Some(Summary {
            length,
            starts_at,
            ends_at: starts_at + length,
            deep_hours_after: if form.deep { before + length } else { before },
        })
    }

    /// Start a Block through the enforcement service.
    pub fn begin(&mut self, form: &BeginForm, local_now: DateTime<Utc>) -> Result<(), Notice> {
        let minutes = match (form.missing(), form.minutes()) {
            (None, Some(minutes)) => minutes,
            _ => return Err(Notice::what_is_so("The Block isn't ready to begin.")),
        };
        let command = form.command(new_block_id(), minutes);
        // A refusal during Evening Lockdown or a Rest Day names when work starts again.
        self.send(command, self.next_workday_start, local_now)
    }

    /// Record the Depth Rating (1 to 5) of an ended Deep Session.
    pub fn rate(
        &mut self,
        block_id: &str,
        rating: u8,
        local_now: DateTime<Utc>,
    ) -> Result<(), Notice> {
        let ends_at = self
            .blocks
            .iter()
            .find(|b| b.id == block_id)
            .map(|b| b.ends_at);
        let command = Command::RecordDepthRating {
            block_id: block_id.into(),
            rating,
        };
        self.send(command, ends_at, local_now)
    }

    fn send(
        &mut self,
        command: Command,
        about: Option<DateTime<Utc>>,
        local_now: DateTime<Utc>,
    ) -> Result<(), Notice> {
        let response = self.enforcer.send(&Request::Command { command });
        let result = match response {
            Ok(Response::Accepted { .. }) => Ok(()),
            Ok(Response::Rejected { reason }) => Err(Notice::refusal(&reason, about, &self.zone)),
            Ok(Response::Status { .. }) => Err(Notice::refusal("", about, &self.zone)),
            Err(_) => Err(Notice::unreachable()),
        };
        self.refresh(local_now);
        result
    }
}

fn new_block_id() -> String {
    let nanos = Utc::now().timestamp_nanos_opt().unwrap_or_default();
    format!("blk-{nanos:x}-{:x}", std::process::id())
}
