//! The service's trusted clock: the last server time it was given, plus the time a monotonic
//! clock (that keeps counting through sleep) says has passed since. It is persisted, so it holds
//! across restarts and reboots.
//!
//! The wall clock is never trusted: setting it back can't lift Evening Lockdown and setting it
//! forward can't end a Block early, while the service runs or across a reboot. After a reboot
//! the monotonic clock restarts, so trusted time continues from the saved anchor plus the
//! uptime (at least that much time has passed). That can lag real time by however long the
//! Device was off; [`Clock::wall_hint`] lets the service also enforce what the wall clock says
//! when it is later (so a Device that was off still enters Evening Lockdown), never instead.

use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::service::Clock;

/// How often the anchor is written to disk at most (a reboot loses at most this much).
pub const CHECKPOINT_EVERY: Duration = Duration::from_secs(30);

/// A trusted time and the monotonic reading taken at that moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub trusted_at: DateTime<Utc>,
    /// Time since boot (including sleep) when `trusted_at` was taken.
    pub monotonic: Duration,
}

/// Trusted time at monotonic reading `monotonic_now`, from `anchor`. A reading below the
/// anchor's means the Device rebooted since: then at least the uptime has passed.
pub fn trusted_now(anchor: Anchor, monotonic_now: Duration) -> DateTime<Utc> {
    let elapsed = monotonic_now
        .checked_sub(anchor.monotonic)
        .unwrap_or(monotonic_now);
    anchor.trusted_at + chrono::Duration::from_std(elapsed).unwrap_or(chrono::Duration::zero())
}

/// Where the clock reads its monotonic time and wall time (the system, or a test's fake).
pub trait Source: Send {
    /// Time since boot, including sleep.
    fn monotonic(&self) -> Duration;
    fn wall(&self) -> DateTime<Utc>;
}

/// The operating system's clocks.
pub struct System;

impl Source for System {
    fn monotonic(&self) -> Duration {
        monotonic()
    }
    fn wall(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// [`Clock`] anchored on server time and a monotonic clock, saved at `path`.
pub struct TrustedClock<S = System> {
    source: S,
    path: Option<PathBuf>,
    anchor: Anchor,
    saved: Option<Anchor>,
}

impl TrustedClock<System> {
    /// The clock saved at `path`; a new one starts from the wall clock (the only time known
    /// before the first server time).
    pub fn open(path: impl Into<PathBuf>) -> Self {
        TrustedClock::with_source(System, Some(path.into()))
    }
}

impl<S: Source> TrustedClock<S> {
    pub fn with_source(source: S, path: Option<PathBuf>) -> Self {
        let saved: Option<Anchor> = path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        let anchor = saved.unwrap_or(Anchor {
            trusted_at: source.wall(),
            monotonic: source.monotonic(),
        });
        let mut clock = TrustedClock {
            source,
            path,
            anchor,
            saved,
        };
        // Re-anchor on this boot at once, so the uptime isn't counted twice after a reboot.
        clock.anchor = Anchor {
            trusted_at: clock.now(),
            monotonic: clock.source.monotonic(),
        };
        clock.save();
        clock
    }

    pub fn anchor(&self) -> Anchor {
        self.anchor
    }

    fn save(&mut self) {
        let Some(path) = &self.path else { return };
        if self.saved == Some(self.anchor) {
            return;
        }
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec(&self.anchor)?)?;
            std::fs::rename(&tmp, path)
        };
        match write() {
            Ok(()) => self.saved = Some(self.anchor),
            Err(e) => eprintln!("cloisterd: saving the clock {}: {e}", path.display()),
        }
    }
}

impl<S: Source> Clock for TrustedClock<S> {
    fn now(&self) -> DateTime<Utc> {
        trusted_now(self.anchor, self.source.monotonic())
    }

    fn wall_hint(&self) -> Option<DateTime<Utc>> {
        Some(self.source.wall())
    }

    fn set_server_time(&mut self, server_time: DateTime<Utc>) {
        self.anchor = Anchor {
            trusted_at: server_time,
            monotonic: self.source.monotonic(),
        };
        self.save();
    }

    fn checkpoint(&mut self) {
        let monotonic = self.source.monotonic();
        let due = self
            .saved
            .is_none_or(|s| monotonic < s.monotonic || monotonic - s.monotonic >= CHECKPOINT_EVERY);
        if due {
            self.anchor = Anchor {
                trusted_at: self.now(),
                monotonic,
            };
            self.save();
        }
    }
}

