//! Calm notices. A refusal is never an error: it says what is so, and names the time it
//! ends or takes effect. No "sorry", "oops", "error" or "not allowed"; full stops only.

use chrono::{DateTime, Utc};

use crate::format::{clock, Zone};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    /// Sunk background: what is so.
    WhatIsSo,
    /// Brass: what was recorded.
    Recorded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub kind: NoticeKind,
    pub text: String,
}

impl Notice {
    pub fn what_is_so(text: impl Into<String>) -> Self {
        Notice {
            kind: NoticeKind::WhatIsSo,
            text: text.into(),
        }
    }

    pub fn recorded(text: impl Into<String>) -> Self {
        Notice {
            kind: NoticeKind::Recorded,
            text: text.into(),
        }
    }

    pub fn unreachable() -> Self {
        Self::what_is_so(
            "The enforcement service isn't running on this Mac. Blocks start once it is.",
        )
    }

    /// The notice for a rejection `code` from the service. `ends_at` is the end of the
    /// Block the refusal is about, when there is one.
    pub fn refusal(code: &str, ends_at: Option<DateTime<Utc>>, zone: &Zone) -> Self {
        let ends = |then: &str| match ends_at {
            Some(t) => format!("Ends at {}. {then}", clock(t, zone)),
            None => then.to_string(),
        };
        Self::what_is_so(match code {
            "block_cannot_be_cancelled"
            | "block_cannot_be_shortened"
            | "loosening_not_accepted" => ends("Can't be cancelled."),
            "deep_session_not_ended" => ends("Rate it then."),
            "confirmation_required" => {
                "Blocks over 4 hours need the confirmation that they can't be undone.".into()
            }
            "block_too_long" => "Blocks are 24 hours at most.".into(),
            "invalid_length" => "Choose a length.".into(),
            "depth_rating_already_recorded" => "This Depth Rating is already recorded.".into(),
            "storage_unavailable" => "The Block wasn't saved, so it hasn't started.".into(),
            _ => "Not accepted. Nothing has changed.".into(),
        })
    }
}
