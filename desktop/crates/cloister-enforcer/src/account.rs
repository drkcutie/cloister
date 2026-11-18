//! The service's own, authenticated view of the account.
//!
//! Anything that lifts enforcement (a completed Shutdown Ritual, a Shutdown Delay, settings or
//! Blocklist changes that loosen tomorrow, a changed Schedule) and the server time itself are
//! only taken from the backend, fetched by the service over TLS with the signed-in Device's
//! access token. The local socket is open to every local user, so what arrives over it can
//! only tighten. The backend checks the token; the service pins the first account it sees and
//! ignores any other.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use cloister_core::{Block, State};
use serde::{Deserialize, Serialize};

/// The account's data as the backend has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// The account the access token belongs to, as the backend says.
    pub user_id: String,
    pub blocks: Vec<Block>,
    /// The account's state in the engine's form (Blocks aside).
    pub state: State,
    pub server_time: DateTime<Utc>,
}

/// Fetches the account with an access token.
pub trait Account: Send {
    fn fetch(&self, access_token: &str) -> Result<Fetched, String>;
}

/// The backend over HTTPS (Supabase REST).
pub struct Backend {
    url: String,
    anon_key: String,
    agent: ureq::Agent,
}

const BLOCKS: &str = "blocks?select=id,starts_at,ends_at,blocklists,deep,intent,depth_rating,extra_domains,extra_apps,quit_social&order=starts_at,id";

impl Backend {
    pub fn new(url: &str, anon_key: &str) -> Result<Self, String> {
        let builder = ureq::AgentBuilder::new().timeout(Duration::from_secs(5));
        #[cfg(windows)]
        let builder = builder.tls_connector(std::sync::Arc::new(
            native_tls::TlsConnector::new().map_err(|e| e.to_string())?,
        ));
        Ok(Backend {
            url: url.trim_end_matches('/').to_string(),
            anon_key: anon_key.to_string(),
            agent: builder.build(),
        })
    }

    fn get(&self, path: &str, token: &str) -> Result<serde_json::Value, String> {
        self.read(
            self.agent
                .get(&format!("{}/{path}", self.url))
                .set("apikey", &self.anon_key)
                .set("Authorization", &format!("Bearer {token}"))
                .call(),
        )
    }

    fn rpc(&self, name: &str, token: &str) -> Result<serde_json::Value, String> {
        self.read(
            self.agent
                .post(&format!("{}/rest/v1/rpc/{name}", self.url))
                .set("apikey", &self.anon_key)
                .set("Authorization", &format!("Bearer {token}"))
                .send_json(serde_json::json!({})),
        )
    }

    fn read(
        &self,
        answer: Result<ureq::Response, ureq::Error>,
    ) -> Result<serde_json::Value, String> {
        match answer {
            Ok(resp) => resp.into_json().map_err(|e| e.to_string()),
            Err(ureq::Error::Status(status, _)) => Err(format!("backend answered {status}")),
            Err(e) => Err(e.to_string()),
        }
    }
}

impl Account for Backend {
    fn fetch(&self, token: &str) -> Result<Fetched, String> {
        let user = self.get("auth/v1/user", token)?;
        let user_id = user["id"]
            .as_str()
            .ok_or("the backend sent no user id")?
            .to_string();
        let blocks = serde_json::from_value(self.get(&format!("rest/v1/{BLOCKS}"), token)?)
            .map_err(|e| format!("blocks: {e}"))?;
        let state =
            serde_json::from_value(self.rpc("state", token)?).map_err(|e| format!("state: {e}"))?;
        let server_time = serde_json::from_value(self.rpc("server_now", token)?)
            .map_err(|e| format!("server time: {e}"))?;
        Ok(Fetched {
            user_id,
            blocks,
            state,
            server_time,
        })
    }
}

#[derive(Serialize, Deserialize)]
struct Pin {
    user_id: String,
}

/// The service's link to the account: the fetcher, the pinned account and how often to fetch.
pub struct AccountLink {
    account: Box<dyn Account>,
    pin_path: Option<PathBuf>,
    pinned: Option<String>,
    last_fetch: Option<Instant>,
    /// Never fetch more often than this.
    pub min_interval: Duration,
    /// Fetch at least this often while a Device hands over a token, even with nothing new.
    pub max_interval: Duration,
}

impl AccountLink {
    /// `pin_path` keeps the pinned account across restarts (root-owned, next to the state).
    pub fn new(account: Box<dyn Account>, pin_path: Option<PathBuf>) -> Self {
        let pinned = pin_path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Pin>(&b).ok())
            .map(|p| p.user_id);
        AccountLink {
            account,
            pin_path,
            pinned,
            last_fetch: None,
            min_interval: Duration::from_secs(2),
            max_interval: Duration::from_secs(60),
        }
    }

    pub fn pinned(&self) -> Option<&str> {
        self.pinned.as_deref()
    }

    /// The account's data, if a fetch is due (`news`: the Device's view differs from the
    /// service's) and it is the pinned account (the first one seen is pinned).
    pub fn fetch(&mut self, token: &str, news: bool) -> Option<Fetched> {
        let since = self.last_fetch.map(|t| t.elapsed());
        let due = match since {
            None => true,
            Some(s) if s < self.min_interval => false,
            Some(s) => news || s >= self.max_interval,
        };
        if !due {
            return None;
        }
        self.last_fetch = Some(Instant::now());
        let fetched = match self.account.fetch(token) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("cloisterd: fetching the account: {e}");
                return None;
            }
        };
        match &self.pinned {
            Some(pinned) if *pinned != fetched.user_id => {
                eprintln!("cloisterd: ignoring a token for another account");
                return None;
            }
            Some(_) => {}
            None => {
                if let Some(path) = &self.pin_path {
                    let pin = Pin {
                        user_id: fetched.user_id.clone(),
                    };
                    let saved = serde_json::to_vec(&pin)
                        .map_err(std::io::Error::from)
                        .and_then(|bytes| std::fs::write(path, bytes));
                    if let Err(e) = saved {
                        eprintln!("cloisterd: saving the account {}: {e}", path.display());
                        return None;
                    }
                }
                self.pinned = Some(fetched.user_id.clone());
            }
        }
        Some(fetched)
    }
}
