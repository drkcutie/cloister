//! Every refusal any layer can give (a rule's [`Reason`], the enforcement service or the
//! backend), with the one calm sentence each Device shows for it.
//!
//! Codes cross the socket and the network as strings; [`Refusal::from_code`] turns them back.
//! Copy follows the voice rules: it says what is so and names the time it ends or takes effect.
//! No "sorry", "oops", "error" or "not allowed"; full stops only; a raw code is never shown.

use crate::Reason;

/// Why a request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// A rule of the engine (the same in every engine).
    Rule(Reason),
    /// The enforcement service: a loosening goes through the account, which applies it from
    /// tomorrow (a Blocklist removal or opt-out, settings, a Day Plan or Schedule change).
    TakesEffectTomorrow,
    /// The enforcement service: completing the ritual, a Shutdown Delay or closing a Loose End
    /// goes through the account.
    AccountRequired,
    /// The enforcement service: a synced Block no rule could have produced.
    InvalidBlock,
    /// The enforcement service couldn't save an accepted command.
    StorageUnavailable,
    /// The backend: a command it doesn't know, or with the wrong fields.
    InvalidCommand,
    /// The backend: no signed-in account.
    NotSignedIn,
    /// The backend: a Block row can't be changed directly.
    BlockCannotBeChanged,
}

/// Times and bounds the copy names, already formatted for the reader ("11:00", "Wed 4 Feb").
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Details {
    /// When what the refusal is about ends: the Block, the started Deep Session, or (for
    /// Evening Lockdown and Rest Days) the next Workday Start.
    pub ends_at: Option<String>,
    /// The day of `ends_at`, when it isn't today.
    pub ends_on: Option<String>,
    /// The earliest a deep Slot may start (now).
    pub earliest: Option<String>,
    /// The Workday's bounds, "09:00 to 17:00".
    pub workday: Option<String>,
}

impl Refusal {
    const SERVICE: [Refusal; 7] = [
        Refusal::TakesEffectTomorrow,
        Refusal::AccountRequired,
        Refusal::InvalidBlock,
        Refusal::StorageUnavailable,
        Refusal::InvalidCommand,
        Refusal::NotSignedIn,
        Refusal::BlockCannotBeChanged,
    ];

