use cloister_core::{
    normalize_app, normalize_domain, Command, EntryKind, CONFIRM_LONG_AFTER, MAX_BLOCK,
};

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
    /// Sites blocked for this Block only, on top of the Distraction List (which is always in).
    pub extra_domains: Vec<String>,
    /// Apps blocked for this Block only.
    pub extra_apps: Vec<String>,
}

impl Default for BeginForm {
    fn default() -> Self {
        BeginForm {
            intent: String::new(),
            length: Length::Preset(90),
            deep: true,
            long_confirmed: false,
            extra_domains: Vec::new(),
            extra_apps: Vec::new(),
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

    /// Add a site (as typed: a URL is fine) or an app for this Block only. `false` when it
    /// isn't a domain or an app name; nothing is added then.
    pub fn add_extra(&mut self, kind: EntryKind, input: &str) -> bool {
        let (list, entry) = match kind {
            EntryKind::Domain => (
                &mut self.extra_domains,
                normalize_domain(&crate::blocklists::site_input(input)),
            ),
            EntryKind::App => (&mut self.extra_apps, normalize_app(input)),
        };
        let Some(entry) = entry else { return false };
        if !list.contains(&entry) {
            list.push(entry);
        }
        true
    }

    /// Take an extra off again (before the Block begins).
    pub fn remove_extra(&mut self, kind: EntryKind, entry: &str) {
        match kind {
            EntryKind::Domain => self.extra_domains.retain(|e| e != entry),
            EntryKind::App => self.extra_apps.retain(|e| e != entry),
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
            extra_domains: self.extra_domains.clone(),
            extra_apps: self.extra_apps.clone(),
        }
    }
}
