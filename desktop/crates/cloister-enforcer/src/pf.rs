//! `pf` rules (ADR 0002): force all DNS through the local filter and drop traffic to the
//! resolved IPs of blocked domains. Loaded into the `com.apple/cloister` anchor, which the
//! stock macOS `/etc/pf.conf` already evaluates (`anchor "com.apple/*"`).

use std::collections::BTreeSet;
use std::net::Ipv4Addr;

pub const ANCHOR: &str = "com.apple/cloister";
pub const TABLE: &str = "cloister_blocked";

/// The anchor's rules. Only root (the enforcement service's forwarder) may send DNS or
/// DNS-over-TLS anywhere other than the loopback filter.
pub fn render(blocked_ips: &BTreeSet<Ipv4Addr>) -> String {
    let ips: Vec<String> = blocked_ips.iter().map(Ipv4Addr::to_string).collect();
    let table = if ips.is_empty() {
        format!("table <{TABLE}> persist\n")
    } else {
        format!("table <{TABLE}> persist {{ {} }}\n", ips.join(", "))
    };
    format!(
        "{table}\
         block drop out quick proto {{ tcp, udp }} from any to <{TABLE}>\n\
         pass out quick proto {{ tcp, udp }} from any to any port {{ 53, 853 }} user root\n\
         block drop out quick proto {{ tcp, udp }} from any to ! 127.0.0.1 port {{ 53, 853 }}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forces_dns_through_the_filter_even_with_nothing_blocked() {
        let rules = render(&BTreeSet::new());
        assert!(rules.contains("table <cloister_blocked> persist\n"));
        assert!(rules.contains("port { 53, 853 } user root"));
        assert!(rules.contains("to ! 127.0.0.1 port { 53, 853 }"));
    }

    #[test]
    fn drops_traffic_to_blocked_ips() {
        let ips = BTreeSet::from([Ipv4Addr::new(142, 250, 1, 1), Ipv4Addr::new(10, 0, 0, 1)]);
        let rules = render(&ips);
        assert!(rules.contains("table <cloister_blocked> persist { 10.0.0.1, 142.250.1.1 }"));
        // The IP block comes before the root DNS pass so root can't reach blocked IPs either.
        let block = rules.find("to <cloister_blocked>").unwrap();
        let pass = rules.find("user root").unwrap();
        assert!(block < pass);
    }
}
