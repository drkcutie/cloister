//! Persisted rules-engine state, so Blocks survive restarts and reboots.

use std::io;
use std::path::{Path, PathBuf};

use cloister_core::State;

pub struct StateStore {
    path: PathBuf,
}

impl StateStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        StateStore { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The saved state; empty if nothing was saved yet. A corrupt file is an error: the
    /// service then refuses to start, and with the system resolver pointing at the stopped
    /// filter nothing resolves, so it fails closed.
    pub fn load(&self) -> io::Result<State> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(State::default()),
            Err(e) => Err(e),
        }
    }

    /// Write atomically (temp file + rename).
    pub fn save(&self, state: &State) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(state)?)?;
        std::fs::rename(&tmp, &self.path)
    }
}