/// Time since boot, including sleep.
#[cfg(unix)]
fn monotonic() -> Duration {
    #[cfg(target_os = "macos")]
    const CLOCK: libc::clockid_t = libc::CLOCK_MONOTONIC; // counts through sleep on macOS
    #[cfg(not(target_os = "macos"))]
    const CLOCK: libc::clockid_t = libc::CLOCK_BOOTTIME;
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid, writable timespec.
    unsafe { libc::clock_gettime(CLOCK, &mut ts) };
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

/// Time since boot, including sleep (GetTickCount64 counts through sleep and hibernation).
#[cfg(windows)]
fn monotonic() -> Duration {
    // SAFETY: no preconditions.
    Duration::from_millis(unsafe {
        windows_sys::Win32::System::SystemInformation::GetTickCount64()
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    fn t(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    /// A fake machine: wall clock and uptime set by the test.
    #[derive(Clone)]
    struct Machine(Arc<Mutex<(DateTime<Utc>, Duration)>>);

    impl Machine {
        fn new(wall: &str, uptime_secs: u64) -> Self {
            Machine(Arc::new(Mutex::new((
                t(wall),
                Duration::from_secs(uptime_secs),
            ))))
        }
        /// Real time passes: both clocks move.
        fn pass(&self, secs: u64) {
            let mut m = self.0.lock().unwrap();
            m.0 += chrono::Duration::seconds(secs as i64);
            m.1 += Duration::from_secs(secs);
        }
        fn set_wall(&self, wall: &str) {
            self.0.lock().unwrap().0 = t(wall);
        }
        /// Off for `off_secs`, then booted `uptime_secs` ago.
        fn reboot(&self, off_secs: u64, uptime_secs: u64) {
            let mut m = self.0.lock().unwrap();
            m.0 += chrono::Duration::seconds((off_secs + uptime_secs) as i64);
            m.1 = Duration::from_secs(uptime_secs);
        }
    }

    impl Source for Machine {
        fn monotonic(&self) -> Duration {
            self.0.lock().unwrap().1
        }
        fn wall(&self) -> DateTime<Utc> {
            self.0.lock().unwrap().0
        }
    }

    fn path() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clock.json");
        (dir, path)
    }

    #[test]
    fn counts_from_the_server_time_on_the_monotonic_clock() {
        let machine = Machine::new("2026-01-05T09:00:00Z", 1000);
        let mut clock = TrustedClock::with_source(machine.clone(), None);
        clock.set_server_time(t("2026-01-05T09:00:30Z"));
        machine.pass(600);
        assert_eq!(clock.now(), t("2026-01-05T09:10:30Z"));
    }

    #[test]
    fn ignores_the_wall_clock_jumping_forward_or_back() {
        let machine = Machine::new("2026-01-05T17:30:00Z", 1000);
        let mut clock = TrustedClock::with_source(machine.clone(), None);
        clock.set_server_time(t("2026-01-05T17:30:00Z"));
        machine.set_wall("2026-01-05T08:00:00Z"); // back: Evening Lockdown must hold
        machine.pass(60);
        assert_eq!(clock.now(), t("2026-01-05T17:31:00Z"));
        machine.set_wall("2026-01-07T08:00:00Z"); // forward: Blocks must not end early
        machine.pass(60);
        assert_eq!(clock.now(), t("2026-01-05T17:32:00Z"));
        assert_eq!(clock.wall_hint(), Some(t("2026-01-07T08:01:00Z")));
    }

    #[test]
    fn a_reboot_with_the_clock_set_forward_doesnt_skip_ahead() {
        let (_dir, path) = path();
        let machine = Machine::new("2026-01-05T09:00:00Z", 5000);
        let mut clock = TrustedClock::with_source(machine.clone(), Some(path.clone()));
        clock.set_server_time(t("2026-01-05T09:00:00Z"));
        machine.pass(120);
        clock.checkpoint();
        // Off for an hour, the wall clock moved a day forward, booted 10 minutes ago.
        machine.reboot(3600, 600);
        machine.set_wall("2026-01-06T10:12:00Z");
        let clock = TrustedClock::with_source(machine.clone(), Some(path.clone()));
        // At least the uptime has passed since the last saved trusted time.
        assert_eq!(clock.now(), t("2026-01-05T09:12:00Z"));
        machine.pass(60);
        assert_eq!(clock.now(), t("2026-01-05T09:13:00Z"));
    }

    #[test]
    fn a_reboot_with_the_clock_set_back_doesnt_go_back() {
        let (_dir, path) = path();
        let machine = Machine::new("2026-01-05T18:00:00Z", 5000);
        let mut clock = TrustedClock::with_source(machine.clone(), Some(path.clone()));
        clock.set_server_time(t("2026-01-05T18:00:00Z"));
        machine.reboot(0, 60);
        machine.set_wall("2026-01-05T12:00:00Z");
        let clock = TrustedClock::with_source(machine.clone(), Some(path));
        assert_eq!(clock.now(), t("2026-01-05T18:01:00Z"));
    }

    #[test]
    fn a_restart_without_reboot_continues_where_it_was() {
        let (_dir, path) = path();
        let machine = Machine::new("2026-01-05T09:00:00Z", 5000);
        let mut clock = TrustedClock::with_source(machine.clone(), Some(path.clone()));
        clock.set_server_time(t("2026-01-05T09:00:00Z"));
        machine.pass(300);
        machine.set_wall("2026-01-09T09:00:00Z");
        drop(clock);
        let clock = TrustedClock::with_source(machine.clone(), Some(path));
        assert_eq!(clock.now(), t("2026-01-05T09:05:00Z"));
    }

    #[test]
    fn checkpoints_at_most_every_thirty_seconds() {
        let (_dir, path) = path();
        let machine = Machine::new("2026-01-05T09:00:00Z", 5000);
        let mut clock = TrustedClock::with_source(machine.clone(), Some(path.clone()));
        let first = clock.anchor();
        machine.pass(10);
        clock.checkpoint();
        assert_eq!(clock.anchor(), first);
        machine.pass(25);
        clock.checkpoint();
        assert_eq!(clock.anchor().trusted_at, t("2026-01-05T09:00:35Z"));
    }
}
