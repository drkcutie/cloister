//! Calm notices. A refusal is never an error: it says what is so, and names the time it
//! ends or takes effect. No "sorry", "oops", "error" or "not allowed"; full stops only.

use chrono::{DateTime, Utc};
use cloister_core::{Details, Refusal};

use crate::format::{clock, day, Zone};

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
            "The enforcement service isn't running on this Device. Blocks start once it is.",
        )
    }

    /// The notice for a rejection `code` (from the service or the backend), from the one
    /// copy map ([`Refusal`]). `ends_at` is the end of the Block the refusal is about, when
    /// there is one (for Evening Lockdown and Rest Days: the next Workday Start). A code no
    /// layer gives gets calm generic copy; the code goes to the log, never on screen.
    pub fn refusal(code: &str, ends_at: Option<DateTime<Utc>>, zone: &Zone) -> Self {
        Self::refused(
            code,
            &Details {
                ends_at: ends_at.map(|t| clock(t, zone)),
                ends_on: ends_at.map(|t| day(t, zone)),
                ..Details::default()
            },
        )
    }

    /// The notice for a rejection `code` with the times it names already formatted.
    pub fn refused(code: &str, details: &Details) -> Self {
        if Refusal::from_code(code).is_none() {
            eprintln!("cloister: refused with an unknown code: {code}");
        }
        Self::what_is_so(Refusal::copy_for(code, details))
    }
}