    /// The stable snake_case code.
    pub fn code(&self) -> &'static str {
        match self {
            Refusal::Rule(reason) => reason.code(),
            Refusal::TakesEffectTomorrow => "takes_effect_tomorrow",
            Refusal::AccountRequired => "account_required",
            Refusal::InvalidBlock => "invalid_block",
            Refusal::StorageUnavailable => "storage_unavailable",
            Refusal::InvalidCommand => "invalid_command",
            Refusal::NotSignedIn => "not_signed_in",
            Refusal::BlockCannotBeChanged => "block_cannot_be_changed",
        }
    }

    /// The refusal a code stands for; `None` for a code no layer gives.
    pub fn from_code(code: &str) -> Option<Self> {
        if let Ok(reason) = serde_json::from_value::<Reason>(serde_json::json!(code)) {
            return Some(Refusal::Rule(reason));
        }
        Self::SERVICE.into_iter().find(|r| r.code() == code)
    }

    /// The sentence for a code, or calm generic copy for one no layer gives (log the code).
    pub fn copy_for(code: &str, details: &Details) -> String {
        match Refusal::from_code(code) {
            Some(refusal) => refusal.copy(details),
            None => NOTHING_CHANGED.to_string(),
        }
    }

    /// The sentence a Device shows.
    pub fn copy(&self, d: &Details) -> String {
        let ends = |then: &str| match &d.ends_at {
            Some(t) => format!("Ends at {t}. {then}"),
            None => then.to_string(),
        };
        let reason = match self {
            Refusal::Rule(reason) => *reason,
            Refusal::TakesEffectTomorrow => {
                return "This goes through your account and takes effect tomorrow. Sign in on this Device to make it.".into()
            }
            Refusal::AccountRequired => {
                return "This goes through your account. Sign in on this Device to do it.".into()
            }
            Refusal::InvalidBlock => return "That Block can't be right. It wasn't taken.".into(),
            Refusal::StorageUnavailable => {
                return "It wasn't saved, so nothing has changed.".into()
            }
            Refusal::InvalidCommand => return NOTHING_CHANGED.into(),
            Refusal::NotSignedIn => return "Sign in to do this.".into(),
            Refusal::BlockCannotBeChanged => return ends("A Block can be extended, not changed."),
        };
        use Reason::*;
        match reason {
            InvalidLength => "Choose a length.".into(),
            BlockTooLong => "Blocks are 24 hours at most.".into(),
            ConfirmationRequired => {
                "Blocks over 4 hours need the confirmation that they can't be undone.".into()
            }
            DuplicateBlock => "A Block with that id already exists.".into(),
            UnknownBlock => "No Block has that id.".into(),
            UnknownBlocklist => "No Blocklist has that name.".into(),
            BlockCannotBeShortened => ends("A Block can be extended, not shortened."),
            BlockCannotBeCancelled => ends("Can't be cancelled."),
            BlockHasEnded => "That Block has already ended.".into(),
            NotADeepSession => "Only a Deep Session has a Depth Rating.".into(),
            DeepSessionNotEnded => ends("Rate it then."),
            InvalidDepthRating => "A Depth Rating is 1 to 5.".into(),
            DepthRatingAlreadyRecorded => "This Depth Rating is already recorded.".into(),
            InvalidEntry => "Enter a domain, like example.com, or an app name.".into(),
            UnknownEntry => "That Blocklist doesn't hold this entry.".into(),
            CannotOptOut => "The Work List can be edited, not switched off.".into(),
            InvalidDayPlan => "The Workday runs from Workday Start to Shutdown Time.".into(),
            InvalidSlot => "Every Slot needs a label and a time inside the Workday.".into(),
            SlotsOverlap => "Two Slots overlap. Move one.".into(),
            DuplicateSlot => "Two Slots have the same id.".into(),
            DeepSessionHasStarted => match &d.ends_at {
                Some(t) => format!("Ends at {t}. Can't be changed."),
                None => "This Deep Session has started. It can't be changed.".into(),
            },
            SlotInThePast => match &d.earliest {
                Some(t) => format!("Deep Slots start at {t} at the earliest."),
                None => "Deep Slots start from now on.".into(),
            },
            SlotCannotBeRemoved => "This Slot can be moved, not removed.".into(),
            InvalidSchedule => "A Schedule needs Workdays and a start before its end.".into(),
            UnknownTimeZone => "That time zone isn't known. Use a name like Europe/Paris.".into(),
            SchedulesOverlap => "Another Schedule already covers one of those days.".into(),
            EveningLockdown => match &d.ends_at {
                Some(t) => format!("Evening Lockdown. Deep Sessions start again at {t}."),
                None => "Evening Lockdown. Deep Sessions start again at Workday Start.".into(),
            },
            RestDay => match (&d.ends_on, &d.ends_at) {
                (Some(day), Some(t)) => {
                    format!("A Rest Day. Deep Sessions start again {day}, {t}.")
                }
                _ => "A Rest Day. Deep Sessions start again on the next Workday.".into(),
            },
            InvalidSettings => "Workday Start comes before Shutdown Time.".into(),
            TimeZoneCannotChange => "The time zone is set once, with the first settings.".into(),
            InvalidLooseEnd => "A Loose End needs a few words, 500 characters at most.".into(),
            DuplicateLooseEnd => "That Loose End is already captured.".into(),
            UnknownLooseEnd => "That Loose End is already closed.".into(),
            NoShutdownTime => "Set a Shutdown Time first.".into(),
            ShutdownDelayUsed => "No Shutdown Delay left this week.".into(),
            RitualNotDue => "No Shutdown Ritual is due.".into(),
            RitualAlreadyCompleted => "Shutdown is already complete.".into(),
            LooseEndRequired => "Capture at least one Loose End.".into(),
            DayPlanRequired => "Save tomorrow's Day Plan.".into(),
            DeepSlotRequired => "Tomorrow needs at least one deep Slot.".into(),
            WorkdayNotCovered => match &d.workday {
                Some(b) => format!("Cover the whole Workday with Slots, {b}."),
                None => "Cover the whole Workday with Slots.".into(),
            },
            ConfirmationMismatch => {
                format!("Type \"{}\" exactly.", crate::SHUTDOWN_COMPLETE)
            }
            InvalidWeeklyTarget => "A weekly target is 1 to 100 Deep Hours.".into(),
            WeeklyReviewRequired => "Set next week's target.".into(),
            WeeklyReviewNotDue => "The Weekly Review comes on the last Workday of the week.".into(),
        }
    }
}

/// Copy for a refusal no layer is known to give.
pub const NOTHING_CHANGED: &str = "Not accepted. Nothing has changed.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_round_trips() {
        for code in [
            "evening_lockdown",
            "takes_effect_tomorrow",
            "account_required",
            "block_cannot_be_changed",
        ] {
            assert_eq!(Refusal::from_code(code).unwrap().code(), code);
        }
        assert_eq!(Refusal::from_code("loosening_not_accepted"), None);
    }

    #[test]
    fn copy_names_the_time_and_never_the_code() {
        let d = Details {
            ends_at: Some("09:00".into()),
            ..Details::default()
        };
        assert_eq!(
            Refusal::copy_for("evening_lockdown", &d),
            "Evening Lockdown. Deep Sessions start again at 09:00."
        );
        assert_eq!(Refusal::copy_for("something_new", &d), NOTHING_CHANGED);
        assert!(!Refusal::copy_for("block_cannot_be_cancelled", &d).contains('_'));
    }
}
