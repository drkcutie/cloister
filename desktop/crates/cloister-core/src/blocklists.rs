//! The account's own Blocklist edits on top of the built-in catalog, under
//! **tighten now, loosen tomorrow** (ADR 0001): additions apply immediately, removals and
//! Distraction List opt-outs are stored as pending and take effect at the next day boundary.

use std::collections::BTreeSet;

use chrono::{DateTime, Duration, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::{BlocklistKind, Catalog, Reason, State};

/// What a Blocklist entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    /// A desktop or Android app: a bundle id, package name or app name.
    App,
    /// A domain; it also covers its subdomains.
    Domain,
}

/// A domain or app the account added to a Blocklist.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlocklistEntry {
    pub blocklist: String,
    pub kind: EntryKind,
    pub entry: String,
}

/// A removal from a Blocklist (of a built-in or added entry). In force from `effective_at`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlocklistRemoval {
    pub blocklist: String,
    pub kind: EntryKind,
    pub entry: String,
    pub effective_at: DateTime<Utc>,
}

impl BlocklistRemoval {
    pub fn is_pending(&self, now: DateTime<Utc>) -> bool {
        now < self.effective_at
    }

    fn matches(&self, blocklist: &str, kind: EntryKind, entry: &str) -> bool {
        self.blocklist == blocklist && self.kind == kind && self.entry == entry
    }
}

/// Opting out of a default Distraction List category. In force from `effective_at`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlocklistOptOut {
    pub blocklist: String,
    pub effective_at: DateTime<Utc>,
}

impl BlocklistOptOut {
    pub fn is_pending(&self, now: DateTime<Utc>) -> bool {
        now < self.effective_at
    }
}

/// The time zone used for day boundaries when the account hasn't set one.
pub const DEFAULT_TIME_ZONE: &str = "UTC";

/// The next local midnight after `now` in the account's time zone: when a loosening change
/// made at `now` takes effect. An unknown time zone falls back to UTC.
pub fn next_day_boundary(state: &State, now: DateTime<Utc>) -> DateTime<Utc> {
    let tz: Tz = state.time_zone.parse().unwrap_or(Tz::UTC);
    let tomorrow = now.with_timezone(&tz).date_naive() + Duration::days(1);
    let midnight = tomorrow.and_time(NaiveTime::MIN);
    let local = tz.from_local_datetime(&midnight);
    match local.latest() {
        Some(t) => t.with_timezone(&Utc),
        // Midnight skipped by a clock change: the instant it would have been on the old offset.
        None => tz
            .from_local_datetime(&(midnight - Duration::hours(1)))
            .earliest()
            .map(|t| t.with_timezone(&Utc) + Duration::hours(1))
            .unwrap_or_else(|| midnight.and_utc()),
    }
}

/// Trim, lower-case and drop a trailing dot; `None` unless the result is a plausible domain
/// (two or more labels of letters, digits and inner hyphens).
pub fn normalize_domain(input: &str) -> Option<String> {
    let d = input.trim().to_ascii_lowercase();
    let d = d.strip_suffix('.').unwrap_or(&d).to_string();
    let labels: Vec<&str> = d.split('.').collect();
    let label_ok = |l: &&str| {
        (1..=63).contains(&l.len())
            && l.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !l.starts_with('-')
            && !l.ends_with('-')
    };
    (d.len() <= 253 && labels.len() >= 2 && labels.iter().all(label_ok)).then_some(d)
}

/// Trim; `None` when blank or longer than 200 characters.
pub fn normalize_app(input: &str) -> Option<String> {
    let a = input.trim();
    (!a.is_empty() && a.chars().count() <= 200).then(|| a.to_string())
}

pub(crate) fn normalize(kind: EntryKind, input: &str) -> Result<String, Reason> {
    match kind {
        EntryKind::Domain => normalize_domain(input),
        EntryKind::App => normalize_app(input),
    }
    .ok_or(Reason::InvalidEntry)
}

/// Normalize, validate, deduplicate and sort a list of entries.
pub(crate) fn normalize_all(kind: EntryKind, inputs: &[String]) -> Result<Vec<String>, Reason> {
    let set: BTreeSet<String> = inputs
        .iter()
        .map(|i| normalize(kind, i))
        .collect::<Result<_, _>>()?;
    Ok(set.into_iter().collect())
}

