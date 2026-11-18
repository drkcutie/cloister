//! Windows: a Windows Service (`cloisterd service`, registered by `cloisterd install`), Windows
//! Firewall rules ([`crate::winfw`]), every adapter's DNS pointed at the local filter, the
//! hosts file fallback and the app killer. While the Shutdown Ritual is due the service starts
//! the ritual window in the console user's session and starts it again whenever it exits.
//!
//! Paths and scripts are pure and compiled everywhere; [`WindowsPlatform`] only on Windows.
//! Exercised by `scripts/windows/smoke-test.ps1` (run as Administrator).

pub const SERVICE_NAME: &str = "Cloister";
pub const HOSTS_PATH: &str = r"C:\Windows\System32\drivers\etc\hosts";
pub const DATA_DIR: &str = r"C:\ProgramData\Cloister";
pub const STATE_PATH: &str = r"C:\ProgramData\Cloister\state.json";
/// AF_UNIX socket (Windows 10 1803 and later).
pub const SOCKET_PATH: &str = r"C:\ProgramData\Cloister\cloister.sock";

/// Point every connected adapter's IPv4 DNS at the local filter. (The firewall drops DNS to
/// anything else, so IPv6 resolvers just fail over to it.)
pub const DNS_SETTINGS_SCRIPT: &str = "Get-NetAdapter | Where-Object Status -eq 'Up' | \
ForEach-Object { Set-DnsClientServerAddress -InterfaceIndex $_.ifIndex -ServerAddresses 127.0.0.1 }; \
Clear-DnsClientCache";

/// Whether the ritual window must be started now.
pub fn launch_ritual(due: bool, window_running: bool) -> bool {
    due && !window_running
}

#[cfg(windows)]
pub use platform::WindowsPlatform;

#[cfg(windows)]
mod platform {
    use std::collections::BTreeSet;
    use std::io;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::os::windows::process::CommandExt;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::{Duration, Instant};

    use cloister_core::{Enforcement, RitualScreen};

    use super::*;
    use crate::domains::DomainSet;
    use crate::service::Platform;
    use crate::{apps, dns, hosts, winfw};

    const RESOLVE_EVERY: Duration = Duration::from_secs(600);
    const RECHECK_EVERY: Duration = Duration::from_secs(30);
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub struct WindowsPlatform {
        pub upstreams: Vec<SocketAddr>,
        pub hosts_path: PathBuf,
        /// The app that shows the ritual (`cloister-app.exe --ritual`), if known.
        pub app: Option<String>,
        last_domains: Option<DomainSet>,
        blocked_ips: BTreeSet<Ipv4Addr>,
        resolved_at: Option<Instant>,
        loaded_rules: Option<Vec<Vec<String>>>,
        checked_at: Option<Instant>,
        ritual: Option<session::Child>,
    }

    impl WindowsPlatform {
        pub fn new(upstreams: Vec<SocketAddr>, app: Option<String>) -> Self {
            WindowsPlatform {
                upstreams,
                hosts_path: HOSTS_PATH.into(),
                app,
                last_domains: None,
                blocked_ips: BTreeSet::new(),
                resolved_at: None,
                loaded_rules: None,
                checked_at: None,
                ritual: None,
            }
        }

        fn upstream_ips(&self) -> Vec<Ipv4Addr> {
            self.upstreams
                .iter()
                .filter_map(|a| match a.ip() {
                    IpAddr::V4(ip) => Some(ip),
                    IpAddr::V6(_) => None,
                })
                .collect()
        }

        /// Firewall rules and DNS settings: re-applied when they change, and every
        /// [`RECHECK_EVERY`] when someone deleted the rules.
        fn apply_network(&mut self) -> io::Result<()> {
            let rules = winfw::rules(&self.blocked_ips, &self.upstream_ips());
            let recheck = self.checked_at.is_none_or(|t| t.elapsed() > RECHECK_EVERY);
            let unchanged = self.loaded_rules.as_ref() == Some(&rules);
            if unchanged && !recheck {
                return Ok(());
            }
            let present = run(
                "netsh",
                &[
                    "advfirewall",
                    "firewall",
                    "show",
                    "rule",
                    &format!("name={}", winfw::RULE_DNS),
                ],
            )
            .is_ok();
            if !unchanged || !present {
                for (i, args) in rules.iter().enumerate() {
                    let args: Vec<&str> = args.iter().map(String::as_str).collect();
                    let result = run("netsh", &args);
                    // The first two delete old rules; there may be none.
                    if i >= 2 {
                        result?;
                    }
                }
                self.loaded_rules = Some(rules);
            }
            run(
                "powershell.exe",
                &[
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    DNS_SETTINGS_SCRIPT,
                ],
            )?;
            self.checked_at = Some(Instant::now());
            Ok(())
        }

        fn quit_blocked_apps(&self, blocked: &[String]) -> io::Result<()> {
            if blocked.is_empty() {
                return Ok(());
            }
            let processes = apps::parse_tasklist_csv(&run("tasklist", &["/fo", "csv", "/nh"])?);
            for pid in apps::processes_to_quit_by_name(&processes, blocked) {
                let _ = run("taskkill", &["/F", "/PID", &pid.to_string()]);
            }
            Ok(())
        }

