//! The hosts file fallback (ADR 0002). It can only list exact names, so each blocked domain
//! gets itself and its `www.` name. Cloister owns one marked section and never touches the rest.

use crate::domains::DomainSet;

pub const BEGIN: &str =
    "# BEGIN cloister (managed by the Cloister enforcement service; do not edit)";
pub const END: &str = "# END cloister";

/// `existing` hosts file contents with Cloister's section replaced to block `blocked`.
/// With nothing blocked, the section is removed.
pub fn render(existing: &str, blocked: &DomainSet) -> String {
    let mut out = String::with_capacity(existing.len());
    let mut inside = false;
    for line in existing.lines() {
        if line == BEGIN {
            inside = true;
        } else if line == END && inside {
            inside = false;
        } else if !inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !blocked.is_empty() {
        out.push_str(BEGIN);
        out.push('\n');
        for domain in blocked.iter() {
            for name in [domain.to_string(), format!("www.{domain}")] {
                out.push_str(&format!("0.0.0.0\t{name}\n::\t{name}\n"));
            }
        }
        out.push_str(END);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const STOCK: &str = "127.0.0.1\tlocalhost\n255.255.255.255\tbroadcasthost\n::1\tlocalhost\n";

    #[test]
    fn adds_a_section_blocking_each_domain_and_its_www_name() {
        let out = render(STOCK, &DomainSet::new(["youtu.be"]));
        assert!(out.starts_with(STOCK));
        assert!(out.contains("0.0.0.0\tyoutu.be\n"));
        assert!(out.contains("::\tyoutu.be\n"));
        assert!(out.contains("0.0.0.0\twww.youtu.be\n"));
        assert!(out.trim_end().ends_with(END));
    }

    #[test]
    fn replaces_its_own_section_and_keeps_user_lines_after_it() {
        let first = render(STOCK, &DomainSet::new(["youtu.be"]));
        let edited = format!("{first}10.0.0.5\tnas.local\n");
        let out = render(&edited, &DomainSet::new(["reddit.com"]));
        assert!(!out.contains("youtu.be"));
        assert!(out.contains("0.0.0.0\treddit.com\n"));
        assert!(out.contains("10.0.0.5\tnas.local\n"));
        assert_eq!(out.matches(BEGIN).count(), 1);
    }

    #[test]
    fn removes_the_section_when_nothing_is_blocked() {
        let blocked = render(STOCK, &DomainSet::new(["youtu.be"]));
        assert_eq!(render(&blocked, &DomainSet::default()), STOCK);
    }
}
