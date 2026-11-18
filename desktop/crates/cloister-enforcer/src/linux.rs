//! Linux: a systemd service, `nftables` ([`crate::nft`]), the hosts file fallback and the app
//! killer. The Shutdown Ritual window is a systemd *user* unit installed for every user,
//! started by a path unit while cloisterd holds the ritual flag and relaunched a second after
//! it exits; cloisterd also starts it in every logged-in user's manager on each tick while the
//! ritual is due, so stopping or disabling it only lasts a few seconds. Wayland can't lock the
//! screen, so this is "keeps reappearing", not a lock.
//!
//! The unit files and parsers are pure and compiled everywhere; [`LinuxPlatform`] only on Linux.
//! Exercised by `scripts/linux/smoke-test.sh` (in a privileged container, or as root).

use crate::apps::Process;

pub const UNIT: &str = "cloisterd.service";
pub const UNIT_PATH: &str = "/etc/systemd/system/cloisterd.service";
pub const RITUAL_UNIT: &str = "cloister-ritual.service";
pub const RITUAL_UNIT_PATH: &str = "/etc/systemd/user/cloister-ritual.service";
pub const RITUAL_PATH_UNIT: &str = "cloister-ritual.path";
pub const RITUAL_PATH_UNIT_PATH: &str = "/etc/systemd/user/cloister-ritual.path";
/// Exists while the ritual is due. Under /run (tmpfs): a reboot clears it.
pub const RITUAL_FLAG_PATH: &str = "/run/cloister-ritual-due";
pub const HOSTS_PATH: &str = "/etc/hosts";

/// The system service: started at boot, before the network is up, and restarted whenever it exits.
pub fn service_unit(binary: &str) -> String {
    format!(
        "[Unit]\n\
         Description=Cloister enforcement service\n\
         Wants=network-pre.target\n\
         Before=network-pre.target\n\
         \n\
         [Service]\n\
         ExecStart={binary} run\n\
         Restart=always\n\
         RestartSec=2\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n"
    )
}

/// The ritual window, as a user unit. Never gives up restarting while the flag exists.
pub fn ritual_service_unit(app: &str) -> String {
    format!(
        "[Unit]\n\
         Description=Cloister Shutdown Ritual\n\
         ConditionPathExists={RITUAL_FLAG_PATH}\n\
         StartLimitIntervalSec=0\n\
         After=graphical-session.target\n\
         \n\
         [Service]\n\
         ExecStart={app} --ritual\n\
         Restart=always\n\
         RestartSec=1\n"
    )
}

/// Starts the ritual unit as soon as the flag appears (and at login while it exists).
pub fn ritual_path_unit() -> String {
    format!(
        "[Unit]\n\
         Description=Start the Cloister Shutdown Ritual when it is due\n\
         \n\
         [Path]\n\
         PathExists={RITUAL_FLAG_PATH}\n\
         Unit={RITUAL_UNIT}\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n"
    )
}

/// `loginctl list-users --no-legend` output: the names of logged-in users with a uid of 1000
/// or more (people, not system accounts).
pub fn parse_loginctl_users(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let uid: u32 = fields.next()?.parse().ok()?;
            let name = fields.next()?;
            (uid >= 1000).then(|| name.to_string())
        })
        .collect()
}

/// A process from `/proc/<pid>` given the target of its `exe` link (`None` for kernel
/// threads and processes we may not inspect, which are skipped).
pub fn proc_process(pid: &str, exe: Option<&str>) -> Option<Process> {
    let exe = exe?.trim_end_matches(" (deleted)");
    Some(Process {
        pid: pid.parse().ok()?,
        executable: exe.to_string(),
    })
}

#[cfg(target_os = "linux")]
pub use platform::LinuxPlatform;

#[cfg(target_os = "linux")]
mod platform {
    use std::collections::BTreeSet;
    use std::io::{self, Write};
    use std::net::{Ipv4Addr, SocketAddr};
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use cloister_core::{Enforcement, RitualScreen};

    use super::*;
    use crate::domains::DomainSet;
    use crate::service::Platform;
    use crate::{apps, dns, hosts, nft};

    const RESOLVE_EVERY: Duration = Duration::from_secs(600);
    /// How often the loaded table is checked even when nothing changed (someone may flush it).
    const RECHECK_EVERY: Duration = Duration::from_secs(30);

    /// Applies Enforcement to Linux: `nftables` table, hosts file, app killer, ritual unit.
    pub struct LinuxPlatform {
        pub upstream: SocketAddr,
        pub hosts_path: PathBuf,
        last_domains: Option<DomainSet>,
        blocked_ips: BTreeSet<Ipv4Addr>,
        resolved_at: Option<Instant>,
        loaded_rules: Option<String>,
        checked_at: Option<Instant>,
    }

    impl LinuxPlatform {
        pub fn new(upstream: SocketAddr) -> Self {
            LinuxPlatform {
                upstream,
                hosts_path: HOSTS_PATH.into(),
                last_domains: None,
                blocked_ips: BTreeSet::new(),
                resolved_at: None,
                loaded_rules: None,
                checked_at: None,
            }
        }