        fn keep_ritual_window(&mut self, due: bool) -> io::Result<()> {
            let running = self.ritual.as_ref().is_some_and(session::Child::is_running);
            if !running {
                self.ritual = None;
            }
            if !launch_ritual(due, running) {
                return Ok(());
            }
            let Some(app) = &self.app else {
                return Err(io::Error::other("no --app given; can't show the ritual"));
            };
            // No one logged in: nothing to show yet; the next tick after login starts it.
            self.ritual = session::spawn_in_console_session(&format!("\"{app}\" --ritual"))?;
            Ok(())
        }

        fn resolve_blocked_ips(&mut self, domains: &DomainSet) {
            let mut ips = BTreeSet::new();
            for domain in domains.iter() {
                for name in [domain.to_string(), format!("www.{domain}")] {
                    if let Some(found) = self
                        .upstreams
                        .first()
                        .and_then(|u| dns::resolve_a(*u, &name).ok())
                    {
                        ips.extend(found);
                    }
                }
            }
            self.blocked_ips = ips;
            self.resolved_at = Some(Instant::now());
        }
    }

    impl Platform for WindowsPlatform {
        fn apply(&mut self, enforcement: &Enforcement) -> io::Result<()> {
            let domains = DomainSet::new(&enforcement.blocked_domains);
            let changed = self.last_domains.as_ref() != Some(&domains);
            let stale = self.resolved_at.is_none_or(|t| t.elapsed() > RESOLVE_EVERY);
            if changed || stale {
                self.resolve_blocked_ips(&domains);
            }
            let mut errors = Vec::new();
            if let Err(e) = self.apply_network() {
                errors.push(format!("firewall/dns: {e}"));
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
            if let Err(e) = self.keep_ritual_window(enforcement.ritual == RitualScreen::Due) {
                errors.push(format!("ritual window: {e}"));
            }
            if changed {
                let _ = run("ipconfig", &["/flushdns"]);
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
        let out = Command::new(program)
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()?;
        if !out.status.success() {
            return Err(io::Error::other(format!(
                "{program} {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Starting a process in the logged-in user's session from a service (session 0).
    mod session {
        use std::io;
        use std::ptr::null_mut;

        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_TIMEOUT};
        use windows_sys::Win32::System::Environment::{
            CreateEnvironmentBlock, DestroyEnvironmentBlock,
        };
        use windows_sys::Win32::System::RemoteDesktop::{
            WTSGetActiveConsoleSessionId, WTSQueryUserToken,
        };
        use windows_sys::Win32::System::Threading::{
            CreateProcessAsUserW, WaitForSingleObject, CREATE_UNICODE_ENVIRONMENT,
            PROCESS_INFORMATION, STARTUPINFOW,
        };

        pub struct Child(HANDLE);

        // SAFETY: a process handle may be used from any thread.
        unsafe impl Send for Child {}

        impl Child {
            pub fn is_running(&self) -> bool {
                // SAFETY: self.0 is a valid process handle until drop.
                unsafe { WaitForSingleObject(self.0, 0) == WAIT_TIMEOUT }
            }
        }

        impl Drop for Child {
            fn drop(&mut self) {
                // SAFETY: closing our own handle once.
                unsafe { CloseHandle(self.0) };
            }
        }

        fn wide(s: &str) -> Vec<u16> {
            s.encode_utf16().chain(std::iter::once(0)).collect()
        }

        /// Run `command_line` on the interactive desktop of the console session's user.
        /// `Ok(None)` when nobody is logged in.
        pub fn spawn_in_console_session(command_line: &str) -> io::Result<Option<Child>> {
            // SAFETY: plain Win32 calls; every handle we get is closed below or owned by Child.
            unsafe {
                let session = WTSGetActiveConsoleSessionId();
                if session == u32::MAX {
                    return Ok(None);
                }
                let mut token: HANDLE = null_mut();
                if WTSQueryUserToken(session, &mut token) == 0 {
                    return Ok(None);
                }
                let mut env = null_mut();
                let have_env = CreateEnvironmentBlock(&mut env, token, 0) != 0;
                let mut desktop = wide("winsta0\\default");
                let mut startup: STARTUPINFOW = std::mem::zeroed();
                startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
                startup.lpDesktop = desktop.as_mut_ptr();
                let mut info: PROCESS_INFORMATION = std::mem::zeroed();
                let mut cmd = wide(command_line);
                let ok = CreateProcessAsUserW(
                    token,
                    std::ptr::null(),
                    cmd.as_mut_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    0,
                    CREATE_UNICODE_ENVIRONMENT,
                    if have_env { env } else { null_mut() },
                    std::ptr::null(),
                    &startup,
                    &mut info,
                );
                let error = io::Error::last_os_error();
                if have_env {
                    DestroyEnvironmentBlock(env);
                }
                CloseHandle(token);
                if ok == 0 {
                    return Err(error);
                }
                CloseHandle(info.hThread);
                Ok(Some(Child(info.hProcess)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ritual_window_is_started_only_while_due_and_not_already_showing() {
        assert!(launch_ritual(true, false));
        assert!(!launch_ritual(true, true));
        assert!(!launch_ritual(false, false));
    }
}
