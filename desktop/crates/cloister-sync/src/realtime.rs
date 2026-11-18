//! A minimal Supabase Realtime client (Phoenix channels, protocol vsn 1.0.0) for
//! `postgres_changes` on the signed-in user's Blocks. Messages are hints: on each one the
//! Device re-reads its Blocks over REST, so a missed message costs nothing but latency.

use std::io::ErrorKind;
use std::net::TcpStream;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::{Backend, Error, Result, Session};

const HEARTBEAT_EVERY: Duration = Duration::from_secs(25);

pub(crate) struct Channel {
    ws: WebSocket<MaybeTlsStream<TcpStream>>,
    topic: String,
    next_ref: u64,
    last_heartbeat: Instant,
}

/// What arrived on the channel.
pub(crate) enum Event {
    /// A row the user can see changed.
    Changed,
    /// Nothing within the wait.
    Idle,
}

impl Channel {
    /// Connect and join; returns once Realtime streams changes for the user's Blocks.
    pub(crate) fn join(backend: &Backend, session: &Session) -> Result<Self> {
        let base = backend.url().replacen("http", "ws", 1);
        let url = format!(
            "{base}/realtime/v1/websocket?apikey={}&vsn=1.0.0",
            backend.anon_key()
        );
        let (ws, _) = tungstenite::connect(url).map_err(|e| Error::Offline(e.to_string()))?;
        let mut channel = Channel {
            ws,
            topic: format!("realtime:blocks-{}", session.user_id),
            next_ref: 1,
            last_heartbeat: Instant::now(),
        };
        channel.set_read_timeout(Duration::from_millis(500))?;
        let join_ref = channel
            .send(
                &channel.topic.clone(),
                "phx_join",
                json!({
                    "config": {
                        "broadcast": { "ack": false, "self": false },
                        "presence": { "enabled": false },
                        "postgres_changes": (["blocks", "day_plans", "schedules"].map(|table| json!({
                            "event": "*", "schema": "public", "table": table,
                            "filter": format!("user_id=eq.{}", session.user_id),
                        }))),
                        "private": false,
                    },
                    "access_token": session.access_token,
                }),
            )?
            .to_string();
        // The join reply comes first; changes are only streamed once Realtime says it has
        // subscribed to Postgres. Anything earlier must be read over REST by the caller.
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut joined = false;
        while Instant::now() < deadline {
            let Some(msg) = channel.read()? else { continue };
            let status = &msg["payload"]["status"];
            if msg["event"] == "phx_reply" && msg["ref"].as_str() == Some(join_ref.as_str()) {
                if status != "ok" {
                    return Err(Error::Protocol(format!("join refused: {}", msg["payload"])));
                }
                joined = true;
            } else if joined
                && msg["event"] == "system"
                && msg["payload"]["extension"] == "postgres_changes"
            {
                return if status == "ok" {
                    Ok(channel)
                } else {
                    Err(Error::Protocol(format!(
                        "subscription failed: {}",
                        msg["payload"]
                    )))
                };
            }
        }
        Err(Error::Offline("no subscription confirmation".into()))
    }

    /// Wait up to about half a second for the next event, keeping the connection alive.
    pub(crate) fn next(&mut self) -> Result<Event> {
        if self.last_heartbeat.elapsed() >= HEARTBEAT_EVERY {
            self.send("phoenix", "heartbeat", json!({}))?;
            self.last_heartbeat = Instant::now();
        }
        match self.read()? {
            Some(msg) if msg["event"] == "postgres_changes" => Ok(Event::Changed),
            Some(msg) if msg["event"] == "phx_error" || msg["event"] == "phx_close" => {
                Err(Error::Offline(format!("channel closed: {msg}")))
            }
            Some(msg) if msg["event"] == "system" && msg["payload"]["status"] == "error" => {
                Err(Error::Protocol(format!("realtime: {}", msg["payload"])))
            }
            _ => Ok(Event::Idle),
        }
    }

    /// Hand the channel a refreshed access token.
    pub(crate) fn update_token(&mut self, session: &Session) -> Result<()> {
        let topic = self.topic.clone();
        self.send(
            &topic,
            "access_token",
            json!({ "access_token": session.access_token }),
        )
        .map(|_| ())
    }

    fn send(&mut self, topic: &str, event: &str, payload: Value) -> Result<u64> {
        let r = self.next_ref;
        self.next_ref += 1;
        let msg = json!({ "topic": topic, "event": event, "payload": payload,
                          "ref": r.to_string(), "join_ref": "1" });
        self.ws
            .send(Message::Text(msg.to_string()))
            .map_err(|e| Error::Offline(e.to_string()))?;
        Ok(r)
    }

    fn read(&mut self) -> Result<Option<Value>> {
        match self.ws.read() {
            Ok(Message::Text(text)) => {
                if std::env::var_os("CLOISTER_REALTIME_DEBUG").is_some() {
                    eprintln!("realtime <- {text}");
                }
                Ok(serde_json::from_str(&text).ok())
            }
            Ok(Message::Close(_)) => Err(Error::Offline("connection closed".into())),
            Ok(_) => Ok(None),
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
            {
                Ok(None)
            }
            Err(e) => Err(Error::Offline(e.to_string())),
        }
    }

    fn set_read_timeout(&mut self, timeout: Duration) -> Result<()> {
        let tcp = match self.ws.get_mut() {
            MaybeTlsStream::Plain(s) => s,
            MaybeTlsStream::Rustls(s) => s.get_mut(),
            _ => return Ok(()),
        };
        tcp.set_read_timeout(Some(timeout))
            .map_err(|e| Error::Offline(e.to_string()))
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        let _ = self.ws.close(None);
    }
}
