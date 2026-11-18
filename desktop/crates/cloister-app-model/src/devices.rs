//! The Devices screen and the signed-in account: every Device with a status word, and this
//! Device's sync state ("Offline, still enforcing").

use chrono::{DateTime, Utc};
use cloister_core::{Command, State};
use cloister_sync::{
    Backend, CommandOutcome, Device, DeviceInfo, DeviceStatus, DeviceSummary, Platform, SyncStatus,
};

use crate::blocklists::Rules;
use crate::format::{clock, Zone};
use crate::notice::Notice;

/// What the Devices screen shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevicesView {
    /// "Online" or "Offline, still enforcing".
    pub sync: &'static str,
    pub online: bool,
    pub rows: Vec<DeviceRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRow {
    pub name: String,
    pub platform: &'static str,
    /// "Enforcing", "Enforcing with Hardcore Mode" or "Offline": a word beside a square.
    pub status: &'static str,
    pub enforcing: bool,
    pub this_device: bool,
    /// "Seen at 11:42." or "Seen on Mon 5 Jan."
    pub seen: String,
}

pub fn platform_name(p: Platform) -> &'static str {
    match p {
        Platform::Macos => "macOS",
        Platform::Windows => "Windows",
        Platform::Linux => "Linux",
        Platform::Android => "Android",
    }
}

/// The Devices screen for `devices` (as last listed) and this Device's `sync` status.
pub fn devices_view(
    devices: &[DeviceSummary],
    sync: SyncStatus,
    now: DateTime<Utc>,
    zone: &Zone,
) -> DevicesView {
    let day = |t: DateTime<Utc>| match zone {
        Zone::Local => t.with_timezone(&chrono::Local).date_naive(),
        Zone::Utc => t.date_naive(),
    };
    DevicesView {
        sync: sync.label(),
        online: sync == SyncStatus::Online,
        rows: devices
            .iter()
            .map(|d| DeviceRow {
                name: d.name.clone(),
                platform: platform_name(d.platform),
                status: match (d.status, d.hardcore) {
                    (DeviceStatus::Enforcing, true) => "Enforcing with Hardcore Mode",
                    (status, _) => status.label(),
                },
                enforcing: d.status == DeviceStatus::Enforcing,
                this_device: d.this_device,
                seen: if day(d.last_seen_at) == day(now) {
                    format!("Seen at {}.", clock(d.last_seen_at, zone))
                } else {
                    format!("Seen on {}.", day(d.last_seen_at).format("%a %-d %b"))
                },
            })
            .collect(),
    }
}

/// Where this user's Cloister data lives (the mirror, the onboarding marker):
/// `~/Library/Application Support/Cloister` on macOS, `%APPDATA%\Cloister` on Windows,
/// `$XDG_DATA_HOME/cloister` (or `~/.local/share/cloister`) on Linux.
pub fn data_dir() -> std::path::PathBuf {
    use std::path::PathBuf;
    let home = || PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    if cfg!(target_os = "macos") {
        home().join("Library/Application Support/Cloister")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("Cloister")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".local/share"))
            .join("cloister")
    }
}

/// This computer's name for the Devices screen.
fn computer_name() -> String {
    let output = |program: &str, args: &[&str]| {
        std::process::Command::new(program)
            .args(args)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|n| !n.is_empty())
    };
    if cfg!(target_os = "macos") {
        output("scutil", &["--get", "ComputerName"]).unwrap_or_else(|| "This Mac".into())
    } else {
        std::env::var("COMPUTERNAME")
            .ok()
            .or_else(|| output("hostname", &[]))
            .unwrap_or_else(|| "This computer".into())
    }
}

/// This Device's account, when the backend is configured (`CLOISTER_API_URL`,
/// `CLOISTER_ANON_KEY`). It shares its mirror with `cloister-device` (`$CLOISTER_MIRROR`, else
/// the user data dir), so signing in there signs the app in too.
pub fn open_account() -> Option<Device> {
    let backend = Backend::from_env()?;
    let mirror = std::env::var_os("CLOISTER_MIRROR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| data_dir().join("mirror.sqlite"));
    Device::open(
        backend,
        mirror,
        DeviceInfo {
            name: computer_name(),
            platform: Platform::current(),
        },
    )
    .ok()
}

/// The signed-in account as the authority for Blocklist edits (loosening waits until tomorrow).
pub struct AccountRules(pub Device);

impl Rules for AccountRules {
    fn state(&self) -> Result<(State, DateTime<Utc>), Notice> {
        self.0
            .state()
            .map(|s| (s, self.0.server_time()))
            .map_err(|_| Notice::what_is_so("This Device's copy of your account can't be read."))
    }

    fn send(&self, command: &Command) -> Result<Result<(), String>, Notice> {
        match self.0.send(command) {
            Ok(CommandOutcome::Accepted) => Ok(Ok(())),
            Ok(CommandOutcome::Rejected { reason }) => Ok(Err(reason)),
            Err(_) => Err(Notice::what_is_so(
                "Offline, still enforcing. Blocklist changes need a connection.",
            )),
        }
    }
}
