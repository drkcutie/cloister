//! The local SQLite mirror of the account's Blocks: the enforcement source on a Device.

use std::path::Path;

use chrono::{DateTime, Utc};
use cloister_core::{Block, DayPlan, Schedule, State};
use rusqlite::{params, Connection, OptionalExtension};

use crate::Result;

const DAY_PLANS_KEY: &str = "day_plans";
const SCHEDULES_KEY: &str = "schedules";

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
        Ok(Mirror { db })
    }

    /// The rules-engine state to enforce: every Block this Device knows of.
    pub fn state(&self) -> Result<State> {
        let mut stmt = self.db.prepare(
            "select id, starts_at, ends_at, blocklists, deep, intent, depth_rating
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
                ))
            })?
            .map(|row| {
                let (id, starts_at, ends_at, blocklists, deep, intent, depth_rating) = row?;
                Ok(Block {
                    id,
                    starts_at: parse_time(&starts_at)?,
                    ends_at: parse_time(&ends_at)?,
                    blocklists: serde_json::from_str(&blocklists)
                        .map_err(|e| crate::Error::Storage(e.to_string()))?,
                    deep,
                    intent,
                    depth_rating,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(State {
            blocks,
            day_plans: self.json(DAY_PLANS_KEY)?,
            schedules: self.json(SCHEDULES_KEY)?,
        })
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
            changed += tx.execute(
                "insert into blocks (id, starts_at, ends_at, blocklists, deep, intent, depth_rating)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7)
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
                    b.depth_rating
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
