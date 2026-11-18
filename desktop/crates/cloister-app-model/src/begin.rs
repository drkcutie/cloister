use cloister_core::{Command, CONFIRM_LONG_AFTER, MAX_BLOCK};

/// How long the Block will be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Length {
    /// One of [`BeginForm::PRESETS`], in minutes.
    Preset(i64),
    /// What was typed into the custom field, in minutes.
    Custom(String),
}

/// What still stands between the form and the hold-to-commit button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missing {
    /// A Deep Session needs an Intent.
    Intent,
    /// The custom length is empty.
    Length,
    /// Over 4 hours: the "can't be undone" confirmation hasn't been ticked.
    LongConfirmation,
}

/// The Begin screen's choices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeginForm {
    pub intent: String,
    pub length: Length,
    /// "Counts as deep work". On by default: a plain Block is the exception.
    pub deep: bool,
    /// The "can't be undone" confirmation for Blocks over 4 hours.
    pub long_confirmed: bool,
}

impl Default for BeginForm {
    fn default() -> Self {
        BeginForm {
            intent: String::new(),
            length: Length::Preset(90),
            deep: true,
            long_confirmed: false,
        }
    }
}

impl BeginForm {
    pub const PRESETS: [i64; 4] = [45, 60, 90, 120];

    pub fn minutes(&self) -> Option<i64> {
        match &self.length {
            Length::Preset(m) => Some(*m),
            Length::Custom(text) => text.trim().parse().ok().filter(|m| *m > 0),
        }
    }

    /// Whether `text` may stand in the custom length field: whole minutes, at most 24 hours.
    /// Anything else is refused as it is typed, so a longer Block can't be entered at all.
    pub fn custom_input_allowed(text: &str) -> bool {
        text.is_empty()
            || (text.len() <= 4
                && text.bytes().all(|b| b.is_ascii_digit())
                && text
                    .parse::<i64>()
                    .is_ok_and(|m| m <= MAX_BLOCK.num_minutes()))
    }

    /// Whether the chosen length needs the "can't be undone" confirmation.
    pub fn is_long(&self) -> bool {
        self.minutes()
            .is_some_and(|m| m > CONFIRM_LONG_AFTER.num_minutes())
    }

    /// The first thing still missing, or `None` when the Block can be committed.
    pub fn missing(&self) -> Option<Missing> {
        if self.deep && self.intent.trim().is_empty() {
            Some(Missing::Intent)
        } else if self.minutes().is_none() {
            Some(Missing::Length)
        } else if self.is_long() && !self.long_confirmed {
            Some(Missing::LongConfirmation)
        } else {
            None
        }
    }

    pub(crate) fn command(&self, block_id: String, minutes: i64) -> Command {
        Command::StartBlock {
            block_id,
            minutes,
            deep: self.deep,
            blocklists: vec![],
            confirm_long: self.long_confirmed && self.is_long(),
            client_time: None,
            intent: self.deep.then(|| self.intent.trim().to_string()),
            extra_domains: vec![],
            extra_apps: vec![],
        }
    }
}
