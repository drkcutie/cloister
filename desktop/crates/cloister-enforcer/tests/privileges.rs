//! `cloisterd run` touches the system (DNS settings, pf, hosts file) only as root. As an admin
//! user without root it could change the DNS servers but not the firewall or hosts file, and a
//! half-applied enforcement broke DNS. So it refuses to start unless it is root or dry-running.

use cloister_enforcer::privileges::may_enforce;

#[test]
fn a_dry_run_needs_no_root() {
    assert!(may_enforce(true, 501).is_ok());
    assert!(may_enforce(true, 0).is_ok());
}

#[test]
fn real_enforcement_refuses_to_start_unless_root() {
    let refusal = may_enforce(false, 501).unwrap_err();
    assert!(refusal.contains("root"), "{refusal}");
    assert!(refusal.contains("--dry-run"), "{refusal}");
    assert!(may_enforce(false, 0).is_ok());
}
