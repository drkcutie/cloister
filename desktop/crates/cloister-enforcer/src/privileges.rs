//! Who may run real enforcement. Applying an Enforcement means changing the system resolver,
//! the firewall and the hosts file together; anything less (an admin user can change DNS
//! servers without root, but not pf or /etc/hosts) leaves a half-applied state that is worse
//! than none. So `cloisterd run` refuses to start unless it is root or `--dry-run`.

/// `Ok` when a service with effective user id `euid` may start, `Err` with the reason otherwise.
pub fn may_enforce(dry_run: bool, euid: u32) -> Result<(), String> {
    if dry_run || euid == 0 {
        Ok(())
    } else {
        Err(
            "enforcement needs root (it is installed as a launchd daemon); \
             nothing was changed. Use --dry-run to run unprivileged."
                .into(),
        )
    }
}

/// The effective user id of this process.
pub fn effective_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and can't fail.
    unsafe { libc::geteuid() }
}
