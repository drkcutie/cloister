//! A one-shot loopback HTTP listener that receives the sign-in redirect (RFC 8252 §7.3).

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use crate::{Error, Result};

const PATH: &str = "/auth/callback";
const DONE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Cloister</title>\
<body style=\"font-family:sans-serif;background:#EEF0EE;color:#17211F;padding:48px\">\
<p>Signed in to Cloister. You can close this tab.</p>";

pub struct LoopbackRedirect {
    listener: TcpListener,
}

impl LoopbackRedirect {
    /// Listen on a free port on 127.0.0.1.
    pub fn bind() -> Result<Self> {
        let listener =
            TcpListener::bind("127.0.0.1:0").map_err(|e| Error::Auth(format!("loopback: {e}")))?;
        Ok(LoopbackRedirect { listener })
    }

    /// The `redirect_to` URL to give the backend.
    pub fn url(&self) -> String {
        let port = self.listener.local_addr().map(|a| a.port()).unwrap_or(0);
        format!("http://127.0.0.1:{port}{PATH}")
    }

    /// Wait for the browser to arrive with `?code=`. Other requests are answered and ignored.
    pub fn wait_for_code(&self, timeout: Duration) -> Result<String> {
        let deadline = Instant::now() + timeout;
        self.listener
            .set_nonblocking(true)
            .map_err(|e| Error::Auth(e.to_string()))?;
        while Instant::now() < deadline {
            let (stream, _) = match self.listener.accept() {
                Ok(conn) => conn,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                }
                Err(e) => return Err(Error::Auth(e.to_string())),
            };
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut line = String::new();
            let mut reader = BufReader::new(&stream);
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            // Drain the headers so the browser sees a clean response.
            let mut header = String::new();
            while reader
                .read_line(&mut header)
                .map(|n| n > 2)
                .unwrap_or(false)
            {
                header.clear();
            }
            let target = line.split_whitespace().nth(1).unwrap_or("/");
            let url = url::Url::parse(&format!("http://127.0.0.1{target}"))
                .map_err(|e| Error::Auth(e.to_string()))?;
            let param = |k: &str| {
                url.query_pairs()
                    .find(|(key, _)| key == k)
                    .map(|(_, v)| v.into_owned())
            };
            let mut stream = &stream;
            if url.path() != PATH {
                let _ = stream.write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
                continue;
            }
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{DONE_PAGE}",
                DONE_PAGE.len()
            );
            if let Some(code) = param("code") {
                return Ok(code);
            }
            if let Some(error) = param("error_description").or_else(|| param("error")) {
                return Err(Error::Auth(error));
            }
        }
        Err(Error::Auth("sign-in timed out".into()))
    }
}
