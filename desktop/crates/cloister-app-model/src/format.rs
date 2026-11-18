//! How times and lengths are written on screen. Figures are set in Source Code Pro, so
//! every digit has the same width and the countdown never shifts.

use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, TimeZone, Utc};

/// The timezone the app shows times in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Zone {
    #[default]
    Local,
    Utc,
}

impl Zone {
    /// The start of the ISO week (Monday 00:00) containing `t`, in this zone.
    pub fn week_start(&self, t: DateTime<Utc>) -> DateTime<Utc> {
        fn monday<Z: TimeZone>(z: &Z, t: DateTime<Utc>) -> DateTime<Utc> {
            let local = t.with_timezone(z);
            let days = local.weekday().num_days_from_monday() as i64;
            let date = local.date_naive() - Duration::days(days);
            z.from_local_datetime(&date.and_time(NaiveTime::MIN))
                .earliest()
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or(t)
        }
        match self {
            Zone::Local => monday(&Local, t),
            Zone::Utc => monday(&Utc, t),
        }
    }
}

/// "11:00": a time of day in `zone`, 24-hour.
pub fn clock(t: DateTime<Utc>, zone: &Zone) -> String {
    match zone {
        Zone::Local => t.with_timezone(&Local).format("%H:%M").to_string(),
        Zone::Utc => t.format("%H:%M").to_string(),
    }
}

/// "47:12" under an hour, "1:30:00" from an hour up. Rounded up to the second, so the
/// countdown reaches 0:00 exactly when the Block ends.
pub fn countdown(remaining: Duration) -> String {
    let millis = remaining.num_milliseconds().max(0);
    let secs = (millis + 999) / 1000;
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// "2h 15m", "45m": Deep Hours and Block lengths.
pub fn hours(d: Duration) -> String {
    let mins = d.num_minutes().max(0);
    match (mins / 60, mins % 60) {
        (0, m) => format!("{m}m"),
        (h, m) => format!("{h}h {m:02}m"),
    }
}
