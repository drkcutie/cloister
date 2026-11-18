//! The Supabase backend: sign-in (GoTrue) and the REST API (PostgREST), over blocking HTTP.

use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{Error, Result};

/// Where the backend is: its API URL and public (anon) key.
#[derive(Clone)]
pub struct Backend {
    url: String,
    anon_key: String,
    agent: ureq::Agent,
}

/// A signed-in account on this Device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: DateTime<Utc>,
    pub user_id: String,
    pub email: Option<String>,
}

impl Session {
    /// Whether the access token should be refreshed before use.
    pub fn expires_soon(&self, now: DateTime<Utc>) -> bool {
        self.expires_at - chrono::Duration::seconds(60) <= now
    }
}

/// A PKCE verifier and its S256 challenge (RFC 7636).
pub struct Pkce {
    verifier: String,
}

impl Pkce {
    pub fn new() -> Self {
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes).expect("system randomness");
        Pkce::from_verifier(&URL_SAFE_NO_PAD.encode(bytes))
    }

    pub fn from_verifier(verifier: &str) -> Self {
        Pkce {
            verifier: verifier.to_string(),
        }
    }

    pub fn verifier(&self) -> &str {
        &self.verifier
    }

    pub fn challenge(&self) -> String {
        URL_SAFE_NO_PAD.encode(Sha256::digest(self.verifier.as_bytes()))
    }
}

impl Default for Pkce {
    fn default() -> Self {
        Pkce::new()
    }
}

impl Backend {
    pub fn new(url: &str, anon_key: &str) -> Self {
        Backend {
            url: url.trim_end_matches('/').to_string(),
            anon_key: anon_key.to_string(),
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(5))
                .timeout(Duration::from_secs(15))
                .build(),
        }
    }

    /// From `CLOISTER_API_URL` and `CLOISTER_ANON_KEY`.
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("CLOISTER_API_URL").ok()?;
        let key = std::env::var("CLOISTER_ANON_KEY").ok()?;
        Some(Backend::new(&url, &key))
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn anon_key(&self) -> &str {
        &self.anon_key
    }

    // Sign-in --------------------------------------------------------------------------------

    /// Email a magic link. Clicking it redirects to `redirect_to` with a `code` to pass to
    /// [`Backend::exchange_code`] with the same `pkce`.
    pub fn send_magic_link(&self, email: &str, redirect_to: &str, pkce: &Pkce) -> Result<()> {
        let req = self
            .agent
            .post(&format!("{}/auth/v1/otp", self.url))
            .query("redirect_to", redirect_to);
        self.call(
            req,
            Some(json!({
                "email": email,
                "create_user": true,
                "code_challenge": pkce.challenge(),
                "code_challenge_method": "s256",
            })),
            None,
        )
        .map(|_| ())
    }

    /// The page to open in the browser for Google sign-in. It redirects to `redirect_to` with
    /// a `code` for [`Backend::exchange_code`].
    pub fn google_sign_in_url(&self, redirect_to: &str, pkce: &Pkce) -> String {
        let mut url = url::Url::parse(&format!("{}/auth/v1/authorize", self.url)).expect("valid URL");
        url.query_pairs_mut()
            .append_pair("provider", "google")
            .append_pair("redirect_to", redirect_to)
            .append_pair("code_challenge", &pkce.challenge())
            .append_pair("code_challenge_method", "s256");
        url.into()
    }

    pub fn exchange_code(&self, code: &str, pkce: &Pkce) -> Result<Session> {
        let req = self
            .agent
            .post(&format!("{}/auth/v1/token", self.url))
            .query("grant_type", "pkce");
        let body = json!({ "auth_code": code, "code_verifier": pkce.verifier() });
        self.session_from(self.call(req, Some(body), None))
    }

    /// Sign in with the 6-digit code from the sign-in email instead of the link.
    pub fn verify_email_code(&self, email: &str, code: &str) -> Result<Session> {
        let req = self.agent.post(&format!("{}/auth/v1/verify", self.url));
        let body = json!({ "type": "email", "email": email, "token": code });
        self.session_from(self.call(req, Some(body), None))
    }

    /// A fresh session. Refresh tokens rotate: keep the returned one.
    pub fn refresh(&self, session: &Session) -> Result<Session> {
        let req = self
            .agent
            .post(&format!("{}/auth/v1/token", self.url))
            .query("grant_type", "refresh_token");
        let body = json!({ "refresh_token": session.refresh_token });
        self.session_from(self.call(req, Some(body), None))
    }

    fn session_from(&self, answer: Result<Value>) -> Result<Session> {
        let v = answer.map_err(|e| match e {
            Error::Http { status, body } if (400..500).contains(&status) => Error::Auth(body),
            other => other,
        })?;
        let text = |key: &str| {
            v[key]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| Error::Protocol(format!("no {key} in session")))
        };
        let expires_at = v["expires_at"]
            .as_i64()
            .and_then(|s| Utc.timestamp_opt(s, 0).single())
            .or_else(|| v["expires_in"].as_i64().map(|s| Utc::now() + chrono::Duration::seconds(s)))
            .ok_or_else(|| Error::Protocol("no expiry in session".into()))?;
        Ok(Session {
            access_token: text("access_token")?,
            refresh_token: text("refresh_token")?,
            expires_at,
            user_id: v["user"]["id"]
                .as_str()
                .ok_or_else(|| Error::Protocol("no user id in session".into()))?
                .to_string(),
            email: v["user"]["email"].as_str().map(str::to_string),
        })
    }

    // REST -----------------------------------------------------------------------------------

    pub(crate) fn rest_get(&self, session: &Session, path_and_query: &str) -> Result<Value> {
        let req = self.agent.get(&format!("{}/rest/v1/{path_and_query}", self.url));
        self.call(req, None, Some(session))
    }

    pub(crate) fn rest_post(
        &self,
        session: &Session,
        path: &str,
        body: Value,
        prefer: Option<&str>,
    ) -> Result<Value> {
        let mut req = self.agent.post(&format!("{}/rest/v1/{path}", self.url));
        if let Some(p) = prefer {
            req = req.set("Prefer", p);
        }
        self.call(req, Some(body), Some(session))
    }

    fn call(&self, req: ureq::Request, body: Option<Value>, session: Option<&Session>) -> Result<Value> {
        let mut req = req.set("apikey", &self.anon_key);
        if let Some(s) = session {
            req = req.set("Authorization", &format!("Bearer {}", s.access_token));
        }
        let answer = match body {
            Some(b) => req.send_json(b),
            None => req.call(),
        };
        match answer {
            Ok(resp) => {
                let text = resp.into_string().map_err(|e| Error::Offline(e.to_string()))?;
                if text.trim().is_empty() {
                    Ok(Value::Null)
                } else {
                    serde_json::from_str(&text).map_err(|e| Error::Protocol(format!("{e}: {text}")))
                }
            }
            Err(ureq::Error::Status(status, resp)) => Err(Error::Http {
                status,
                body: resp.into_string().unwrap_or_default(),
            }),
            Err(ureq::Error::Transport(t)) => Err(Error::Offline(t.to_string())),
        }
    }
}
