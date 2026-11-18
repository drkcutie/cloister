//! First run on the desktop: a short philosophy, sign-in, installing the enforcement service
//! (one administrator prompt), the Distraction List, Workdays and times, and the first Day Plan.
//! Steps already done (signed in, service running, settings set) are skipped, so onboarding can
//! be resumed or rerun safely.

use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;

use cloister_core::Catalog;
use cloister_core::Command;
use cloister_enforcer::install::{elevated_install, Os};

use crate::blocklists::title;
use crate::notice::Notice;

/// One page of the philosophy: why Cloister is strict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub title: &'static str,
    pub body: &'static str,
}

/// The philosophy, in three screens.
pub const PHILOSOPHY: [Page; 3] = [
    Page {
        title: "Deep work is rare, and valuable.",
        body: "Focus without distraction on a hard problem is the work that moves things. \
               It can't be done in the gaps between notifications.",
    },
    Page {
        title: "Willpower isn't the plan. Rules are.",
        body: "A Deep Session can't be cancelled, on any Device, until it ends. Rules tighten \
               at once and loosen only tomorrow, so a weak moment can't undo a good decision.",
    },
    Page {
        title: "Work ends, on purpose.",
        body: "At Shutdown Time you capture Loose Ends and plan tomorrow. Then work tools stay \
               closed until morning. Rest is part of the method.",
    },
];

/// Where onboarding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingStep {
    /// Page `n` of [`PHILOSOPHY`].
    Philosophy(usize),
    SignIn,
    /// Installing the enforcement service.
    Service,
    DistractionList,
    /// Workdays, Workday Start, Shutdown Time.
    Workdays,
    /// The first Day Plan; the last step.
    DayPlan,
}

/// What is already set up on this Device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Progress {
    pub signed_in: bool,
    pub service_running: bool,
    pub settings_set: bool,
}

/// The onboarding walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Onboarding {
    step: OnboardingStep,
    history: Vec<OnboardingStep>,
}

impl Default for Onboarding {
    fn default() -> Self {
        Self::new()
    }
}

impl Onboarding {
    pub fn new() -> Self {
        Onboarding {
            step: OnboardingStep::Philosophy(0),
            history: Vec::new(),
        }
    }

    pub fn step(&self) -> OnboardingStep {
        self.step
    }

    /// Move to the next step that isn't done yet. `None` once past the last step (onboarding
    /// is finished; the current step stays the last one).
    pub fn next(&mut self, progress: &Progress) -> Option<OnboardingStep> {
        use OnboardingStep::*;
        let mut candidate = self.step;
        loop {
            candidate = match candidate {
                Philosophy(n) if n + 1 < PHILOSOPHY.len() => Philosophy(n + 1),
                Philosophy(_) => SignIn,
                SignIn => Service,
                Service => DistractionList,
                DistractionList => Workdays,
                Workdays => DayPlan,
                DayPlan => return None,
            };
            let done = match candidate {
                SignIn => progress.signed_in,
                Service => progress.service_running,
                Workdays => progress.settings_set,
                _ => false,
            };
            if !done {
                self.history.push(self.step);
                self.step = candidate;
                return Some(candidate);
            }
        }
    }

    /// Back to the step shown before this one.
    pub fn back(&mut self) -> OnboardingStep {
        if let Some(previous) = self.history.pop() {
            self.step = previous;
        }
        self.step
    }
}

/// The Workdays step: which days are Workdays, when work starts and when it ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkdaysForm {
    /// ISO weekdays, 1 = Monday.
    pub workdays: BTreeSet<u8>,
    pub workday_start: String,
    pub shutdown_time: String,
    /// IANA time zone the times are in (this computer's).
    pub time_zone: String,
}

const DAY_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// The short name of ISO weekday `day` (1 = Monday).
pub fn day_name(day: u8) -> &'static str {
    DAY_NAMES[usize::from(day.clamp(1, 7) - 1)]
}

impl WorkdaysForm {
    /// Monday to Friday, 09:00 to 17:00, in `time_zone`.
    pub fn new(time_zone: &str) -> Self {
        WorkdaysForm {
            workdays: (1..=5).collect(),
            workday_start: "09:00".into(),
            shutdown_time: "17:00".into(),
            time_zone: time_zone.into(),
        }
    }

