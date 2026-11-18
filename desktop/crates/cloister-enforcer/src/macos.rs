//! The privileged macOS layer. Deliberately thin: everything it writes is rendered by the
//! unprivileged, tested modules ([`crate::hosts`], [`crate::pf`]) or by pure functions here.
//! Exercised by `scripts/macos/smoke-test.sh` (needs root).

use std::collections::{BTreeSet, HashMap};
use std::io::{self, Write};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use cloister_core::{Enforcement, RitualScreen};

use crate::domains::DomainSet;
use crate::service::Platform;
use crate::{apps, dns, hosts, pf};

pub const LABEL: &str = "com.cloister.enforcer";
pub const PLIST_PATH: &str = "/Library/LaunchDaemons/com.cloister.enforcer.plist";
pub const HOSTS_PATH: &str = "/etc/hosts";
const RESOLVE_EVERY: Duration = Duration::from_secs(600);
/// The per-user launch agent that shows the Shutdown Ritual window.
pub const RITUAL_LABEL: &str = "com.cloister.ritual";
/// Root-owned, so it loads in every GUI login and the user can't edit or delete it.
pub const RITUAL_PLIST_PATH: &str = "/Library/LaunchAgents/com.cloister.ritual.plist";
/// Exists while the ritual is due (written by cloisterd). Under /var/run, so a reboot clears it
/// and the first tick after boot decides afresh, before anyone logs in.
pub const RITUAL_FLAG_PATH: &str = "/var/run/cloister-ritual-due";

/// The ritual window's launch agent. launchd keeps it running while `flag` exists: it starts
/// at login if the ritual is due, and is relaunched one second after it exits or is killed.
/// Nothing runs while the flag is absent.
pub fn ritual_agent_plist(app: &str, flag: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{RITUAL_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{app}</string>
        <string>--ritual</string>
    </array>
    <key>LimitLoadToSessionType</key>
    <string>Aqua</string>
    <key>ProcessType</key>
    <string>Interactive</string>
    <key>KeepAlive</key>
    <dict>
        <key>PathState</key>
        <dict>
            <key>{flag}</key>
            <true/>
        </dict>
    </dict>
    <key>ThrottleInterval</key>
    <integer>1</integer>
</dict>
</plist>
"#
    )
}

/// The user id that owns the GUI session on the main display, if someone is logged in.
fn console_user() -> Option<u32> {
    let uid: u32 = run("/usr/bin/stat", &["-f", "%u", "/dev/console"])
        .ok()?
        .trim()
        .parse()
        .ok()?;
    (uid != 0).then_some(uid)
}

/// While the ritual is due, make sure the ritual agent is loaded in the console user's GUI
/// domain (a user can `launchctl bootout` their own agents; root loads it back).
fn keep_ritual_agent_loaded() -> io::Result<()> {
    let Some(uid) = console_user() else {
        return Ok(());
    };
    let domain = format!("gui/{uid}");
    if run(
        "/bin/launchctl",
        &["print", &format!("{domain}/{RITUAL_LABEL}")],
    )
    .is_err()
    {
        run("/bin/launchctl", &["bootstrap", &domain, RITUAL_PLIST_PATH])?;
    }
    Ok(())
}

/// The launchd daemon definition: starts at boot and is relaunched whenever it exits.
pub fn launchd_plist(binary: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{binary}</string>
        <string>run</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ThrottleInterval</key>
    <integer>2</integer>
    <key>StandardErrorPath</key>
    <string>/var/log/cloisterd.log</string>
</dict>
</plist>
"#
    )
}

