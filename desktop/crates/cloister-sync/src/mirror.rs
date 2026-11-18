//! The local SQLite mirror of the account's Blocks: the enforcement source on a Device.

use std::path::Path;

use chrono::{DateTime, Utc};
use cloister_core::{Block, State};
use rusqlite::{params, Connection, OptionalExtension};

use crate::Result;

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
               deep integer not null
             );
             create table if not exists meta (key text primary key, value text not null);",
        )?;
        Ok(Mirror { db })
    }

    /// The rules-engine state to enforce: every Block this Device knows of.
    pub fn state(&self) -> Result<State> {
        let mut stmt = self
            .db
            .prepare("select id, starts_at, ends_at, blocklists, deep from blocks order by starts_at, id")?;
        let blocks = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, bool>(4)?,
                ))
            })?
            .map(|row| {
                let (id, starts_at, ends_at, blocklists, deep) = row?;
                Ok(Block {
                    id,
                    starts_at: parse_time(&starts_at)?,
                    ends_at: parse_time(&ends_at)?,
                    blocklists: serde_json::from_str(&blocklists)
                        .map_err(|e| crate::Error::Storage(e.to_string()))?,
                    deep,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(State { blocks })
    }

    /// Fold Blocks from the backend into the mirror. Only tightens: a new Block is added, a
    /// later end is taken, and nothing is shortened or removed. Returns whether anything changed.
    pub fn merge_blocks(&mut self, blocks: &[Block]) -> Result<bool> {
        let tx = self.db.transaction()?;
        let mut changed = 0;
        for b in blocks {
            let lists = serde_json::to_string(&b.blocklists).expect("strings serialize");
            changed += tx.execute(
                "insert into blocks (id, starts_at, ends_at, blocklists, deep)
                 values (?1, ?2, ?3, ?4, ?5)
                 on conflict (id) do update set ends_at = excluded.ends_at
                 where excluded.ends_at > blocks.ends_at",
                params![b.id, format_time(b.starts_at), format_time(b.ends_at), lists, b.deep],
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
    s.parse().map_err(|e: chrono::ParseError| crate::Error::Storage(e.to_string()))
}
