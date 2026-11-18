//! Matching DNS names against an Enforcement's blocked domains.

use std::collections::BTreeSet;

/// Always answered with NXDOMAIN so Firefox turns off its own DNS-over-HTTPS
/// and keeps using the system resolver (our filter).
pub const FIREFOX_DOH_CANARY: &str = "use-application-dns.net";

/// A set of blocked domains. A domain blocks itself and every subdomain.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DomainSet {
    domains: BTreeSet<String>,
}

impl DomainSet {
    pub fn new<I, S>(domains: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        DomainSet {
            domains: domains.into_iter().map(|d| normalize(d.as_ref())).collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.domains.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.domains.iter().map(String::as_str)
    }

    /// Whether `name` (any case, with or without a trailing dot) is blocked.
    pub fn is_blocked(&self, name: &str) -> bool {
        let name = normalize(name);
        if name == FIREFOX_DOH_CANARY {
            return true;
        }
        // Walk the name's suffixes on label boundaries: a.b.c -> b.c -> c.
        let mut rest = name.as_str();
        loop {
            if self.domains.contains(rest) {
                return true;
            }
            match rest.find('.') {
                Some(i) => rest = &rest[i + 1..],
                None => return false,
            }
        }
    }
}

fn normalize(name: &str) -> String {
    name.trim().trim_end_matches('.').to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_the_domain_its_subdomains_and_nothing_that_only_looks_alike() {
        let set = DomainSet::new(["youtube.com", "youtu.be"]);
        assert!(set.is_blocked("youtube.com"));
        assert!(set.is_blocked("WWW.YouTube.com."));
        assert!(set.is_blocked("m.youtube.com"));
        assert!(set.is_blocked("a.b.c.youtube.com"));
        assert!(set.is_blocked("youtu.be"));
        assert!(!set.is_blocked("notyoutube.com"));
        assert!(!set.is_blocked("youtube.com.evil.example"));
        assert!(!set.is_blocked("com"));
        assert!(!set.is_blocked("example.com"));
    }

    #[test]
    fn always_answers_the_firefox_doh_canary() {
        assert!(DomainSet::default().is_blocked("use-application-dns.net"));
    }
}
