//! The DNS filter, driven over real UDP on high ports (no root needed).

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use cloister_enforcer::dns::{self, DnsFilter, QTYPE_A, RCODE_NXDOMAIN};
use cloister_enforcer::domains::DomainSet;

const UPSTREAM_IP: Ipv4Addr = Ipv4Addr::new(93, 184, 216, 34);

/// A stand-in upstream resolver that answers every A query with UPSTREAM_IP.
fn fake_upstream() -> SocketAddr {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = socket.local_addr().unwrap();
    thread::spawn(move || {
        let mut buf = [0u8; 512];
        while let Ok((len, from)) = socket.recv_from(&mut buf) {
            let mut reply = buf[..len].to_vec();
            reply[2] |= 0x80;
            reply[7] = 1; // ANCOUNT = 1
            reply.extend_from_slice(&[0xc0, 12, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4]);
            reply.extend_from_slice(&UPSTREAM_IP.octets());
            socket.send_to(&reply, from).unwrap();
        }
    });
    addr
}

fn filter(blocked: &[&str]) -> SocketAddr {
    let domains = Arc::new(RwLock::new(DomainSet::new(blocked)));
    DnsFilter::spawn(
        "127.0.0.1:0".parse().unwrap(),
        vec![fake_upstream()],
        domains,
    )
    .unwrap()
    .local_addr()
}

fn ask(server: SocketAddr, name: &str) -> Vec<u8> {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    socket
        .send_to(&dns::query_packet(0x1234, name, QTYPE_A), server)
        .unwrap();
    let mut buf = [0u8; 512];
    let (len, _) = socket.recv_from(&mut buf).unwrap();
    buf[..len].to_vec()
}

#[test]
fn blocked_domains_their_subdomains_and_related_domains_do_not_resolve() {
    let server = filter(&["youtube.com", "youtu.be", "googlevideo.com"]);
    for name in [
        "youtube.com",
        "www.youtube.com",
        "m.YouTube.com",
        "youtu.be",
        "rr3---sn-abc.googlevideo.com",
    ] {
        let reply = ask(server, name);
        assert_eq!(dns::response_code(&reply), Some(RCODE_NXDOMAIN), "{name}");
        assert!(dns::a_records(&reply).is_empty(), "{name}");
        assert_eq!(dns::question(&reply).unwrap().0, name.to_ascii_lowercase());
        assert_eq!(reply[..2], [0x12, 0x34], "reply keeps the query id");
    }
}

#[test]
fn other_domains_are_forwarded_upstream() {
    let server = filter(&["youtube.com"]);
    for name in ["example.com", "notyoutube.com", "youtube.com.example.org"] {
        let reply = ask(server, name);
        assert_eq!(dns::response_code(&reply), Some(0), "{name}");
        assert_eq!(dns::a_records(&reply), [UPSTREAM_IP], "{name}");
    }
}

#[test]
fn the_blocked_set_can_change_while_running() {
    let domains = Arc::new(RwLock::new(DomainSet::default()));
    let server = DnsFilter::spawn(
        "127.0.0.1:0".parse().unwrap(),
        vec![fake_upstream()],
        domains.clone(),
    )
    .unwrap()
    .local_addr();
    assert_eq!(dns::response_code(&ask(server, "reddit.com")), Some(0));
    *domains.write().unwrap() = DomainSet::new(["reddit.com"]);
    assert_eq!(
        dns::response_code(&ask(server, "old.reddit.com")),
        Some(RCODE_NXDOMAIN)
    );
}

#[test]
fn resolves_a_records_for_the_firewall_table() {
    assert_eq!(
        dns::resolve_a(fake_upstream(), "youtube.com").unwrap(),
        [UPSTREAM_IP]
    );
}