    /// This computer's time zone, or UTC when it can't be read.
    pub fn local() -> Self {
        Self::new(&iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into()))
    }

    pub fn toggle(&mut self, day: u8) {
        if !self.workdays.remove(&day) {
            self.workdays.insert(day);
        }
    }

    /// The settings command, or a calm notice saying what to fix.
    pub fn command(&self) -> Result<Command, Notice> {
        let times = "Times are written like 09:00 or 17:30.";
        let start = parse_time(&self.workday_start).ok_or_else(|| Notice::what_is_so(times))?;
        let shutdown = parse_time(&self.shutdown_time).ok_or_else(|| Notice::what_is_so(times))?;
        if self.workdays.is_empty() {
            return Err(Notice::what_is_so("Choose at least one Workday."));
        }
        if shutdown <= start {
            return Err(Notice::what_is_so(
                "Shutdown Time comes after Workday Start.",
            ));
        }
        Ok(Command::ChangeSettings {
            workdays: self.workdays.iter().copied().collect(),
            workday_start: hhmm(start),
            shutdown_time: hhmm(shutdown),
            time_zone: Some(self.time_zone.clone()),
        })
    }

    /// One line saying what the settings mean, e.g. "Mon to Fri, 09:00 to 17:00. Evening
    /// Lockdown starts at 17:00."
    pub fn summary(&self) -> String {
        let days: Vec<u8> = self.workdays.iter().copied().collect();
        let consecutive = days.windows(2).all(|w| w[1] == w[0] + 1);
        let days = if days.len() >= 3 && consecutive {
            format!(
                "{} to {}",
                day_name(days[0]),
                day_name(*days.last().unwrap())
            )
        } else {
            days.iter()
                .map(|d| day_name(*d))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let start = parse_time(&self.workday_start).map(hhmm);
        let end = parse_time(&self.shutdown_time).map(hhmm);
        match (start, end) {
            (Some(s), Some(e)) => format!("{days}, {s} to {e}. Evening Lockdown starts at {e}."),
            _ => days,
        }
    }
}

/// "9:00", "09:00" -> minutes after midnight.
fn parse_time(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60 && m.to_string().len() <= 2 && s.trim().len() <= 5).then_some(h * 60 + m)
}

fn hhmm(minutes: u32) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// One Distraction List category, as onboarding shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub id: String,
    pub title: String,
    /// A few of its sites.
    pub examples: Vec<String>,
}

/// The Distraction List, on by default for every Deep Session.
pub fn distraction_list() -> Vec<Category> {
    let catalog = Catalog::builtin();
    catalog
        .distraction_ids()
        .filter_map(|id| catalog.get(&id))
        .map(|list| {
            // The best-known sites first: those with mirrors in the related-domain map.
            let mut domains: Vec<&String> = list.domains.iter().collect();
            domains.sort_by_key(|d| !catalog.related_domains.contains_key(d.as_str()));
            Category {
                id: list.id.clone(),
                title: title(&list.id),
                examples: domains.into_iter().take(4).cloned().collect(),
            }
        })
        .collect()
}

/// The sign-in step: an email, then the 6-digit code from the email (or its link).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SignInForm {
    pub email: String,
    pub code: String,
}

impl SignInForm {
    pub fn email(&self) -> &str {
        self.email.trim()
    }

    pub fn can_send(&self) -> bool {
        let email = self.email();
        email.len() >= 3 && email.contains('@') && !email.contains(char::is_whitespace)
    }

    pub fn can_verify(&self) -> bool {
        self.code.len() == 6 && self.code.bytes().all(|b| b.is_ascii_digit())
    }
}

/// The command that installs the enforcement service with one administrator prompt:
/// `cloisterd` next to this app (in the app bundle, install dir or build dir).
pub fn install_service_command() -> io::Result<(String, Vec<String>)> {
    let app = std::env::current_exe()?;
    let dir = app
        .parent()
        .ok_or_else(|| io::Error::other("the app has no directory"))?;
    let daemon = dir.join(if cfg!(windows) {
        "cloisterd.exe"
    } else {
        "cloisterd"
    });
    if !daemon.exists() {
        return Err(io::Error::other(format!(
            "cloisterd isn't next to the app ({})",
            daemon.display()
        )));
    }
    Ok(elevated_install(
        Os::current(),
        &daemon.to_string_lossy(),
        &app.to_string_lossy(),
    ))
}

/// Remembers that onboarding finished on this computer.
#[derive(Debug, Clone)]
pub struct OnboardingMarker(PathBuf);

impl OnboardingMarker {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        OnboardingMarker(path.into())
    }

    /// `$CLOISTER_ONBOARDED`, else a file in the user's data directory.
    pub fn from_env() -> Self {
        Self::new(
            std::env::var_os("CLOISTER_ONBOARDED")
                .map(PathBuf::from)
                .unwrap_or_else(|| crate::devices::data_dir().join("onboarded")),
        )
    }

    pub fn is_done(&self) -> bool {
        self.0.exists()
    }

    pub fn mark_done(&self) -> io::Result<()> {
        if let Some(dir) = self.0.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&self.0, b"")
    }
}
