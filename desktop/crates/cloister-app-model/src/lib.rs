//! What the Cloister desktop app shows and does, independent of any UI toolkit.
//!
//! The GPUI app is a thin skin over this crate: [`Today`] talks to the enforcement service
//! over IPC and answers "what should the screen show now?" as a [`View`]; [`BeginForm`] holds
//! the Begin screen's choices; [`Hold`] is the two-second hold-to-commit; [`Notice`] turns
//! refusals into calm copy.

mod begin;
mod enforcer;
pub mod format;
mod hold;
mod notice;
mod today;

pub use begin::{BeginForm, Length, Missing};
pub use enforcer::{Enforcer, SocketEnforcer};
pub use format::Zone;
pub use hold::Hold;
pub use notice::{Notice, NoticeKind};
pub use today::{ActiveSession, RatingPrompt, Summary, Today, View};