/// Network service names from `networksetup -listallnetworkservices` (disabled ones too).
pub fn parse_network_services(output: &str) -> Vec<String> {
    output
        .lines()
        .skip(1) // "An asterisk (*) denotes that a network service is disabled."
        .map(|l| l.trim_start_matches('*').trim())
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Applies Enforcement to macOS: system DNS -> local filter, `pf` anchor, hosts file.
pub struct MacPlatform {
    pub upstream: SocketAddr,
    pub hosts_path: PathBuf,
    last_domains: Option<DomainSet>,
    blocked_ips: BTreeSet<Ipv4Addr>,
    resolved_at: Option<Instant>,
    /// App bundle path -> bundle identifier, read once from its Info.plist.
    bundle_ids: HashMap<String, Option<String>>,
}

impl MacPlatform {
    pub fn new(upstream: SocketAddr) -> Self {
        MacPlatform {
            upstream,
            hosts_path: HOSTS_PATH.into(),
            last_domains: None,
            blocked_ips: BTreeSet::new(),
            resolved_at: None,
            bundle_ids: HashMap::new(),
        }
    }

    fn point_system_dns_at_filter(&self) -> io::Result<()> {
        let list = run("networksetup", &["-listallnetworkservices"])?;
        for service in parse_network_services(&list) {
            let current = run("networksetup", &["-getdnsservers", &service]).unwrap_or_default();
            if current.trim() != "127.0.0.1" {
                run("networksetup", &["-setdnsservers", &service, "127.0.0.1"])?;
            }
        }
        Ok(())
    }

    /// Force-quit every running process of a blocked app (decided by [`apps`]).
    fn quit_blocked_apps(&mut self, blocked: &[String]) -> io::Result<()> {
        if blocked.is_empty() {
            return Ok(());
        }
        let processes = apps::parse_ps(&run("/bin/ps", &["-axo", "pid=,comm="])?);
        let cache = &mut self.bundle_ids;
        let pids = apps::processes_to_quit(&processes, blocked, |path| {
            cache
                .entry(path.to_string())
                .or_insert_with(|| read_bundle_id(path))
                .clone()
        });
        for pid in pids {
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

impl Platform for MacPlatform {
    fn apply(&mut self, enforcement: &Enforcement) -> io::Result<()> {
        let domains = DomainSet::new(&enforcement.blocked_domains);
        let changed = self.last_domains.as_ref() != Some(&domains);
        let stale = self.resolved_at.is_none_or(|t| t.elapsed() > RESOLVE_EVERY);
        if changed || stale {
            self.resolve_blocked_ips(&domains);
        }

        let mut errors = Vec::new();
        if let Err(e) = self.point_system_dns_at_filter() {
            errors.push(format!("dns settings: {e}"));
        }
        if let Err(e) = load_pf(&pf::render(&self.blocked_ips)) {
            errors.push(format!("pf: {e}"));
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
            if let Err(e) = keep_ritual_agent_loaded() {
                errors.push(format!("ritual agent: {e}"));
            }
        }
        if changed {
            let _ = run("dscacheutil", &["-flushcache"]);
            let _ = run("killall", &["-HUP", "mDNSResponder"]);
            self.last_domains = Some(domains);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(io::Error::other(errors.join("; ")))
        }
    }
}

fn load_pf(rules: &str) -> io::Result<()> {
    let mut child = Command::new("/sbin/pfctl")
        .args(["-q", "-a", pf::ANCHOR, "-f", "-"])
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
        return Err(io::Error::other(
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ));
    }
    // Enable pf; "already enabled" is fine.
    let _ = Command::new("/sbin/pfctl").args(["-q", "-e"]).output();
    Ok(())
}

fn read_bundle_id(app: &str) -> Option<String> {
    let plist = format!("{app}/Contents/Info.plist");
    run(
        "/usr/bin/plutil",
        &["-extract", "CFBundleIdentifier", "raw", "-o", "-", &plist],
    )
    .ok()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_every_network_service_including_disabled_ones() {
        let out = "An asterisk (*) denotes that a network service is disabled.\nWi-Fi\nThunderbolt Bridge\n*USB 10/100/1000 LAN\n";
        assert_eq!(
            parse_network_services(out),
            ["Wi-Fi", "Thunderbolt Bridge", "USB 10/100/1000 LAN"]
        );
    }

    #[test]
    fn the_ritual_agent_is_kept_alive_only_while_the_flag_exists() {
        let plist = ritual_agent_plist(
            "/Applications/Cloister.app/Contents/MacOS/cloister-app",
            RITUAL_FLAG_PATH,
        );
        assert!(plist.contains("<string>--ritual</string>"));
        assert!(plist.contains("<key>PathState</key>\n        <dict>\n            <key>/var/run/cloister-ritual-due</key>\n            <true/>"));
        assert!(plist.contains("<key>ThrottleInterval</key>\n    <integer>1</integer>"));
        assert!(
            !plist.contains("RunAtLoad"),
            "nothing runs while the ritual isn't due"
        );
    }

    #[test]
    fn the_daemon_runs_at_boot_and_is_kept_alive() {
        let plist = launchd_plist("/usr/local/libexec/cloisterd");
        assert!(plist.contains("<string>/usr/local/libexec/cloisterd</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>\n    <true/>"));
        assert!(plist.contains("<key>KeepAlive</key>\n    <true/>"));
    }
}
