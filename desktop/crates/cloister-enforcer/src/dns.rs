//! The local DNS filter (ADR 0002): the system's only resolver while Cloister is installed.
//!
//! Queries for a blocked domain or any of its subdomains get NXDOMAIN. Everything else is
//! forwarded unchanged to an upstream resolver and the answer relayed back. UDP only;
//! `pf` stops DNS (including TCP and DNS-over-TLS) from going anywhere else.

use std::io;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use crate::domains::DomainSet;

pub const QTYPE_A: u16 = 1;
pub const RCODE_NXDOMAIN: u8 = 3;
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(3);

/// Shared, live-updated set of blocked domains. The service swaps it on every tick.
pub type SharedDomains = Arc<RwLock<DomainSet>>;

/// A running DNS filter.
pub struct DnsFilter {
    local_addr: SocketAddr,
}

impl DnsFilter {
    /// Listen on `listen` (e.g. `127.0.0.1:53`, or port 0 in tests) and serve forever on a
    /// background thread.
    pub fn spawn(
        listen: SocketAddr,
        upstreams: Vec<SocketAddr>,
        blocked: SharedDomains,
    ) -> io::Result<DnsFilter> {
        let socket = UdpSocket::bind(listen)?;
        let local_addr = socket.local_addr()?;
        let upstreams = Arc::new(upstreams);
        thread::Builder::new()
            .name("dns-filter".into())
            .spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    let Ok((len, from)) = socket.recv_from(&mut buf) else {
                        continue;
                    };
                    let query = buf[..len].to_vec();
                    let Ok(reply_socket) = socket.try_clone() else {
                        continue;
                    };
                    let upstreams = Arc::clone(&upstreams);
                    let blocked = Arc::clone(&blocked);
                    thread::spawn(move || {
                        if let Some(reply) = answer(&query, &upstreams, &blocked) {
                            let _ = reply_socket.send_to(&reply, from);
                        }
                    });
                }
            })?;
        Ok(DnsFilter { local_addr })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
}

fn answer(query: &[u8], upstreams: &[SocketAddr], blocked: &SharedDomains) -> Option<Vec<u8>> {
    let (name, _) = question(query)?;
    let is_blocked = blocked
        .read()
        .map(|set| set.is_blocked(&name))
        .unwrap_or(true); // a poisoned lock fails closed
    if is_blocked {
        return Some(nxdomain(query));
    }
    upstreams.iter().find_map(|u| exchange(*u, query).ok())
}

/// Send `query` to `server` and wait for the matching reply.
pub fn exchange(server: SocketAddr, query: &[u8]) -> io::Result<Vec<u8>> {
    let bind: SocketAddr = if server.is_ipv4() {
        "0.0.0.0:0".parse().unwrap()
    } else {
        "[::]:0".parse().unwrap()
    };
    let socket = UdpSocket::bind(bind)?;
    socket.set_read_timeout(Some(UPSTREAM_TIMEOUT))?;
    socket.send_to(query, server)?;
    let mut buf = [0u8; 4096];
    loop {
        let (len, from) = socket.recv_from(&mut buf)?;
        if from == server && len >= 2 && buf[..2] == query[..2] {
            return Ok(buf[..len].to_vec());
        }
    }
}

/// Look up the IPv4 addresses of `name` at `server`. Used to fill the firewall's IP table.
pub fn resolve_a(server: SocketAddr, name: &str) -> io::Result<Vec<Ipv4Addr>> {
    let id = (std::process::id() as u16) ^ (name.len() as u16).rotate_left(7);
    let reply = exchange(server, &query_packet(id, name, QTYPE_A))?;
    Ok(a_records(&reply))
}

/// Build a standard recursive query for `name`.
pub fn query_packet(id: u16, name: &str, qtype: u16) -> Vec<u8> {
    let mut p = Vec::with_capacity(32 + name.len());
    p.extend_from_slice(&id.to_be_bytes());
    p.extend_from_slice(&[0x01, 0x00]); // RD
    p.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]); // QDCOUNT=1
    for label in name.trim_end_matches('.').split('.') {
        p.push(label.len() as u8);
        p.extend_from_slice(label.as_bytes());
    }
    p.push(0);
    p.extend_from_slice(&qtype.to_be_bytes());
    p.extend_from_slice(&[0, 1]); // IN
    p
}

/// The response code (RCODE) of a DNS message.
pub fn response_code(packet: &[u8]) -> Option<u8> {
    packet.get(3).map(|b| b & 0x0f)
}

/// The first question's name (lowercase, no trailing dot) and the offset just past it
/// (after QTYPE and QCLASS).
pub fn question(packet: &[u8]) -> Option<(String, usize)> {
    if packet.len() < 12 || u16::from_be_bytes([packet[4], packet[5]]) == 0 {
        return None;
    }
    let mut labels = Vec::new();
    let mut i = 12;
    loop {
        let len = *packet.get(i)? as usize;
        i += 1;
        if len == 0 {
            break;
        }
        if len > 63 {
            return None; // compression is not valid in a question we parse
        }
        labels.push(std::str::from_utf8(packet.get(i..i + len)?).ok()?);
        i += len;
    }
    let end = i + 4;
    if packet.len() < end {
        return None;
    }
    Some((labels.join(".").to_ascii_lowercase(), end))
}

/// An NXDOMAIN reply to `query`, echoing its question.
pub fn nxdomain(query: &[u8]) -> Vec<u8> {
    let end = question(query)
        .map(|(_, end)| end)
        .unwrap_or(12.min(query.len()));
    let mut reply = query[..end].to_vec();
    reply.resize(12.max(end), 0);
    reply[2] = 0x80 | (query.get(2).copied().unwrap_or(0) & 0x79); // QR, opcode, RD
    reply[3] = 0x80 | RCODE_NXDOMAIN; // RA
    reply[4..6].copy_from_slice(&[0, (end > 12) as u8]);
    reply[6..12].fill(0);
    reply
}

/// All A records in the answer section of `packet`.
pub fn a_records(packet: &[u8]) -> Vec<Ipv4Addr> {
    let mut out = Vec::new();
    let Some((_, mut i)) = question(packet) else {
        return out;
    };
    let answers = u16::from_be_bytes([packet[6], packet[7]]);
    for _ in 0..answers {
        let Some(after_name) = skip_name(packet, i) else {
            break;
        };
        let Some(header) = packet.get(after_name..after_name + 10) else {
            break;
        };
        let rtype = u16::from_be_bytes([header[0], header[1]]);
        let rdlen = u16::from_be_bytes([header[8], header[9]]) as usize;
        let data_start = after_name + 10;
        let Some(data) = packet.get(data_start..data_start + rdlen) else {
            break;
        };
        if rtype == QTYPE_A && rdlen == 4 {
            out.push(Ipv4Addr::new(data[0], data[1], data[2], data[3]));
        }
        i = data_start + rdlen;
    }
    out
}

fn skip_name(packet: &[u8], mut i: usize) -> Option<usize> {
    loop {
        let len = *packet.get(i)?;
        if len & 0xc0 == 0xc0 {
            return Some(i + 2);
        }
        i += 1 + len as usize;
        if len == 0 {
            return Some(i);
        }
    }
}
