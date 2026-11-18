//! The Blocklists screen: the Distraction List categories and the Work List with their
//! domains and apps, what is pending, and the edits, under **tighten now, loosen tomorrow**.
//!
//! Edits go to a [`Rules`]: the account's backend when this Device is signed in (it decides,
//! and loosening waits until tomorrow), else the local enforcement service, which accepts
//! only tightening edits.

use chrono::{DateTime, Duration, Utc};
use cloister_core::{
    distraction_ids_in_force, effective_entries, BlocklistKind, Catalog, Command, EntryKind, State,
};
use cloister_enforcer::ipc::{Request, Response};

use crate::enforcer::Enforcer;
use crate::format::{clock, Zone};
use crate::notice::Notice;

/// Where Blocklist edits are decided.
pub trait Rules {
    /// The state and the best estimate of server time now.
    fn state(&self) -> Result<(State, DateTime<Utc>), Notice>;
    /// Apply `command`; a rejection comes back as its stable reason code.
    fn send(&self, command: &Command) -> Result<Result<(), String>, Notice>;
}

impl<R: Rules + ?Sized> Rules for Box<R> {
    fn state(&self) -> Result<(State, DateTime<Utc>), Notice> {
        (**self).state()
    }

    fn send(&self, command: &Command) -> Result<Result<(), String>, Notice> {
        (**self).send(command)
    }
}

/// The local enforcement service: Blocklist additions and opting back in only.
pub struct ServiceRules<E>(pub E);

impl<E: Enforcer> Rules for ServiceRules<E> {
    fn state(&self) -> Result<(State, DateTime<Utc>), Notice> {
        match self.0.send(&Request::Status) {
            Ok(Response::Status {
                server_time,
                blocks,
                blocklists,
                ..
            }) => Ok((
                State {
                    blocks,
                    blocklist_entries: blocklists.entries,
                    blocklist_removals: blocklists.removals,
                    blocklist_opt_outs: blocklists.opt_outs,
                    ..State::default()
                },
                server_time,
            )),
            _ => Err(Notice::unreachable()),
        }
    }

    fn send(&self, command: &Command) -> Result<Result<(), String>, Notice> {
        match self.0.send(&Request::Command {
            command: command.clone(),
        }) {
            Ok(Response::Accepted { .. }) => Ok(Ok(())),
            Ok(Response::Rejected { reason }) => Ok(Err(reason)),
            _ => Err(Notice::unreachable()),
        }
    }
}

/// The screen's model.
pub struct Blocklists<R> {
    rules: R,
    zone: Zone,
    state: State,
    now: DateTime<Utc>,
    reachable: bool,
}

