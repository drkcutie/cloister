//! Sign-in from a desktop Device: email magic link and Google, both with PKCE and a loopback
//! redirect (RFC 8252).

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use cloister_sync::{LoopbackRedirect, Pkce};

#[test]
fn pkce_challenge_follows_rfc_7636() {
    // Appendix B of RFC 7636.
    let pkce = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
    assert_eq!(pkce.challenge(), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    let fresh = Pkce::new();
    assert!(fresh.verifier().len() >= 43);
    assert_ne!(fresh.verifier(), Pkce::new().verifier());
}

#[test]
fn the_loopback_redirect_hands_over_the_code() {
    let redirect = LoopbackRedirect::bind().unwrap();
    let url = url::Url::parse(&redirect.url()).unwrap();
    let addr = format!("{}:{}", url.host_str().unwrap(), url.port().unwrap());
    let path = url.path().to_string();
    let browser = std::thread::spawn(move || {
        // A stray request first, as browsers do.
        let mut s = TcpStream::connect(&addr).unwrap();
        s.write_all(b"GET /favicon.ico HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        let mut s = TcpStream::connect(&addr).unwrap();
        s.write_all(format!("GET {path}?code=abc-123&state=x HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
            .unwrap();
        let mut page = String::new();
        s.read_to_string(&mut page).unwrap();
        page
    });
    assert_eq!(redirect.wait_for_code(Duration::from_secs(5)).unwrap(), "abc-123");
    let page = browser.join().unwrap();
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
}

#[test]
fn the_google_sign_in_url_uses_pkce_and_the_loopback_redirect() {
    let backend = cloister_sync::Backend::new("https://example.supabase.co", "anon");
    let pkce = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
    let url = url::Url::parse(&backend.google_sign_in_url("http://127.0.0.1:5000/auth/callback", &pkce))
        .unwrap();
    assert_eq!(url.path(), "/auth/v1/authorize");
    let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(q["provider"], "google");
    assert_eq!(q["redirect_to"], "http://127.0.0.1:5000/auth/callback");
    assert_eq!(q["code_challenge"], "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    assert_eq!(q["code_challenge_method"], "s256");
}

#[test]
#[ignore = "needs a local Supabase; run backend/scripts/test.sh"]
fn magic_link_signs_in_and_the_session_refreshes() {
    let backend = common::backend();
    let email = common::unique_email("magic");
    let session = common::sign_in(&backend, &email);
    assert_eq!(session.email.as_deref(), Some(email.as_str()));
    assert_eq!(session.user_id.len(), 36);
    let refreshed = backend.refresh(&session).unwrap();
    assert_eq!(refreshed.user_id, session.user_id);
    assert_ne!(refreshed.refresh_token, session.refresh_token, "refresh tokens rotate");
}
