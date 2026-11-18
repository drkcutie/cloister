//! The service's clock until server time arrives through sync.
//!
//! Wall-clock time is trusted only as far as a monotonic clock that keeps counting through
//! sleep agrees with it: moving the system clock forward can't end a Block early while the
//! service runs. Moving it back only makes Blocks last longer.

use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::service::Clock;

/// How far the wall clock may run ahead of the monotonic clock (NTP corrections).
pub const SLACK: Duration = Duration::from_secs(5);

/// The trusted time: the wall clock, capped at what the monotonic clock allows.
pub fn trusted_now(
    wall_at_start: DateTime<Utc>,
    elapsed_since_start: Duration,
    wall_now: DateTime<Utc>,
) -> DateTime<Utc> {
    let ceiling = wall_at_start + elapsed_since_start + SLACK;
    wall_now.min(ceiling)
}

/// [`Clock`] backed by the system clock and a monotonic clock that includes sleep.
pub struct TrustedClock {
    wall_at_start: DateTime<Utc>,
    mono_at_start: Duration,
}

impl TrustedClock {
    pub fn start() -> Self {
        TrustedClock {
            wall_at_start: Utc::now(),
            mono_at_start: monotonic(),
        }
    }
}

impl Clock for TrustedClock {
    fn now(&self) -> DateTime<Utc> {
        let elapsed = monotonic().saturating_sub(self.mono_at_start);
        trusted_now(self.wall_at_start, elapsed, Utc::now())
    }
}

/// Time since boot, including sleep.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn follows_the_wall_clock_when_it_agrees() {
        let now = trusted_now(
            t("2026-01-05T09:00:00Z"),
            Duration::from_secs(600),
            t("2026-01-05T09:10:00Z"),
        );
        assert_eq!(now, t("2026-01-05T09:10:00Z"));
    }

    #[test]
    fn ignores_the_wall_clock_jumping_forward() {
        let now = trusted_now(
            t("2026-01-05T09:00:00Z"),
            Duration::from_secs(600),
            t("2026-01-05T23:00:00Z"),
        );
        assert_eq!(now, t("2026-01-05T09:10:05Z"));
    }

    #[test]
    fn follows_the_wall_clock_back() {
        let now = trusted_now(
            t("2026-01-05T09:00:00Z"),
            Duration::from_secs(600),
            t("2026-01-05T08:00:00Z"),
        );
        assert_eq!(now, t("2026-01-05T08:00:00Z"));
    }
}