        fn load_rules(&mut self) -> io::Result<()> {
            let rules = nft::render(&self.blocked_ips);
            let recheck = self.checked_at.is_none_or(|t| t.elapsed() > RECHECK_EVERY);
            let present = || run("nft", &["list", "table", "inet", nft::TABLE]).is_ok();
            if self.loaded_rules.as_deref() == Some(&rules) && !(recheck && !present()) {
                if recheck {
                    self.checked_at = Some(Instant::now());
                }
                return Ok(());
            }
            let mut child = Command::new("nft")
                .args(["-f", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()?;
            child
                .stdin
                .take()
                .expect("piped")
                .write_all(rules.as_bytes())?;
            let out = child.wait_with_output()?;
            if !out.status.success() {
                self.loaded_rules = None;
                return Err(io::Error::other(
                    String::from_utf8_lossy(&out.stderr).into_owned(),
                ));
            }
            self.loaded_rules = Some(rules);
            self.checked_at = Some(Instant::now());
            Ok(())
        }

        fn quit_blocked_apps(&self, blocked: &[String]) -> io::Result<()> {
            if blocked.is_empty() {
                return Ok(());
            }
            let mut processes = Vec::new();
            for entry in std::fs::read_dir("/proc")?.flatten() {
                let pid = entry.file_name().to_string_lossy().into_owned();
                if !pid.bytes().all(|b| b.is_ascii_digit()) {
                    continue;
                }
                let exe = std::fs::read_link(entry.path().join("exe")).ok();
                let exe = exe.as_ref().map(|p| p.to_string_lossy().into_owned());
                processes.extend(proc_process(&pid, exe.as_deref()));
            }
            for pid in apps::processes_to_quit_by_name(&processes, blocked) {
                // SAFETY: plain syscall; a pid that already exited just yields ESRCH.
                unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
            }
            Ok(())
        }

        fn resolve_blocked_ips(&mut self, domains: &DomainSet) {
            let mut ips = BTreeSet::new();
            for domain in domains.iter() {
                for name in [domain.to_string(), format!("www.{domain}")] {
                    if let Ok(found) = dns::resolve_a(self.upstream, &name) {
                        ips.extend(found);
                    }
                }
            }
            self.blocked_ips = ips;
            self.resolved_at = Some(Instant::now());
        }
    }

    /// While the ritual is due, start the ritual unit in every logged-in user's manager (it
    /// may have been stopped or its path unit disabled).
    fn keep_ritual_running() -> io::Result<()> {
        let users = parse_loginctl_users(&run("loginctl", &["list-users", "--no-legend"])?);
        for user in users {
            let machine = format!("{user}@");
            if run(
                "systemctl",
                &[
                    "--user",
                    "-M",
                    &machine,
                    "is-active",
                    "--quiet",
                    RITUAL_UNIT,
                ],
            )
            .is_err()
            {
                let _ = run(
                    "systemctl",
                    &["--user", "-M", &machine, "start", RITUAL_UNIT],
                );
            }
        }
        Ok(())
    }

    impl Platform for LinuxPlatform {
        fn apply(&mut self, enforcement: &Enforcement) -> io::Result<()> {
            let domains = DomainSet::new(&enforcement.blocked_domains);
            let changed = self.last_domains.as_ref() != Some(&domains);
            let stale = self.resolved_at.is_none_or(|t| t.elapsed() > RESOLVE_EVERY);
            if changed || stale {
                self.resolve_blocked_ips(&domains);
            }
            let mut errors = Vec::new();
            if let Err(e) = self.load_rules() {
                errors.push(format!("nftables: {e}"));
            }
            let existing = std::fs::read_to_string(&self.hosts_path).unwrap_or_default();
            let wanted = hosts::render(&existing, &domains);
            if wanted != existing {
                if let Err(e) = std::fs::write(&self.hosts_path, wanted) {
                    errors.push(format!("hosts: {e}"));
                }
            }
            if let Err(e) = self.quit_blocked_apps(&enforcement.blocked_apps) {
                errors.push(format!("apps: {e}"));
            }
            if enforcement.ritual == RitualScreen::Due {
                if let Err(e) = keep_ritual_running() {
                    errors.push(format!("ritual unit: {e}"));
                }
            }
            if changed {
                let _ = run("resolvectl", &["flush-caches"]);
                self.last_domains = Some(domains);
            }
            if errors.is_empty() {
                Ok(())
            } else {
                Err(io::Error::other(errors.join("; ")))
            }
        }
    }

    fn run(program: &str, args: &[&str]) -> io::Result<String> {
        let out = Command::new(program).args(args).output()?;
        if !out.status.success() {
            return Err(io::Error::other(format!(
                "{program} {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_people_get_a_ritual_window() {
        let out = "   0 root   no  active\n1000 ana    yes active\n 977 gdm    no  active\n1001 ben    no  online\n";
        assert_eq!(parse_loginctl_users(out), ["ana", "ben"]);
    }

    #[test]
    fn kernel_threads_are_skipped_and_replaced_binaries_still_match() {
        assert_eq!(proc_process("2", None), None);
        assert_eq!(
            proc_process("4242", Some("/usr/share/discord/Discord (deleted)")),
            Some(Process {
                pid: 4242,
                executable: "/usr/share/discord/Discord".into()
            })
        );
    }
}
