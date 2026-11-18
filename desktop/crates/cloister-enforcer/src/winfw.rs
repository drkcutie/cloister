//! Windows Firewall rules (ADR 0002), the counterpart of [`crate::pf`] and [`crate::nft`]:
//! traffic to the resolved IPs of blocked domains is dropped, and DNS (53) and DNS-over-TLS
//! (853) may only go to the local filter or to the service's upstream resolvers.
//!
//! Windows Firewall block rules always win over allow rules and can't exclude a program, so
//! "everything except the service may not send DNS" is written as "nobody may send DNS to any
//! address except loopback and the upstreams": the rule's remote addresses are every range
//! *around* those addresses. Rendered here as `netsh advfirewall firewall` argument lists; the
//! Windows layer only runs them.

use std::collections::BTreeSet;
use std::net::Ipv4Addr;

/// Rule names have no spaces, so no argument needs quoting when passed to `netsh`.
pub const RULE_BLOCKED: &str = "Cloister-blocked-sites";
pub const RULE_DNS: &str = "Cloister-DNS";

/// Every IPv4 range outside `addresses`, as `a-b` strings in address order.
pub fn ranges_excluding(addresses: &BTreeSet<Ipv4Addr>) -> Vec<String> {
    let mut ranges = Vec::new();
    let mut start: u64 = 0;
    for &address in addresses {
        let n = u64::from(u32::from(address));
        if n > start {
            ranges.push(range(start, n - 1));
        }
        start = n + 1;
    }
    if start <= u64::from(u32::MAX) {
        ranges.push(range(start, u64::from(u32::MAX)));
    }
    ranges
}

fn range(from: u64, to: u64) -> String {
    let a = Ipv4Addr::from(from as u32);
    let b = Ipv4Addr::from(to as u32);
    format!("{a}-{b}")
}

/// Every IPv6 address except `::1` (the filter only listens on IPv4 loopback).
const IPV6_EXCEPT_LOOPBACK: &str = "::-::0,::2-ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff";

/// The `netsh` argument lists that replace Cloister's rules: delete the old ones, then add a
/// rule dropping traffic to `blocked_ips` (only when there are any) and the DNS rules.
pub fn rules(blocked_ips: &BTreeSet<Ipv4Addr>, upstreams: &[Ipv4Addr]) -> Vec<Vec<String>> {
    let netsh = |s: String| -> Vec<String> { s.split(' ').map(str::to_string).collect() };
    let mut out = vec![
        netsh(format!(
            "advfirewall firewall delete rule name={RULE_BLOCKED}"
        )),
        netsh(format!("advfirewall firewall delete rule name={RULE_DNS}")),
    ];
    if !blocked_ips.is_empty() {
        let ips: Vec<String> = blocked_ips.iter().map(Ipv4Addr::to_string).collect();
        out.push(netsh(format!(
            "advfirewall firewall add rule name={RULE_BLOCKED} dir=out action=block protocol=any remoteip={}",
            ips.join(",")
        )));
    }
    let mut allowed: BTreeSet<Ipv4Addr> = upstreams.iter().copied().collect();
    allowed.insert(Ipv4Addr::LOCALHOST);
    let remote = format!(
        "{},{IPV6_EXCEPT_LOOPBACK}",
        ranges_excluding(&allowed).join(",")
    );
    for protocol in ["UDP", "TCP"] {
        out.push(netsh(format!(
            "advfirewall firewall add rule name={RULE_DNS} dir=out action=block protocol={protocol} remoteport=53,853 remoteip={remote}"
        )));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    #[test]
    fn the_ranges_around_some_addresses_cover_everything_else() {
        let ranges = ranges_excluding(&BTreeSet::from([
            ip("1.1.1.1"),
            ip("127.0.0.1"),
            ip("9.9.9.9"),
        ]));
        assert_eq!(
            ranges,
            [
                "0.0.0.0-1.1.1.0",
                "1.1.1.2-9.9.9.8",
                "9.9.9.10-127.0.0.0",
                "127.0.0.2-255.255.255.255",
            ]
        );
    }

    #[test]
    fn the_edges_of_the_address_space_need_no_empty_range() {
        let ranges = ranges_excluding(&BTreeSet::from([ip("0.0.0.0"), ip("255.255.255.255")]));
        assert_eq!(ranges, ["0.0.0.1-255.255.255.254"]);
    }

    fn rendered(blocked: &[&str]) -> Vec<String> {
        let blocked = blocked.iter().map(|s| ip(s)).collect();
        rules(&blocked, &[ip("1.1.1.1")])
            .into_iter()
            .map(|args| args.join(" "))
            .collect()
    }

    #[test]
    fn dns_may_only_reach_the_filter_and_the_upstreams() {
        let rules = rendered(&[]);
        let dns: Vec<_> = rules
            .iter()
            .filter(|r| r.contains("add rule name=Cloister-DNS"))
            .collect();
        assert_eq!(dns.len(), 2, "{rules:#?}");
        for rule in &dns {
            assert!(rule.contains("dir=out action=block"));
            assert!(rule.contains("remoteip=0.0.0.0-1.1.1.0,1.1.1.2-127.0.0.0,127.0.0.2-255.255.255.255,::-::0,::2-ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"), "{rule}");
        }
        assert!(dns
            .iter()
            .any(|r| r.contains("protocol=UDP remoteport=53,853")));
        assert!(dns
            .iter()
            .any(|r| r.contains("protocol=TCP remoteport=53,853")));
    }

    #[test]
    fn old_rules_are_deleted_before_new_ones_are_added() {
        let rules = rendered(&["10.0.0.1"]);
        assert_eq!(
            rules[0],
            "advfirewall firewall delete rule name=Cloister-blocked-sites"
        );
        assert_eq!(
            rules[1],
            "advfirewall firewall delete rule name=Cloister-DNS"
        );
    }

    #[test]
    fn traffic_to_blocked_ips_is_dropped() {
        let rules = rendered(&["142.250.1.1", "10.0.0.1"]);
        assert!(rules.iter().any(|r| r
            == "advfirewall firewall add rule name=Cloister-blocked-sites dir=out action=block protocol=any remoteip=10.0.0.1,142.250.1.1"));
    }

    #[test]
    fn with_nothing_blocked_there_is_no_ip_rule_at_all() {
        // An empty remoteip would mean "any": it must never be rendered.
        let rules = rendered(&[]);
        assert!(!rules
            .iter()
            .any(|r| r.contains("add rule name=Cloister-blocked-sites")));
    }
}