fn known_blocklist(blocklist: &str) -> Result<(), Reason> {
    Catalog::builtin()
        .get(blocklist)
        .map(|_| ())
        .ok_or(Reason::UnknownBlocklist)
}

/// Entries of a Blocklist before any removal: built-in ones plus the account's additions.
fn listed(state: &State, blocklist: &str, kind: EntryKind) -> BTreeSet<String> {
    let mut set: BTreeSet<String> = Catalog::builtin()
        .get(blocklist)
        .map(|l| match kind {
            EntryKind::Domain => l.domains.clone(),
            EntryKind::App => l.apps.clone(),
        })
        .unwrap_or_default()
        .into_iter()
        .collect();
    set.extend(
        state
            .blocklist_entries
            .iter()
            .filter(|e| e.blocklist == blocklist && e.kind == kind)
            .map(|e| e.entry.clone()),
    );
    set
}

/// The entries of a Blocklist in force at `now`, sorted.
pub fn effective_entries(
    state: &State,
    blocklist: &str,
    kind: EntryKind,
    now: DateTime<Utc>,
) -> Vec<String> {
    let mut set = listed(state, blocklist, kind);
    for r in &state.blocklist_removals {
        if r.blocklist == blocklist && r.kind == kind && !r.is_pending(now) {
            set.remove(&r.entry);
        }
    }
    set.into_iter().collect()
}

/// Ids of the Distraction List categories in force at `now` (not opted out), sorted.
pub fn distraction_ids_in_force(state: &State, now: DateTime<Utc>) -> BTreeSet<String> {
    Catalog::builtin()
        .distraction_ids()
        .filter(|id| {
            !state
                .blocklist_opt_outs
                .iter()
                .any(|o| &o.blocklist == id && !o.is_pending(now))
        })
        .collect()
}

pub(crate) fn add_entry(
    next: &mut State,
    blocklist: &str,
    kind: EntryKind,
    entry: &str,
) -> Result<(), Reason> {
    known_blocklist(blocklist)?;
    let entry = normalize(kind, entry)?;
    next.blocklist_removals
        .retain(|r| !r.matches(blocklist, kind, &entry));
    if !listed(next, blocklist, kind).contains(&entry) {
        next.blocklist_entries.push(BlocklistEntry {
            blocklist: blocklist.to_string(),
            kind,
            entry,
        });
        next.blocklist_entries.sort();
    }
    Ok(())
}

pub(crate) fn remove_entry(
    next: &mut State,
    blocklist: &str,
    kind: EntryKind,
    entry: &str,
    now: DateTime<Utc>,
) -> Result<(), Reason> {
    known_blocklist(blocklist)?;
    let entry = normalize(kind, entry)?;
    if !effective_entries(next, blocklist, kind, now).contains(&entry) {
        return Err(Reason::UnknownEntry);
    }
    if next
        .blocklist_removals
        .iter()
        .any(|r| r.matches(blocklist, kind, &entry))
    {
        return Ok(()); // already pending; its date stays
    }
    let effective_at = next_day_boundary(next, now);
    next.blocklist_removals.push(BlocklistRemoval {
        blocklist: blocklist.to_string(),
        kind,
        entry,
        effective_at,
    });
    next.blocklist_removals.sort();
    Ok(())
}

pub(crate) fn opt_out(next: &mut State, blocklist: &str, now: DateTime<Utc>) -> Result<(), Reason> {
    known_blocklist(blocklist)?;
    if Catalog::builtin().get(blocklist).map(|l| l.kind) != Some(BlocklistKind::Distraction) {
        return Err(Reason::CannotOptOut);
    }
    if next
        .blocklist_opt_outs
        .iter()
        .any(|o| o.blocklist == blocklist)
    {
        return Ok(());
    }
    let effective_at = next_day_boundary(next, now);
    next.blocklist_opt_outs.push(BlocklistOptOut {
        blocklist: blocklist.to_string(),
        effective_at,
    });
    next.blocklist_opt_outs.sort();
    Ok(())
}