/// What the Blocklists screen shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlocklistsView {
    pub reachable: bool,
    /// Social, News, Video, Shopping, Forums.
    pub distraction: Vec<ListView>,
    pub work: ListView,
    /// Loosening changes not yet in force, soonest first.
    pub pending: Vec<PendingChange>,
    /// The running Quit-Social Block, if any: when it ends.
    pub quit_social_ends: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListView {
    pub id: String,
    pub title: String,
    /// Included in every Block (a Distraction List category not opted out; the Work List
    /// is only blocked when a Block or Evening Lockdown asks for it).
    pub on: bool,
    /// "Off from tomorrow" while an opt-out is pending.
    pub badge: Option<String>,
    pub domains: Vec<EntryView>,
    pub apps: Vec<EntryView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryView {
    pub entry: String,
    /// "Takes effect tomorrow" while its removal is pending.
    pub badge: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingChange {
    /// "reddit.com leaves Forums."
    pub what: String,
    /// "Takes effect tomorrow at 00:00."
    pub when: String,
    pub at: DateTime<Utc>,
}

/// The name a Blocklist is shown under.
pub fn title(id: &str) -> String {
    match id {
        "work" => "Work List".into(),
        other => {
            let mut chars = other.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

/// What the user typed for a site, made into a domain: no scheme, path, port or `www.`.
pub fn site_input(input: &str) -> String {
    let s = input.trim();
    let s = s.split_once("://").map_or(s, |(_, rest)| rest);
    let s = s.split(['/', '?', '#']).next().unwrap_or_default();
    let s = s.rsplit_once('@').map_or(s, |(_, host)| host);
    let s = s.split(':').next().unwrap_or_default();
    let lower = s.to_ascii_lowercase();
    lower.strip_prefix("www.").unwrap_or(&lower).to_string()
}

/// "Takes effect tomorrow at 00:00." / "Takes effect on Wed 7 Jan at 00:00."
pub fn takes_effect(at: DateTime<Utc>, now: DateTime<Utc>, zone: &Zone) -> String {
    let date = |t: DateTime<Utc>| match zone {
        Zone::Local => t.with_timezone(&chrono::Local).date_naive(),
        Zone::Utc => t.date_naive(),
    };
    if date(at) == date(now) + Duration::days(1) {
        format!("Takes effect tomorrow at {}.", clock(at, zone))
    } else {
        format!(
            "Takes effect on {} at {}.",
            date(at).format("%a %-d %b"),
            clock(at, zone)
        )
    }
}

const TOMORROW: &str = "Takes effect tomorrow";

impl<R: Rules> Blocklists<R> {
    pub fn new(rules: R) -> Self {
        Blocklists {
            rules,
            zone: Zone::Local,
            state: State::default(),
            now: Utc::now(),
            reachable: false,
        }
    }

    pub fn in_zone(mut self, zone: Zone) -> Self {
        self.zone = zone;
        self
    }

    pub fn refresh(&mut self) {
        match self.rules.state() {
            Ok((state, now)) => {
                self.state = state;
                self.now = now;
                self.reachable = true;
            }
            Err(_) => self.reachable = false,
        }
    }

    pub fn view(&self) -> BlocklistsView {
        let (state, now) = (&self.state, self.now);
        let in_force = distraction_ids_in_force(state, now);
        let list = |id: &str, on: bool| {
            let entries = |kind: EntryKind| {
                effective_entries(state, id, kind, now)
                    .into_iter()
                    .map(|entry| {
                        let pending = state
                            .blocklist_removals
                            .iter()
                            .any(|r| r.blocklist == id && r.kind == kind && r.entry == entry);
                        EntryView {
                            badge: pending.then(|| TOMORROW.to_string()),
                            entry,
                        }
                    })
                    .collect()
            };
            let opting_out = state
                .blocklist_opt_outs
                .iter()
                .any(|o| o.blocklist == id && o.is_pending(now));
            ListView {
                id: id.to_string(),
                title: title(id),
                on,
                badge: opting_out.then(|| "Off from tomorrow".to_string()),
                domains: entries(EntryKind::Domain),
                apps: entries(EntryKind::App),
            }
        };
        let catalog = Catalog::builtin();
        let distraction = catalog
            .blocklists
            .iter()
            .filter(|l| l.kind == BlocklistKind::Distraction)
            .map(|l| list(&l.id, in_force.contains(&l.id)))
            .collect();
        let work = list("work", false);

        let mut pending: Vec<PendingChange> = state
            .blocklist_removals
            .iter()
            .filter(|r| r.is_pending(now))
            .map(|r| PendingChange {
                what: format!("{} leaves {}.", r.entry, title(&r.blocklist)),
                when: takes_effect(r.effective_at, now, &self.zone),
                at: r.effective_at,
            })
            .chain(
                state
                    .blocklist_opt_outs
                    .iter()
                    .filter(|o| o.is_pending(now))
                    .map(|o| PendingChange {
                        what: format!("{} is switched off.", title(&o.blocklist)),
                        when: takes_effect(o.effective_at, now, &self.zone),
                        at: o.effective_at,
                    }),
            )
            .collect();
        pending.sort_by(|a, b| a.at.cmp(&b.at).then(a.what.cmp(&b.what)));

        BlocklistsView {
            reachable: self.reachable,
            distraction,
            work,
            pending,
            quit_social_ends: state
                .blocks
                .iter()
                .filter(|b| b.quit_social && b.is_active(now))
                .map(|b| b.ends_at)
                .max(),
        }
    }

    /// Add a site (as typed: a URL is fine) or an app. Applies immediately.
    pub fn add(&mut self, blocklist: &str, kind: EntryKind, input: &str) -> Result<Notice, Notice> {
        let entry = match kind {
            EntryKind::Domain => site_input(input),
            EntryKind::App => input.trim().to_string(),
        };
        let shown = entry.clone();
        self.send(Command::AddBlocklistEntry {
            blocklist: blocklist.into(),
            kind,
            entry,
        })?;
        Ok(Notice::recorded(format!(
            "{shown} is in {} from now.",
            title(blocklist)
        )))
    }

    /// Remove a site or app. Stored as pending; takes effect tomorrow.
    pub fn remove(
        &mut self,
        blocklist: &str,
        kind: EntryKind,
        entry: &str,
    ) -> Result<Notice, Notice> {
        self.send(Command::RemoveBlocklistEntry {
            blocklist: blocklist.into(),
            kind,
            entry: entry.into(),
        })?;
        Ok(Notice::what_is_so("Removals take effect tomorrow."))
    }

    /// Switch a Distraction List category off, from tomorrow.
    pub fn opt_out(&mut self, blocklist: &str) -> Result<Notice, Notice> {
        self.send(Command::OptOutOfBlocklist {
            blocklist: blocklist.into(),
        })?;
        Ok(Notice::what_is_so(format!(
            "{} is switched off from tomorrow.",
            title(blocklist)
        )))
    }

    /// Switch a category back on. Applies immediately.
    pub fn opt_in(&mut self, blocklist: &str) -> Result<Notice, Notice> {
        self.send(Command::OptInToBlocklist {
            blocklist: blocklist.into(),
        })?;
        Ok(Notice::recorded(format!("{} is on.", title(blocklist))))
    }

    /// Start the 30-day Quit-Social Block (the caller has confirmed it can't be undone).
    pub fn start_quit_social(&mut self) -> Result<Notice, Notice> {
        let id = format!(
            "quit-social-{:x}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        self.send(Command::StartQuitSocialBlock {
            block_id: id,
            confirm_long: true,
        })?;
        let ends = self.view().quit_social_ends;
        Ok(Notice::recorded(match ends {
            Some(t) => format!(
                "Quit-Social Block started. Ends on {} at {}.",
                match self.zone {
                    Zone::Local => t
                        .with_timezone(&chrono::Local)
                        .format("%a %-d %b")
                        .to_string(),
                    Zone::Utc => t.format("%a %-d %b").to_string(),
                },
                clock(t, &self.zone)
            ),
            None => "Quit-Social Block started.".into(),
        }))
    }

    fn send(&mut self, command: Command) -> Result<(), Notice> {
        let result = self.rules.send(&command);
        self.refresh();
        match result? {
            Ok(()) => Ok(()),
            Err(reason) => Err(Notice::refusal(&reason, None, &self.zone)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_sites_become_domains() {
        assert_eq!(
            site_input(" https://www.Reddit.com/r/rust?x=1 "),
            "reddit.com"
        );
        assert_eq!(
            site_input("news.ycombinator.com:443"),
            "news.ycombinator.com"
        );
        assert_eq!(site_input("figma.com"), "figma.com");
    }

    #[test]
    fn blocklists_have_their_glossary_names() {
        assert_eq!(title("work"), "Work List");
        assert_eq!(title("social"), "Social");
    }
}
