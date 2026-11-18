//! Helpers for tests against a local Supabase (`backend/scripts/test.sh` sets the env).
#![allow(dead_code)]

use std::time::Duration;

use cloister_sync::{Backend, LoopbackRedirect, Pkce, Session};
use serde_json::Value;

pub fn backend() -> Backend {
    Backend::from_env().expect("CLOISTER_API_URL and CLOISTER_ANON_KEY (run backend/scripts/test.sh)")
}

pub fn mailpit() -> String {
    std::env::var("CLOISTER_MAILPIT_URL").unwrap_or_else(|_| "http://127.0.0.1:56424".into())
}

pub fn unique_email(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{prefix}-{nanos}@devices.test")
}

/// The text of the newest email to `email`.
pub fn email_text_for(email: &str) -> String {
    for _ in 0..50 {
        let search: Value = ureq::get(&format!("{}/api/v1/search", mailpit()))
            .query("query", &format!("to:\"{email}\""))
            .call()
            .unwrap()
            .into_json()
            .unwrap();
        if let Some(id) = search["messages"][0]["ID"].as_str() {
            let message: Value = ureq::get(&format!("{}/api/v1/message/{id}", mailpit()))
                .call()
                .unwrap()
                .into_json()
                .unwrap();
            return message["Text"].as_str().unwrap_or_default().to_string();
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("no email arrived for {email}");
}

/// The magic link in the newest email to `email`, as the user would click it.
pub fn magic_link_for(email: &str) -> String {
    let text = email_text_for(email);
    let start = text.find("http").expect("a link in the email");
    text[start..]
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != ')' && *c != ']')
        .collect()
}

/// Sign in with the email magic link, clicking it in place of the user.
pub fn sign_in(backend: &Backend, email: &str) -> Session {
    let pkce = Pkce::new();
    let redirect = LoopbackRedirect::bind().unwrap();
    backend.send_magic_link(email, &redirect.url(), &pkce).unwrap();
    let link = magic_link_for(email);
    let browser = std::thread::spawn(move || {
        // Follows the redirect to the loopback listener, like a browser.
        let _ = ureq::get(&link).call();
    });
    let code = redirect.wait_for_code(Duration::from_secs(10)).unwrap();
    browser.join().unwrap();
    backend.exchange_code(&code, &pkce).unwrap()
}