pub(crate) fn opt_in(next: &mut State, blocklist: &str) -> Result<(), Reason> {
    known_blocklist(blocklist)?;
    next.blocklist_opt_outs.retain(|o| o.blocklist != blocklist);
    Ok(())
}

/// The account's Blocklist edits as one value, as the backend, the mirror and the enforcement
/// service pass them around.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlocklistEdits {
    #[serde(default)]
    pub entries: Vec<BlocklistEntry>,
    #[serde(default)]
    pub removals: Vec<BlocklistRemoval>,
    #[serde(default)]
    pub opt_outs: Vec<BlocklistOptOut>,
}

impl State {
    /// This state's Blocklist edits.
    pub fn blocklist_edits(&self) -> BlocklistEdits {
        BlocklistEdits {
            entries: self.blocklist_entries.clone(),
            removals: self.blocklist_removals.clone(),
            opt_outs: self.blocklist_opt_outs.clone(),
        }
    }

    /// Fold in Blocklist edits from the backend at `now` so that nothing loosens sooner than
    /// tomorrow from here: every addition is kept, a removal or opt-out the backend no longer
    /// has is dropped (that tightens), a known one keeps its date, and a new one takes
    /// effect no earlier than the next day boundary after `now`.
    pub fn merge_blocklist_edits(&mut self, incoming: &BlocklistEdits, now: DateTime<Utc>) {
        let boundary = next_day_boundary(self, now);
        let mut entries: BTreeSet<BlocklistEntry> = self.blocklist_entries.drain(..).collect();
        entries.extend(incoming.entries.iter().cloned());
        self.blocklist_entries = entries.into_iter().collect();

        let known = std::mem::take(&mut self.blocklist_removals);
        let mut removals: Vec<BlocklistRemoval> = incoming
            .removals
            .iter()
            .map(|r| {
                let effective_at = known
                    .iter()
                    .find(|k| k.matches(&r.blocklist, r.kind, &r.entry))
                    .map(|k| k.effective_at)
                    .unwrap_or_else(|| r.effective_at.max(boundary));
                BlocklistRemoval {
                    effective_at,
                    ..r.clone()
                }
            })
            .collect();
        removals.sort();
        removals.dedup_by(|a, b| a.matches(&b.blocklist, b.kind, &b.entry));
        self.blocklist_removals = removals;

        let known = std::mem::take(&mut self.blocklist_opt_outs);
        let mut opt_outs: Vec<BlocklistOptOut> = incoming
            .opt_outs
            .iter()
            .filter(|o| {
                Catalog::builtin().get(&o.blocklist).map(|l| l.kind)
                    == Some(BlocklistKind::Distraction)
            })
            .map(|o| BlocklistOptOut {
                blocklist: o.blocklist.clone(),
                effective_at: known
                    .iter()
                    .find(|k| k.blocklist == o.blocklist)
                    .map(|k| k.effective_at)
                    .unwrap_or_else(|| o.effective_at.max(boundary)),
            })
            .collect();
        opt_outs.sort();
        opt_outs.dedup_by(|a, b| a.blocklist == b.blocklist);
        self.blocklist_opt_outs = opt_outs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domains_are_normalized_or_refused() {
        assert_eq!(
            normalize_domain(" Example.COM. ").as_deref(),
            Some("example.com")
        );
        assert_eq!(normalize_domain("a-b.c0.io").as_deref(), Some("a-b.c0.io"));
        for bad in [
            "",
            "localhost",
            "a..b",
            "-a.com",
            "a-.com",
            "exa mple.com",
            "https://x.com",
            "é.com",
        ] {
            assert_eq!(normalize_domain(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_skipped_midnight_maps_to_the_first_instant_after_it() {
        // Santiago springs forward at local midnight (00:00 -> 01:00) on 2026-09-06.
        let state = State {
            time_zone: "America/Santiago".into(),
            ..State::default()
        };
        let now = "2026-09-05T12:00:00Z".parse().unwrap();
        assert_eq!(
            next_day_boundary(&state, now),
            "2026-09-06T04:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
    }
}
