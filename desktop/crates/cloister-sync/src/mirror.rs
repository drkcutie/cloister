//! The local SQLite mirror of the account's Blocks: the enforcement source on a Device.

use std::path::Path;

use chrono::{DateTime, Utc};
use cloister_core::{Block, BlocklistEdits, DayPlan, PendingSettings, Schedule, Settings, State};
use rusqlite::{params, Connection, OptionalExtension};

use crate::Result;

const TIME_ZONE_KEY: &str = "time_zone";
const EDITS_KEY: &str = "blocklist_edits";
const DAY_PLANS_KEY: &str = "day_plans";
const LOOSE_ENDS_KEY: &str = "loose_ends";
const RITUALS_KEY: &str = "rituals";
const SHUTDOWN_DELAYS_KEY: &str = "shutdown_delays";
const SCHEDULES_KEY: &str = "schedules";
const SETTINGS_KEY: &str = "settings";
const PENDING_SETTINGS_KEY: &str = "pending_settings";

pub struct Mirror {
    db: Connection,
}

impl Mirror {
    /// Open (or create) the mirror at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(dir) = path.as_ref().parent() {
            std::fs::create_dir_all(dir).map_err(|e| crate::Error::Storage(e.to_string()))?;
        }
        let db = Connection::open(path)?;
        db.execute_batch(
            "pragma journal_mode = wal;
             create table if not exists blocks (
               id text primary key,
               starts_at text not null,
               ends_at text not null,
               blocklists text not null,
               deep integer not null,
               intent text,
               depth_rating integer
             );
             create table if not exists meta (key text primary key, value text not null);",
        )?;
        // Columns added after the first release of the mirror.
        let has_extras: bool = db
            .prepare("select 1 from pragma_table_info('blocks') where name = 'quit_social'")?
            .exists([])?;
        if !has_extras {
            db.execute_batch(
                "alter table blocks add column extra_domains text not null default '[]';
                 alter table blocks add column extra_apps text not null default '[]';
                 alter table blocks add column quit_social integer not null default 0;",
            )?;
        }
        Ok(Mirror { db })
    }

    /// The rules-engine state to enforce: every Block this Device knows of.
    pub fn state(&self) -> Result<State> {
        let mut stmt = self.db.prepare(
            "select id, starts_at, ends_at, blocklists, deep, intent, depth_rating,
                    extra_domains, extra_apps, quit_social
             from blocks order by starts_at, id",
        )?;
        let blocks = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, bool>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<u8>>(6)?,
                    (
                        r.get::<_, String>(7)?,
                        r.get::<_, String>(8)?,
                        r.get::<_, bool>(9)?,
                    ),
                ))
            })?
            .map(|row| {
                let (id, starts_at, ends_at, blocklists, deep, intent, depth_rating, extras) = row?;
                let (extra_domains, extra_apps, quit_social) = extras;
                let list = |s: &str| -> Result<Vec<String>> {
                    serde_json::from_str(s).map_err(|e| crate::Error::Storage(e.to_string()))
                };
                Ok(Block {
                    id,
                    starts_at: parse_time(&starts_at)?,
                    ends_at: parse_time(&ends_at)?,
                    blocklists: list(&blocklists)?,
                    deep,
                    intent,
                    depth_rating,
                    extra_domains: list(&extra_domains)?,
                    extra_apps: list(&extra_apps)?,
                    quit_social,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut state = State {
            blocks,
            day_plans: self.json(DAY_PLANS_KEY)?,
            schedules: self.json(SCHEDULES_KEY)?,
            settings: self.json(SETTINGS_KEY)?,
            pending_settings: self.json(PENDING_SETTINGS_KEY)?,
            loose_ends: self.json(LOOSE_ENDS_KEY)?,
            rituals: self.json(RITUALS_KEY)?,
            shutdown_delays: self.json(SHUTDOWN_DELAYS_KEY)?,
            ..State::default()
        };
        if let Some(tz) = self.get(TIME_ZONE_KEY)? {
            state.time_zone = tz;
        }
        if let Some(edits) = self.get(EDITS_KEY)? {
            let edits: BlocklistEdits =
                serde_json::from_str(&edits).map_err(|e| crate::Error::Storage(e.to_string()))?;
            state.blocklist_entries = edits.entries;
            state.blocklist_removals = edits.removals;
            state.blocklist_opt_outs = edits.opt_outs;
        }
        Ok(state)
    }

    /// Fold the account's time zone and Blocklist edits from the backend into the mirror at
    /// `now` (best estimate of server time). Only tightens before tomorrow: see
    /// [`State::merge_blocklist_edits`]. Returns whether anything changed.
    pub fn merge_blocklist_edits(
        &mut self,
        time_zone: &str,
        edits: &BlocklistEdits,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let before = self.state()?;
        let mut next = before.clone();
        next.time_zone = time_zone.to_string();
        next.merge_blocklist_edits(edits, now);
        if next == before {
            return Ok(false);
        }
        let tx = self.db.transaction()?;
        for (key, value) in [
            (TIME_ZONE_KEY, next.time_zone.clone()),
            (
                EDITS_KEY,
                serde_json::to_string(&next.blocklist_edits()).expect("edits serialize"),
            ),
        ] {
            tx.execute(
                "insert into meta (key, value) values (?1, ?2)
                 on conflict (key) do update set value = excluded.value",
                [key, value.as_str()],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// Fold the account's Workdays, Workday Start and Shutdown Time from the backend into the
    /// mirror at `now`: stricter settings apply at once, looser ones not before the next day
    /// boundary here (see [`State::merge_settings`]). Returns whether anything changed.
    pub fn merge_settings(
        &mut self,
        time_zone: &str,
        settings: Option<&Settings>,
        pending: Option<&PendingSettings>,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let before = self.state()?;
        let mut next = before.clone();
        next.merge_settings(time_zone, settings, pending, now);
        if next == before {
            return Ok(false);
        }
        let tx = self.db.transaction()?;
        for (key, value) in [
            (TIME_ZONE_KEY, next.time_zone.clone()),
            (
                SETTINGS_KEY,
                serde_json::to_string(&next.settings).expect("settings serialize"),
            ),
            (
                PENDING_SETTINGS_KEY,
                serde_json::to_string(&next.pending_settings).expect("settings serialize"),
            ),
        ] {
            tx.execute(
                "insert into meta (key, value) values (?1, ?2)
                 on conflict (key) do update set value = excluded.value",
                [key, value.as_str()],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// Take the backend's Day Plans and Schedules as they are: the backend already applied
    /// the ratchet, and a moved deep Slot must leave its old time. Returns whether anything
    /// changed.
    pub fn replace_day_plans(
        &mut self,
        day_plans: &[DayPlan],
        schedules: &[Schedule],
    ) -> Result<bool> {
        let plans = serde_json::to_string(day_plans).expect("plans serialize");
        let schedules = serde_json::to_string(schedules).expect("schedules serialize");
        let changed = self.get(DAY_PLANS_KEY)?.as_deref() != Some(plans.as_str())
            || self.get(SCHEDULES_KEY)?.as_deref() != Some(schedules.as_str());
        if changed {
            let tx = self.db.transaction()?;
            for (key, value) in [(DAY_PLANS_KEY, &plans), (SCHEDULES_KEY, &schedules)] {
                tx.execute(
                    "insert into meta (key, value) values (?1, ?2)
                     on conflict (key) do update set value = excluded.value",
                    [key, value.as_str()],
                )?;
            }
            tx.commit()?;
        }
        Ok(changed)
    }

    /// Take the backend's Loose Ends, completed rituals and Shutdown Delays as they are (the
    /// backend applied the rules; an enforcement service checks them again on merge). Returns
    /// whether anything changed.
    pub fn replace_ritual_records(&mut self, state: &State) -> Result<bool> {
        let values = [
            (LOOSE_ENDS_KEY, serde_json::to_string(&state.loose_ends)),
            (RITUALS_KEY, serde_json::to_string(&state.rituals)),
            (
                SHUTDOWN_DELAYS_KEY,
                serde_json::to_string(&state.shutdown_delays),
            ),
        ];
        let mut changed = false;
        let tx = self.db.transaction()?;
        for (key, value) in values {
            let value = value.expect("records serialize");
            let known: Option<String> = tx
                .query_row("select value from meta where key = ?1", [key], |r| r.get(0))
                .ok();
            if known.as_deref() != Some(value.as_str()) {
                tx.execute(
                    "insert into meta (key, value) values (?1, ?2)
                     on conflict (key) do update set value = excluded.value",
                    [key, value.as_str()],
                )?;
                changed = true;
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    fn json<T: serde::de::DeserializeOwned + Default>(&self, key: &str) -> Result<T> {
        match self.get(key)? {
            Some(text) => {
                serde_json::from_str(&text).map_err(|e| crate::Error::Storage(e.to_string()))
            }
            None => Ok(T::default()),
        }
    }

    /// Fold Blocks from the backend into the mirror. Only tightens: a new Block is added, a
    /// later end is taken, and nothing is shortened or removed. A Depth Rating, once known, stays. Returns whether anything changed.
    pub fn merge_blocks(&mut self, blocks: &[Block]) -> Result<bool> {
        let tx = self.db.transaction()?;
        let mut changed = 0;
        for b in blocks {
            let lists = serde_json::to_string(&b.blocklists).expect("strings serialize");
            let extra_domains = serde_json::to_string(&b.extra_domains).expect("strings serialize");
            let extra_apps = serde_json::to_string(&b.extra_apps).expect("strings serialize");
            changed += tx.execute(
                "insert into blocks (id, starts_at, ends_at, blocklists, deep, intent, depth_rating,
                                     extra_domains, extra_apps, quit_social)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 on conflict (id) do update set
                   ends_at = max(blocks.ends_at, excluded.ends_at),
                   intent = coalesce(blocks.intent, excluded.intent),
                   depth_rating = coalesce(blocks.depth_rating, excluded.depth_rating)
                 where excluded.ends_at > blocks.ends_at
                    or (blocks.intent is null and excluded.intent is not null)
                    or (blocks.depth_rating is null and excluded.depth_rating is not null)",
                params![
                    b.id,
                    format_time(b.starts_at),
                    format_time(b.ends_at),
                    lists,
                    b.deep,
                    b.intent,
                    b.depth_rating,
                    extra_domains,
                    extra_apps,
                    b.quit_social
                ],
            )?;
        }
        tx.commit()?;
        Ok(changed > 0)
    }

    /// This Device's id, generated the first time it's asked for.
    pub fn device_id(&self) -> Result<String> {
        if let Some(id) = self.get("device_id")? {
            return Ok(id);
        }
        let id = crate::new_uuid();
        self.set("device_id", &id)?;
        Ok(id)
    }

    /// A stored value (session, sync bookkeeping).
    pub fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .db
            .query_row("select value from meta where key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        self.db.execute(
            "insert into meta (key, value) values (?1, ?2)
             on conflict (key) do update set value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }

    pub fn remove(&self, key: &str) -> Result<()> {
        self.db.execute("delete from meta where key = ?1", [key])?;
        Ok(())
    }
}

/// Fixed-width UTC so text order is time order.
fn format_time(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

fn parse_time(s: &str) -> Result<DateTime<Utc>> {
    s.parse()
        .map_err(|e: chrono::ParseError| crate::Error::Storage(e.to_string()))
}
