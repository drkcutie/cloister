//! What the Cloister desktop app shows and does, independent of any UI toolkit.
//!
//! The GPUI app is a thin skin over this crate: [`Today`] talks to the enforcement service
//! over IPC and answers "what should the screen show now?" as a [`View`]; [`BeginForm`] holds
//! the Begin screen's choices; [`Hold`] is the two-second hold-to-commit; [`Notice`] turns
//! refusals into calm copy.

mod begin;
mod blocklists;
mod day_plan;
mod devices;
mod enforcer;
pub mod format;
mod hold;
mod notice;
mod ritual;
mod scoreboard;
mod settings;
mod today;

pub use begin::{BeginForm, Length, Missing};
pub use blocklists::{
    site_input, takes_effect, title, Blocklists, BlocklistsView, EntryView, ListView,
    PendingChange, Rules, ServiceRules,
};
pub use day_plan::{DayPlanEditor, EditorSlot, PlanBackend, DEFAULT_WORKDAY, STEP_MINUTES};
pub use devices::{devices_view, open_account, AccountRules, DeviceRow, DevicesView};
pub use enforcer::{Enforcer, SocketEnforcer};
pub use format::Zone;
pub use hold::Hold;
pub use notice::{Notice, NoticeKind};
pub use ritual::{
    deep_hours_this_week, shutdown_delay, shutdown_warning, use_shutdown_delay, DelayView, Review,
    ReviewSession, RitualFlow, RitualStep, ShutdownWarning, DEFAULT_WEEKLY_TARGET,
};
pub use scoreboard::{deep_hours_figure, scoreboard, DayBar, Record, RecordKind, Scoreboard};
pub use settings::{pending_changes, weekday_names, SettingsForm};
pub use today::{ActiveSession, RatingPrompt, Summary, Today, View, WorkdayEnded};
