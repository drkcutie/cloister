//! `nftables` rules for Linux (ADR 0002), the counterpart of [`crate::pf`]: every DNS query a
//! non-root process sends is redirected to the local filter, DNS over TCP and DNS-over-TLS from
//! non-root processes is dropped, and traffic to the resolved IPs of blocked domains is dropped.
//! Loaded atomically with `nft -f -` into Cloister's own table, so nothing else is touched.

use std::collections::BTreeSet;
use std::net::Ipv4Addr;

pub const TABLE: &str = "cloister";

/// The whole `cloister` table, replacing any previous one in the same transaction.
pub fn render(blocked_ips: &BTreeSet<Ipv4Addr>) -> String {
    let elements = if blocked_ips.is_empty() {
        String::new()
    } else {
        let ips: Vec<String> = blocked_ips.iter().map(Ipv4Addr::to_string).collect();
        format!("\n    elements = {{ {} }}", ips.join(", "))
    };
    format!(
        "table inet {TABLE} {{}}\n\
         delete table inet {TABLE}\n\
         table inet {TABLE} {{\n\
         \x20 set blocked {{\n\
         \x20   type ipv4_addr{elements}\n\
         \x20 }}\n\
         \x20 chain dns_redirect {{\n\
         \x20   type nat hook output priority -100; policy accept;\n\
         \x20   meta skuid != 0 udp dport 53 redirect to :53\n\
         \x20 }}\n\
         \x20 chain output {{\n\
         \x20   type filter hook output priority 0; policy accept;\n\
         \x20   ip daddr @blocked drop\n\
         \x20   meta skuid != 0 tcp dport {{ 53, 853 }} drop\n\
         \x20   meta skuid != 0 udp dport 853 drop\n\
         \x20 }}\n\
         }}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirects_dns_to_the_filter_even_with_nothing_blocked() {
        let rules = render(&BTreeSet::new());
        assert!(rules.contains("type nat hook output priority -100"));
        assert!(rules.contains("meta skuid != 0 udp dport 53 redirect to :53"));
        // DNS over TCP and DoT can't be redirected to a UDP filter: dropped for non-root.
        assert!(rules.contains("meta skuid != 0 tcp dport { 53, 853 } drop"));
        assert!(rules.contains("meta skuid != 0 udp dport 853 drop"));
        assert!(rules.contains("set blocked {\n    type ipv4_addr\n  }"));
    }

    #[test]
    fn replaces_its_own_table_atomically() {
        let rules = render(&BTreeSet::new());
        // Creating then deleting first makes the load work whether or not the table exists.
        assert!(rules.starts_with(
            "table inet cloister {}\ndelete table inet cloister\ntable inet cloister {"
        ));
    }

    #[test]
    fn drops_traffic_to_blocked_ips_for_every_user() {
        let ips = BTreeSet::from([Ipv4Addr::new(142, 250, 1, 1), Ipv4Addr::new(10, 0, 0, 1)]);
        let rules = render(&ips);
        assert!(rules.contains("elements = { 10.0.0.1, 142.250.1.1 }"));
        let drop = rules.find("ip daddr @blocked drop").unwrap();
        let dns = rules.find("tcp dport { 53, 853 } drop").unwrap();
        assert!(
            drop < dns,
            "the IP drop comes first, so root can't reach blocked IPs either"
        );
        assert!(
            rules.contains("\n    ip daddr @blocked drop\n"),
            "for every user"
        );
    }
}
