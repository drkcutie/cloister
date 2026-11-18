//! Who may run real enforcement. Applying an Enforcement means changing the system resolver,
//! the firewall and the hosts file together; anything less (an admin user can change DNS
//! servers without root, but not pf or /etc/hosts) leaves a half-applied state that is worse
//! than none. So `cloisterd run` refuses to start unless it is privileged (root; an elevated
//! Administrator or LocalSystem on Windows) or `--dry-run`.

/// `Ok` when a service that is `privileged` (see [`is_privileged`]) may start, `Err` with the
/// reason otherwise.
pub fn may_enforce(dry_run: bool, privileged: bool) -> Result<(), String> {
    if dry_run || privileged {
        Ok(())
    } else {
        Err(
            "enforcement needs root (Administrator on Windows; it is installed as a system \
             service); nothing was changed. Use --dry-run to run unprivileged."
                .into(),
        )
    }
}

/// Whether this process runs as root (Unix) or elevated (Windows).
#[cfg(unix)]
pub fn is_privileged() -> bool {
    // SAFETY: geteuid has no preconditions and can't fail.
    unsafe { libc::geteuid() == 0 }
}

/// Whether this process runs as root (Unix) or elevated (Windows).
#[cfg(windows)]
pub fn is_privileged() -> bool {
    // SAFETY: no preconditions; true for an elevated Administrator and for LocalSystem.
    unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
}
